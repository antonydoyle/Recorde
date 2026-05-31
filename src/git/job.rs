use std::sync::{Arc, atomic::AtomicBool};

use crate::git::error::GitError;

pub type JobId = u64;

/// Handle returned to the caller when a job is submitted.
pub struct JobHandle {
    pub id: JobId,
    /// Set to `true` to request cancellation (best-effort).
    pub cancel: Arc<AtomicBool>,
}

/// The type of git operation to run.
#[derive(Debug, Clone)]
pub enum JobKind {
    FetchAll,
    PullBranch,
    PullWithRebase,
    PullFfOnly,
    PushBranch,
    PushWithLease,
    PushSetUpstream,
}

impl JobKind {
    pub fn label(&self) -> &'static str {
        match self {
            JobKind::FetchAll       => "Fetching…",
            JobKind::PullBranch     => "Pulling…",
            JobKind::PullWithRebase => "Pulling (rebase)…",
            JobKind::PullFfOnly     => "Pulling (ff-only)…",
            JobKind::PushBranch     => "Pushing…",
            JobKind::PushWithLease  => "Pushing (with lease)…",
            JobKind::PushSetUpstream => "Pushing (set upstream)…",
        }
    }
}

/// Progress events sent from the worker thread back to the UI thread.
pub enum JobProgress {
    Started { id: JobId, label: String, cancel: Arc<AtomicBool> },
    Update  { id: JobId, message: String, pct: Option<u8> },
    Done    { id: JobId },
    Failed  { id: JobId, error: GitError },
}

/// Live operation tracked in AppState, one entry per in-flight job.
pub struct ActiveOp {
    pub id: JobId,
    pub label: String,
    pub progress_pct: Option<u8>,
    pub progress_msg: String,
    pub cancel: Arc<AtomicBool>,
}

/// Internal wire type sent through the job channel.
pub(crate) struct RawJob {
    pub kind: JobKind,
    pub id: JobId,
    pub cancel: Arc<AtomicBool>,
    pub repo_path: std::path::PathBuf,
    pub progress_tx: std::sync::mpsc::Sender<JobProgress>,
}
