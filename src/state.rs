use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::git::capabilities::RepoCapabilities;
use crate::git::graph::CommitGraph;
use crate::git::job::ActiveOp;
use crate::ui::error_notice::{classify, ErrorKind};

/// A redacted failure waiting for the user to dismiss it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserError {
    pub kind: ErrorKind,
    pub message: String,
}

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
    /// Raw UTF-8 line, including its original line ending when present.
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
    pub binary_preview: Option<crate::git::binary_preview::BinaryPreview>,
    pub raw_patch: Vec<u8>,
    pub old_path: Option<String>,
    pub is_rename: bool,
    pub old_mode: u32,
    pub new_mode: u32,
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
    Reword,
    Edit,
    Squash,
    Fixup,
    Drop,
}

impl RebaseAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pick => "pick",
            Self::Reword => "reword",
            Self::Edit => "edit / split",
            Self::Squash => "squash",
            Self::Fixup => "fixup",
            Self::Drop => "drop",
        }
    }
}

/// One commit entry in the interactive rebase todo list.
#[derive(Debug, Clone)]
pub struct RebaseEntry {
    pub message: String,
    pub oid: String,
    pub summary: String,
    pub action: RebaseAction,
}

/// A file with unresolved merge conflicts.
#[derive(Debug, Clone)]
pub struct ConflictedFile {
    pub path: String,
    pub version: Option<crate::git::conflicts::Version>,
    pub ours_deleted: bool,
    pub theirs_deleted: bool,
}

/// Which section of the working tree the current diff was loaded from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum DiffContext {
    #[default]
    CommitDiff,
    StagedFile,
    UnstagedFile,
}

/// One entry from `git reflog`.
#[derive(Debug, Clone)]
pub struct ReflogEntry {
    pub oid: String,
    pub short_oid: String,
    pub refname: String, // e.g. "HEAD@{0}"
    pub message: String, // e.g. "commit: fix typo"
    pub timestamp: i64,
}

/// Per-line annotation produced by `git blame`.
#[derive(Debug, Clone)]
pub struct BlameLine {
    pub lineno: usize,
    pub short_oid: String,
    pub author: String,
    pub timestamp: i64,
    pub content: String,
}

/// A recently-deleted branch that can be restored.
#[derive(Debug, Clone)]
pub struct DeletedBranch {
    pub name: String,
    pub oid: String,
}

/// A git submodule entry parsed from `.gitmodules`.
#[derive(Debug, Clone)]
pub struct SubmoduleInfo {
    pub name: String,
    pub path: String,
    pub url: String,
    pub initialized: bool,
}

/// A linked worktree entry from `git worktree list --porcelain`.
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    pub locked: bool,
    pub prunable: bool,
    pub path: String,
    pub head_oid: String,
    pub branch: Option<String>,
    pub is_main: bool,
}

/// A destructive action pending user confirmation.
#[derive(Debug, Clone)]
pub enum PendingDiscard {
    File {
        path: String,
    },
    Selection {
        repo: std::path::PathBuf,
        diff: Arc<FileDiff>,
        lines: Vec<(usize, usize)>,
    },
    ResetHard,
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
    ConfirmForceDeleteBranch {
        branch_name: String,
    },
    /// Remote-tracking shorthand such as `origin/feature`.
    ConfirmDeleteRemoteBranch {
        name: String,
    },
    ConfirmDeleteRemoteTag {
        tag_name: String,
    },
    ResetToCommit {
        oid: String,
    },
    /// Restore recovery checkpoint `id`, the next undo or redo step.
    ConfirmUndo {
        id: String,
        label: String,
        redo: bool,
    },
    ConfirmCommitSequence {
        preview: crate::git::branches::CommitSequencePreview,
    },
    CreateTag {
        from_oid: Option<String>,
    },
    ConfirmDeleteTag {
        tag_name: String,
    },
    /// Merge with options; `branch` preselects the branch to merge.
    MergeBranch {
        branch: Option<String>,
    },
    RebaseOnto,
    CloneRepo,
    InitRepo,
    RenameBranch {
        old_name: String,
    },
    ConfirmDetachedCheckout {
        oid: String,
    },
    StashWithMessage,
    CreateBranchFromStash {
        target: crate::git::branches::StashTarget,
    },
    ConfirmDropStash {
        target: crate::git::branches::StashTarget,
    },
    Reflog,
    Bisect,
    Compare {
        base_oid: String,
        compare_oid: String,
    },
    GitFlow,
}

