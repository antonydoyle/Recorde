use std::sync::{Arc, Mutex};

use crate::git::graph::CommitGraph;

#[derive(Debug, Clone)]
pub struct FileStatus {
    pub path: String,
    pub index_status: char,  // 'M', 'A', 'D', '?', etc.
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
    pub origin: char,              // '+', '-', ' ', '\\'
    pub content: String,
    pub old_lineno: Option<u32>,
    pub new_lineno: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct DiffHunk {
    pub header: String,            // "@@ -1,4 +1,8 @@ context"
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
    pub working_dir_status: Vec<FileStatus>,
}

pub type SharedState = Arc<Mutex<AppState>>;

pub fn new_shared_state() -> SharedState {
    Arc::new(Mutex::new(AppState::default()))
}
