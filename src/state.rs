use std::sync::{Arc, Mutex};

use crate::git::capabilities::RepoCapabilities;
use crate::git::graph::CommitGraph;
use crate::git::job::ActiveOp;

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

/// Action for a single commit in an interactive rebase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebaseAction {
    Pick,
    Squash,
    Fixup,
    Drop,
}

impl RebaseAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pick   => "pick",
            Self::Squash => "squash",
            Self::Fixup  => "fixup",
            Self::Drop   => "drop",
        }
    }
}

/// One commit entry in the interactive rebase todo list.
#[derive(Debug, Clone)]
pub struct RebaseEntry {
    pub oid: String,
    pub summary: String,
    pub action: RebaseAction,
}

/// A file with unresolved merge conflicts.
#[derive(Debug, Clone)]
pub struct ConflictedFile {
    pub path: String,
}

/// Which section of the working tree the current diff was loaded from.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum DiffContext {
    #[default]
    CommitDiff,
    StagedFile,
    UnstagedFile,
}

/// A destructive action pending user confirmation.
#[derive(Debug, Clone)]
pub enum PendingDiscard {
    File { path: String },
    Hunk { hunk_index: usize },
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
    CreateTag {
        from_oid: Option<String>,
    },
    ConfirmDeleteTag {
        tag_name: String,
    },
    MergeBranch,
    RebaseOnto,
    CloneRepo,
    InitRepo,
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

    pub git_user_initials: String,
    pub head_branch: Option<String>,
    pub ahead_behind: Option<(usize, usize)>,
    pub branch_ahead_behind: std::collections::HashMap<String, (usize, usize)>,
    pub dirty_count: usize,
    pub working_dir_files: Vec<WorkingDirFile>,
    pub stashes: Vec<StashEntry>,

    pub conflicted_files: Vec<ConflictedFile>,
    pub modal: ModalKind,
    /// Context for the currently-loaded diff (commit vs staged vs unstaged).
    pub diff_context: DiffContext,
    /// Destructive action waiting for user confirmation.
    pub pending_discard: Option<PendingDiscard>,
    /// Label shown while a branch/staging operation is running (legacy path).
    pub remote_op: Option<String>,
    /// In-flight jobs dispatched through GitRunner.
    pub active_ops: Vec<ActiveOp>,
    /// Capabilities detected when the repo was last opened.
    pub capabilities: Option<RepoCapabilities>,
    /// Log lines shown in the Console tab.
    pub console_log: Vec<String>,
    /// Commit graph search state.
    pub search_active: bool,
    pub search_query: String,
    pub show_all_branches: bool,
    /// 1=sidebar, 2=graph, 3=changes, 4=diff. 0=none.
    pub focused_pane: u8,
    /// Set by the graph context menu to trigger opening the rebase modal.
    pub pending_open_rebase: Option<String>,
}

pub type SharedState = Arc<Mutex<AppState>>;

pub fn new_shared_state() -> SharedState {
    // Read git user.name from global config for the avatar initials.
    let git_user_initials = git2::Config::open_default()
        .ok()
        .and_then(|c| c.get_string("user.name").ok())
        .map(|name| {
            let mut words = name.split_whitespace();
            let first = words.next().and_then(|w| w.chars().next());
            let last  = words.last().and_then(|w| w.chars().next());
            match (first, last) {
                (Some(a), Some(b)) => format!("{}{}", a.to_uppercase(), b.to_uppercase()),
                (Some(a), None)    => a.to_uppercase().to_string(),
                _                  => "?".into(),
            }
        })
        .unwrap_or_else(|| "?".into());

    Arc::new(Mutex::new(AppState { show_all_branches: true, git_user_initials, ..Default::default() }))
}