#[derive(Default)]
pub struct AppState {
    pub history_limit: usize,
    pub repo_views: std::collections::HashMap<std::path::PathBuf, RepoView>,
    pub repo_path: Option<std::path::PathBuf>,
    /// Latest repository load request; older workers must not publish over it.
    pub repo_load_id: u64,
    pub repo_load_path: Option<std::path::PathBuf>,
    /// Changes when the active repository changes, including A -> B -> A.
    pub repo_epoch: u64,
    pub diff_load_id: u64,
    pub commit_graph: Option<std::sync::Arc<CommitGraph>>,
    pub error: Option<String>,
    pub errors: VecDeque<UserError>,

    pub selected_commit: Option<String>,
    /// Commits chosen with Ctrl/Cmd- or Shift-click. Empty unless two or more
    /// are selected; `selected_commit` remains the most recently clicked one.
    pub selected_commits: Vec<String>,
    pub selected_commit_detail: Option<CommitDetail>,
    pub selected_file: Option<String>,
    pub current_diff: Option<Arc<FileDiff>>,

    pub git_user_initials: String,
    pub head_branch: Option<String>,
    pub ahead_behind: Option<(usize, usize)>,
    /// The checked-out branch's upstream (`origin/main`), when it has one.
    pub head_upstream: Option<String>,
    pub branch_ahead_behind: std::collections::HashMap<String, (usize, usize)>,
    pub dirty_count: usize,
    pub working_dir_files: Vec<WorkingDirFile>,
    pub stashes: Vec<StashEntry>,

    pub conflicted_files: Vec<ConflictedFile>,
    pub pending_conflict_editor: Option<String>,
    /// Working-tree file to open in the built-in editor.
    pub pending_file_edit: Option<String>,
    pub pending_commit_settings: bool,
    pub merged_into_head: std::collections::HashSet<String>,
    pub modal: ModalKind,
    /// Context for the currently-loaded diff (commit vs staged vs unstaged).
    pub diff_context: DiffContext,
    /// Destructive action waiting for user confirmation.
    pub pending_discard: Option<PendingDiscard>,
    /// Label shown while a branch/staging operation is running (legacy path).
    pub remote_op: Option<String>,
    /// In-flight jobs dispatched through GitRunner.
    pub active_ops: Vec<ActiveOp>,
    /// Last failed background fetch, scoped to its repository.
    pub background_fetch: Option<crate::git::job::BackgroundFetchStatus>,
    /// Optional (message, percentage) for local jobs, keyed by their job ID.
    pub local_job_progress: std::collections::HashMap<u64, (String, Option<u8>)>,
    /// Capabilities detected when the repo was last opened.
    pub capabilities: Option<RepoCapabilities>,
    /// Log lines shown in the Console tab.
    pub console_log: Vec<String>,
    /// Commit graph search state.
    pub search_active: bool,
    pub search_query: String,
    pub show_all_branches: bool,
    pub graph_filter: crate::git::graph::GraphFilter,
    /// 1=sidebar, 2=graph, 3=changes, 4=diff. 0=none.
    pub focused_pane: u8,
    /// Set by the graph context menu to trigger opening the rebase modal.
    pub pending_open_rebase: Option<String>,
    /// Selected commits to combine with an interactive rebase.
    pub pending_squash: Option<Vec<String>>,
    pub pending_drop: Option<Vec<String>>,
    /// Next undo and redo steps as (checkpoint ID, label), read on refresh.
    pub undo_step: Option<(String, String)>,
    pub redo_step: Option<(String, String)>,
    /// Git Flow branch configuration, read on refresh.
    pub git_flow: Option<crate::git::gitflow::FlowConfig>,
    /// Deleted branches backed by durable protected refs, restorable after restart.
    pub deleted_branches: Vec<DeletedBranch>,

