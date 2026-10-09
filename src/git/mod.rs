pub mod bisect;
pub mod blame;
pub mod branches;
pub mod capabilities;
pub mod conflicts;
pub mod conflict_edit;
pub mod error;
pub mod graph;
pub mod history;
pub mod job;
pub mod lifecycle;
pub mod lfs;
pub mod merge;
pub(crate) mod mutation;
pub mod operations;
pub mod patch;
pub(crate) mod process;
pub mod rebase;
pub mod recovery;
pub mod reflog;
pub mod remote;
pub mod reviews;
pub mod repo;
pub mod runner;
pub mod staging;
pub mod submodules;
pub mod worktrees;
pub(crate) mod watch;
#[cfg(test)]
pub(crate) mod test_support;

pub mod binary_preview;

pub(crate) mod safe_path;
mod history_cursor;
