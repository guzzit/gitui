//! asyncgit

#![forbid(missing_docs)]
// #![deny(unsafe_code)]
// #![deny(unused_imports)]
// #![deny(unused_must_use)]
// #![deny(dead_code)]
// #![deny(unstable_name_collisions)]
// #![deny(clippy::all, clippy::perf, clippy::nursery, clippy::pedantic)]
// #![deny(clippy::filetype_is_file)]
// #![deny(clippy::cargo)]
// #![deny(clippy::unwrap_used)]
// #![deny(clippy::panic)]
// #![deny(clippy::match_like_matches_macro)]
// #![deny(clippy::needless_update)]
// #![allow(clippy::module_name_repetitions)]
// #![allow(clippy::must_use_candidate)]
// #![allow(clippy::missing_errors_doc)]
//TODO: get this in someday since expect still leads us to crashes sometimes
// #![deny(clippy::expect_used)]

mod blame;
pub mod cached;
mod commit_files;
mod diff;
mod error;
mod fetch;
mod progress;
mod push;
mod push_tags;
pub mod remote_progress;
mod revlog;
mod status;
pub mod sync;
mod tags;

pub use crate::{
    blame::{AsyncBlame, BlameParams},
    commit_files::AsyncCommitFiles,
    diff::{AsyncDiff, DiffParams, DiffType},
    fetch::{AsyncFetch, FetchRequest},
    push::{AsyncPush, PushRequest},
    push_tags::{AsyncPushTags, PushTagsRequest},
    remote_progress::{RemoteProgress, RemoteProgressState},
    revlog::{AsyncLog, FetchStatus},
    status::{AsyncStatus, StatusParams},
    sync::{
        diff::{DiffLine, DiffLineType, FileDiff},
        status::{StatusItem, StatusItemType},
    },
    tags::AsyncTags,
};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

/// this type is used to communicate events back through the channel
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum AsyncNotification {
    /// this indicates that no new state was fetched but that a async process finished
    FinishUnchanged,
    ///
    Status,
    ///
    Diff,
    ///
    Log,
    ///
    CommitFiles,
    ///
    Tags,
    ///
    Push,
    ///
    PushTags,
    ///
    Fetch,
    ///
    Blame,
    ///
    //TODO: this does not belong here
    SyntaxHighlighting,
}

/// current working directory `./`
pub static CWD: &str = "./";

/// helper function to calculate the hash of an arbitrary type that implements the `Hash` trait
pub fn hash<T: Hash + ?Sized>(v: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    v.hash(&mut hasher);
    hasher.finish()
}