    // ── Phase 9: Pull Requests / Merge Requests ───────────────────────────────
    pub pr_loaded: bool,
    pub pr_list: Vec<crate::providers::PrItem>,
    pub pr_source: Option<(String, crate::git::capabilities::ProviderType, String)>,
    pub provider_requests: [u64; 5],
    pub local_jobs: Vec<crate::git::mutation::LocalJob>,
    pub pr_detail_loading: bool,
    pub pr_loading: bool,
    pub pr_error: Option<String>,
    pub pr_selected: Option<usize>,
    pub pr_filter: crate::providers::PrFilter,

    // ── Phase 13: Provider Integrations ──────────────────────────────────────
    pub issue_list: Vec<crate::providers::IssueItem>,
    pub issue_loading: bool,
    pub issue_error: Option<String>,
    pub issue_selected: Option<usize>,
    pub issue_filter: crate::providers::IssueFilter,

    pub ci_runs: Vec<crate::providers::CiRun>,
    pub ci_loading: bool,
    pub ci_error: Option<String>,

    pub releases: Vec<crate::providers::ReleaseItem>,
    pub releases_loading: bool,
    pub releases_error: Option<String>,

    // ── Phase 7: History, Search, Review ─────────────────────────────────────
    /// When set, the commit graph dims commits NOT in this set (file history mode).
    pub file_history_path: Option<String>,
    pub file_history_oids: std::collections::HashSet<String>,
    /// First commit selected for a two-commit comparison.
    pub compare_base_oid: Option<String>,
    /// Files changed between compare_base and compare_oid (loaded on demand).
    pub compare_files: Vec<ChangedFile>,
    /// Current file diff shown inside the Compare modal.
    pub compare_diff: Option<FileDiff>,
    pub compare_selected_file: Option<String>,
    /// Reflog entries loaded for the current repo.
    pub reflog_entries: Vec<ReflogEntry>,
    /// Blame annotations for the currently-selected file.
    pub blame_lines: Vec<BlameLine>,
    pub show_blame: bool,
    /// Bisect state (set when `git bisect start` is active).
    pub bisect_in_progress: bool,
    pub bisect_current_oid: Option<String>,
    /// Set once an active bisect session has identified the first bad commit.
    pub bisect_first_bad: Option<String>,

    // ── Phase 12: Submodules and Worktrees ───────────────────────────────────
    pub submodules: Vec<SubmoduleInfo>,
    pub worktrees: Vec<WorktreeInfo>,
}

pub type SharedState = Arc<Mutex<AppState>>;

impl AppState {
    /// Collect the producer slot without replacing failures already being shown.
    pub fn take_pending_error(&mut self) {
        let Some(message) = self.error.take() else {
            return;
        };
        let message = crate::auth::redact::text(&message);
        let error = UserError {
            kind: classify(&message),
            message,
        };
        if self.errors.back() != Some(&error) {
            self.errors.push_back(error);
        }
        while self.errors.len() > 5 {
            self.errors.pop_front();
        }
    }

    /// The multi-selection ordered oldest first (bottom of the graph first).
    pub fn selected_commits_oldest_first(&self) -> Vec<String> {
        let mut oids = self.selected_commits.clone();
        if let Some(graph) = &self.commit_graph {
            oids.sort_by_key(|oid| {
                std::cmp::Reverse(graph.oid_to_row.get(oid).copied().unwrap_or(0))
            });
        }
        oids
    }

