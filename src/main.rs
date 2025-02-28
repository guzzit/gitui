#![forbid(unsafe_code)]
#![deny(
	unused_imports,
	unused_must_use,
	dead_code,
	unstable_name_collisions,
	unused_assignments
)]
#![deny(clippy::all, clippy::perf, clippy::nursery, clippy::pedantic)]
#![deny(clippy::filetype_is_file)]
#![deny(clippy::cargo)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::panic)]
#![deny(clippy::match_like_matches_macro)]
#![deny(clippy::needless_update)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::multiple_crate_versions)]
//TODO:
// #![deny(clippy::expect_used)]

mod app;
mod args;
mod bug_report;
mod clipboard;
mod cmdbar;
mod components;
mod input;
mod keys;
mod notify_mutex;
mod popup_stack;
mod profiler;
mod queue;
mod spinner;
mod string_utils;
mod strings;
mod tabs;
mod ui;
mod version;
mod watcher;

use crate::{app::App, args::process_cmdline};
use anyhow::{bail, Result};
use app::QuitState;
use asyncgit::{
	sync::{utils::repo_work_dir, RepoPath},
	AsyncGitNotification,
};
use backtrace::Backtrace;
use crossbeam_channel::{tick, unbounded, Receiver, Select};
use crossterm::{
	terminal::{
		disable_raw_mode, enable_raw_mode, EnterAlternateScreen,
		LeaveAlternateScreen,
	},
	ExecutableCommand,
};
use input::{Input, InputEvent, InputState};
use keys::KeyConfig;
use profiler::Profiler;
use scopeguard::defer;
use scopetime::scope_time;
use spinner::Spinner;
use std::{
	cell::RefCell,
	io::{self, Write},
	panic, process,
	time::{Duration, Instant},
};
use tui::{
	backend::{Backend, CrosstermBackend},
	Terminal,
};
use ui::style::Theme;
use watcher::RepoWatcher;

static SPINNER_INTERVAL: Duration = Duration::from_millis(80);

