use std::sync::{Arc, Mutex};

use crate::git::graph::CommitGraph;

/// A file in the working directory, with separate staged and unstaged status.
/// A file may appear in both sections simultaneously (partially staged).
#[derive(Debug, Clone)]
pub struct WorkingDirFile {
    pub path: String,
    /// Status in the index (staged): 'A', 'M', 'D', 'R', or None.
    pub staged_status: Option<char>,
    /// Status in the working tree (unstaged): 'M', 'D', '?', or None.
    pub unstaged_status: Option<char>,
}

#[derive(Debug, Clone)]
pub struct ChangedFile {
    pub path: String,
    pub status: char, // 'M', 'A', 'D', 'R', 'C'
}

#[derive(Debug, Clone)]
pub struct CommitDetail {
    pub oid: String,
    pub author: String,
    pub author_email: String,
    pub timestamp: i64,
    pub message: String,
    pub changed_files: Vec<ChangedFile>,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub origin: char,
    pub content: String,
    pub old_lineno: Option<u32>,
    pub new_lineno: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct DiffHunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone)]
pub struct FileDiff {
    pub commit_oid: String,
    pub file_path: String,
    pub hunks: Vec<DiffHunk>,
    pub is_binary: bool,
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Debug, Clone)]
pub struct StashEntry {
    pub index: usize,
    pub message: String,
    pub oid: String,
}

/// Which modal dialog is currently open (if any).
#[derive(Debug, Clone, Default)]
pub enum ModalKind {
    #[default]
    None,
    CreateBranch {
        /// Commit OID to branch from; None = HEAD.
        from_oid: Option<String>,
    },
    ConfirmDeleteBranch {
        branch_name: String,
    },
}

#[derive(Default)]
pub struct AppState {
    pub repo_path: Option<std::path::PathBuf>,
    pub commit_graph: Option<CommitGraph>,
    pub error: Option<String>,

    pub selected_commit: Option<String>,
    pub selected_commit_detail: Option<CommitDetail>,
    pub selected_file: Option<String>,
    pub current_diff: Option<FileDiff>,

    pub head_branch: Option<String>,
    pub ahead_behind: Option<(usize, usize)>,
    pub dirty_count: usize,
    pub working_dir_files: Vec<WorkingDirFile>,
    pub stashes: Vec<StashEntry>,

    pub modal: ModalKind,
}

pub type SharedState = Arc<Mutex<AppState>>;

pub fn new_shared_state() -> SharedState {
    Arc::new(Mutex::new(AppState::default()))
}