    pub fn invalidate_provider_data(&mut self) {
        for view in self.repo_views.values_mut() {
            view.pr_loaded = false;
            view.pr_list.clear();
            view.pr_source = None;
            view.pr_selected = None;
            view.issue_list.clear();
            view.issue_selected = None;
            view.ci_runs.clear();
            view.releases.clear();
        }
        for request in &mut self.provider_requests {
            *request = request.wrapping_add(1);
        }
        self.pr_loaded = false;
        self.pr_list.clear();
        self.pr_source = None;
        self.pr_selected = None;
        self.pr_loading = false;
        self.pr_detail_loading = false;
        self.pr_error = None;
        self.issue_list.clear();
        self.issue_selected = None;
        self.issue_loading = false;
        self.issue_error = None;
        self.ci_runs.clear();
        self.ci_loading = false;
        self.ci_error = None;
        self.releases.clear();
        self.releases_loading = false;
        self.releases_error = None;
    }
}

#[derive(Clone, Default)]
pub struct RepoView {
    pub modal: ModalKind,
    pub selected_commit_detail: Option<CommitDetail>,
    pub selected_file: Option<String>,
    pub current_diff: Option<Arc<FileDiff>>,
    pub diff_context: DiffContext,
    pub focused_pane: u8,
    pub show_blame: bool,
    pub blame_lines: Vec<BlameLine>,
    pub file_history_path: Option<String>,
    pub file_history_oids: std::collections::HashSet<String>,
    pub compare_base_oid: Option<String>,
    pub pr_loaded: bool,
    pub pr_list: Vec<crate::providers::PrItem>,
    pub pr_source: Option<(String, crate::git::capabilities::ProviderType, String)>,
    pub pr_selected: Option<usize>,
    pub pr_filter: crate::providers::PrFilter,
    pub issue_list: Vec<crate::providers::IssueItem>,
    pub issue_selected: Option<usize>,
    pub issue_filter: crate::providers::IssueFilter,
    pub ci_runs: Vec<crate::providers::CiRun>,
    pub releases: Vec<crate::providers::ReleaseItem>,
    pub history_limit: usize,
    pub selected_commit: Option<String>,
    pub search_active: bool,
    pub search_query: String,
    pub show_all_branches: bool,
    pub graph_filter: crate::git::graph::GraphFilter,
    pub deleted_branches: Vec<DeletedBranch>,
}
impl RepoView {
    pub fn capture(state: &AppState) -> Self {
        Self {
            modal: state.modal.clone(),
            history_limit: state.history_limit,
            selected_commit: state.selected_commit.clone(),
            search_active: state.search_active,
            search_query: state.search_query.clone(),
            show_all_branches: state.show_all_branches,
            graph_filter: state.graph_filter.clone(),
            deleted_branches: state.deleted_branches.clone(),
            selected_commit_detail: state.selected_commit_detail.clone(),
            selected_file: state.selected_file.clone(),
            current_diff: state.current_diff.clone(),
            diff_context: state.diff_context,
            focused_pane: state.focused_pane,
            show_blame: state.show_blame,
            blame_lines: state.blame_lines.clone(),
            file_history_path: state.file_history_path.clone(),
            file_history_oids: state.file_history_oids.clone(),
            compare_base_oid: state.compare_base_oid.clone(),
            pr_loaded: state.pr_loaded,
            pr_list: state.pr_list.clone(),
            pr_source: state.pr_source.clone(),
            pr_selected: state.pr_selected,
            pr_filter: state.pr_filter,
            issue_list: state.issue_list.clone(),
            issue_selected: state.issue_selected,
            issue_filter: state.issue_filter,
            ci_runs: state.ci_runs.clone(),
            releases: state.releases.clone(),
        }
    }
    pub fn restore(self, state: &mut AppState) {
        state.modal = self.modal;
        state.history_limit = self.history_limit;
        state.selected_commit = self.selected_commit;
        state.selected_commit_detail = self.selected_commit_detail;
        state.selected_file = self.selected_file;
        state.current_diff = self.current_diff;
        state.diff_context = self.diff_context;
        state.focused_pane = self.focused_pane;
        state.show_blame = self.show_blame;
        state.blame_lines = self.blame_lines;
        state.file_history_path = self.file_history_path;
        state.file_history_oids = self.file_history_oids;
        state.compare_base_oid = self.compare_base_oid;
        state.pr_loaded = self.pr_loaded;
        state.pr_list = self.pr_list;
        state.pr_source = self.pr_source;
        state.pr_selected = self.pr_selected;
        state.pr_filter = self.pr_filter;
        state.issue_list = self.issue_list;
        state.issue_selected = self.issue_selected;
        state.issue_filter = self.issue_filter;
        state.ci_runs = self.ci_runs;
        state.releases = self.releases;

        state.search_active = self.search_active;
        state.search_query = self.search_query;
        state.graph_filter = self.graph_filter;
        state.show_all_branches = self.show_all_branches;
        state.deleted_branches = self.deleted_branches;
    }
}

