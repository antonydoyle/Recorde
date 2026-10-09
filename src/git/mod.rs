pub mod bisect;
pub mod blame;
pub mod branches;
pub mod capabilities;
pub mod commit_settings;
pub mod conflict_edit;
pub mod conflicts;
pub mod error;
pub mod graph;
pub mod history;
pub mod job;
pub mod lfs;
pub mod lifecycle;
pub mod merge;
pub(crate) mod mutation;
pub mod operations;
pub mod patch;
pub(crate) mod process;
pub mod rebase;
pub mod recovery;
pub mod reflog;
pub mod remote;
pub mod repo;
pub mod reviews;
pub mod runner;
pub mod staging;
pub mod submodules;
#[cfg(test)]
pub(crate) mod test_support;
pub(crate) mod watch;
pub mod worktrees;

pub mod binary_preview;

mod history_cursor;
pub(crate) mod safe_path;