///
#[derive(Clone)]
pub enum QueueEvent {
	Notify,
	SpinnerUpdate,
	AsyncEvent(AsyncNotification),
	InputEvent(InputEvent),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxHighlightProgress {
	Progress,
	Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsyncAppNotification {
	///
	SyntaxHighlighting(SyntaxHighlightProgress),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsyncNotification {
	///
	App(AsyncAppNotification),
	///
	Git(AsyncGitNotification),
}

fn main() -> Result<()> {
	let cliargs = process_cmdline()?;

	let _profiler = Profiler::new();

	asyncgit::register_tracing_logging();

	if !valid_path(&cliargs.repo_path) {
		eprintln!("invalid path\nplease run gitui inside of a non-bare git repository");
		return Ok(());
	}

	let key_config = KeyConfig::init()
		.map_err(|e| eprintln!("KeyConfig loading error: {}", e))
		.unwrap_or_default();
	let theme = Theme::init(&cliargs.theme)
		.map_err(|e| eprintln!("Theme loading error: {}", e))
		.unwrap_or_default();

	setup_terminal()?;
	defer! {
		shutdown_terminal();
	}

	set_panic_handlers()?;

	let mut terminal = start_terminal(io::stdout())?;
	let mut repo_path = cliargs.repo_path;
	let input = Input::new();

	loop {
		let quit_state = run_app(
			repo_path.clone(),
			theme,
			key_config.clone(),
			&input,
			&mut terminal,
		)?;

		match quit_state {
			QuitState::OpenSubmodule(p) => {
				repo_path = p;
			}
			_ => break,
		}
	}

	Ok(())
}

fn run_app(
	repo: RepoPath,
	theme: Theme,
	key_config: KeyConfig,
	input: &Input,
	terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<QuitState, anyhow::Error> {
	let (tx_git, rx_git) = unbounded();
	let (tx_app, rx_app) = unbounded();

	let rx_input = input.receiver();
	let watcher = RepoWatcher::new(repo_work_dir(&repo)?.as_str())?;
	let rx_watcher = watcher.receiver();
	let spinner_ticker = tick(SPINNER_INTERVAL);

	let mut app = App::new(
		RefCell::new(repo),
		&tx_git,
		&tx_app,
		input.clone(),
		theme,
		key_config,
	);

	let mut spinner = Spinner::default();
	let mut first_update = true;

	loop {
		let event = if first_update {
			first_update = false;
			QueueEvent::Notify
		} else {
			select_event(
				&rx_input,
				&rx_git,
				&rx_app,
				&rx_watcher,
				&spinner_ticker,
			)?
		};

		{
			if let QueueEvent::SpinnerUpdate = event {
				spinner.update();
				spinner.draw(terminal)?;
				continue;
			}

			scope_time!("loop");

			match event {
				QueueEvent::InputEvent(ev) => {
					if let InputEvent::State(InputState::Polling) = ev
					{
						//Note: external ed closed, we need to re-hide cursor
						terminal.hide_cursor()?;
					}
					app.event(ev)?;
				}
				QueueEvent::Notify => app.update()?,
				QueueEvent::AsyncEvent(ev) => {
					if !matches!(
						ev,
						AsyncNotification::Git(
							AsyncGitNotification::FinishUnchanged
						)
					) {
						app.update_async(ev)?;
					}
				}
				QueueEvent::SpinnerUpdate => unreachable!(),
			}

			draw(terminal, &app)?;

			spinner.set_state(app.any_work_pending());
			spinner.draw(terminal)?;

			if app.is_quit() {
				break;
			}
		}
	}

	Ok(app.quit_state())
}

fn setup_terminal() -> Result<()> {
	enable_raw_mode()?;
	io::stdout().execute(EnterAlternateScreen)?;
	Ok(())
}

fn shutdown_terminal() {
	let leave_screen =
		io::stdout().execute(LeaveAlternateScreen).map(|_f| ());

	if let Err(e) = leave_screen {
		eprintln!("leave_screen failed:\n{}", e);
	}

	let leave_raw_mode = disable_raw_mode();

	if let Err(e) = leave_raw_mode {
		eprintln!("leave_raw_mode failed:\n{}", e);
	}
}

fn draw<B: Backend>(
	terminal: &mut Terminal<B>,
	app: &App,
) -> io::Result<()> {
	if app.requires_redraw() {
		terminal.resize(terminal.size()?)?;
	}

	terminal.draw(|f| {
		if let Err(e) = app.draw(f) {
			log::error!("failed to draw: {:?}", e);
		}
	})?;

	Ok(())
}

fn valid_path(repo_path: &RepoPath) -> bool {
	asyncgit::sync::is_repo(repo_path)
}

fn select_event(
	rx_input: &Receiver<InputEvent>,
	rx_git: &Receiver<AsyncGitNotification>,
	rx_app: &Receiver<AsyncAppNotification>,
	rx_notify: &Receiver<()>,
	rx_spinner: &Receiver<Instant>,
) -> Result<QueueEvent> {
	let mut sel = Select::new();

	sel.recv(rx_input);
	sel.recv(rx_git);
	sel.recv(rx_app);
	sel.recv(rx_notify);
	sel.recv(rx_spinner);

	let oper = sel.select();
	let index = oper.index();

	let ev = match index {
		0 => oper.recv(rx_input).map(QueueEvent::InputEvent),
		1 => oper.recv(rx_git).map(|e| {
			QueueEvent::AsyncEvent(AsyncNotification::Git(e))
		}),
		2 => oper.recv(rx_app).map(|e| {
			QueueEvent::AsyncEvent(AsyncNotification::App(e))
		}),
		3 => oper.recv(rx_notify).map(|_| QueueEvent::Notify),
		4 => oper.recv(rx_spinner).map(|_| QueueEvent::SpinnerUpdate),
		_ => bail!("unknown select source"),
	}?;

	Ok(ev)
}

fn start_terminal<W: Write>(
	buf: W,
) -> io::Result<Terminal<CrosstermBackend<W>>> {
	let backend = CrosstermBackend::new(buf);
	let mut terminal = Terminal::new(backend)?;
	terminal.hide_cursor()?;
	terminal.clear()?;

	Ok(terminal)
}

fn set_panic_handlers() -> Result<()> {
	// regular panic handler
	panic::set_hook(Box::new(|e| {
		let backtrace = Backtrace::new();
		//TODO: create macro to do both in one
		log::error!("panic: {:?}\ntrace:\n{:?}", e, backtrace);
		eprintln!("panic: {:?}\ntrace:\n{:?}", e, backtrace);
		shutdown_terminal();
	}));

	// global threadpool
	rayon_core::ThreadPoolBuilder::new()
		.panic_handler(|e| {
			let backtrace = Backtrace::new();
			//TODO: create macro to do both in one
			log::error!("panic: {:?}\ntrace:\n{:?}", e, backtrace);
			eprintln!("panic: {:?}\ntrace:\n{:?}", e, backtrace);
			shutdown_terminal();
			process::abort();
		})
		.num_threads(4)
		.build_global()?;

	Ok(())
}