pub fn new_shared_state() -> SharedState {
    // Read git user.name from global config for the avatar initials.
    let git_user_initials = git2::Config::open_default()
        .ok()
        .and_then(|c| c.get_string("user.name").ok())
        .map(|name| {
            let mut words = name.split_whitespace();
            let first = words.next().and_then(|w| w.chars().next());
            let last = words.last().and_then(|w| w.chars().next());
            match (first, last) {
                (Some(a), Some(b)) => format!("{}{}", a.to_uppercase(), b.to_uppercase()),
                (Some(a), None) => a.to_uppercase().to_string(),
                _ => "?".into(),
            }
        })
        .unwrap_or_else(|| "?".into());

    Arc::new(Mutex::new(AppState {
        show_all_branches: true,
        git_user_initials,
        ..Default::default()
    }))
}

#[cfg(test)]
mod error_queue_tests {
    use super::*;

    #[test]
    fn pending_errors_collapse_only_consecutive_duplicates() {
        let mut state = AppState::default();
        for message in [
            "first failure",
            "first failure",
            "second failure",
            "first failure",
        ] {
            state.error = Some(message.into());
            state.take_pending_error();
            assert!(state.error.is_none());
        }
        let messages: Vec<_> = state
            .errors
            .iter()
            .map(|error| error.message.as_str())
            .collect();
        assert_eq!(
            messages,
            ["first failure", "second failure", "first failure"]
        );
        state.take_pending_error();
        assert_eq!(state.errors.len(), 3);
    }

    #[test]
    fn pending_errors_keep_the_five_newest_in_order() {
        let mut state = AppState::default();
        for index in 0..7 {
            state.error = Some(format!("failure {index}"));
            state.take_pending_error();
        }
        let messages: Vec<_> = state
            .errors
            .iter()
            .map(|error| error.message.as_str())
            .collect();
        assert_eq!(
            messages,
            [
                "failure 2",
                "failure 3",
                "failure 4",
                "failure 5",
                "failure 6"
            ]
        );
        state.error = Some("failure 6".into());
        state.take_pending_error();
        assert_eq!(state.errors.len(), 5);
        assert_eq!(state.errors.front().unwrap().message, "failure 2");
    }

    #[test]
    fn pending_errors_are_redacted_before_classification_and_deduplication() {
        let mut state = AppState::default();
        for userinfo in ["user:secret", "other:password"] {
            state.error = Some(format!(
                "fatal: Authentication failed for 'https://{userinfo}@host/o/r.git/'"
            ));
            state.take_pending_error();
        }
        assert!(state.error.is_none());
        assert_eq!(state.errors.len(), 1);
        let error = state.errors.front().unwrap();
        assert_eq!(error.kind, ErrorKind::SignIn);
        assert!(error.message.contains("https://[redacted]@host/o/r.git/"));
        for secret in ["user", "secret", "other", "password"] {
            assert!(!error.message.contains(secret));
        }
    }
}
