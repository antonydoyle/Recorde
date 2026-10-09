use std::sync::{atomic::AtomicBool, Arc};

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
    PushWithLease(crate::git::remote::PushLease),
    PushSetUpstream,
}

impl JobKind {
    pub fn label(&self) -> &'static str {
        match self {
            JobKind::FetchAll => "Fetching…",
            JobKind::PullBranch => "Pulling…",
            JobKind::PullWithRebase => "Pulling (rebase)…",
            JobKind::PullFfOnly => "Pulling (ff-only)…",
            JobKind::PushBranch => "Pushing…",
            JobKind::PushWithLease(_) => "Pushing (with lease)…",
            JobKind::PushSetUpstream => "Pushing (set upstream)…",
        }
    }
}

/// Progress events sent from the worker thread back to the UI thread.
pub enum JobProgress {
    Started {
        id: JobId,
        repo_path: std::path::PathBuf,
        label: String,
        cancel: Arc<AtomicBool>,
    },
    Update {
        id: JobId,
        message: String,
        pct: Option<u8>,
    },
    Done {
        id: JobId,
    },
    Failed {
        id: JobId,
        error: GitError,
    },
}

/// Live operation tracked in AppState, one entry per in-flight job.
pub struct ActiveOp {
    pub id: JobId,
    pub repo_path: std::path::PathBuf,
    pub label: String,
    pub progress_pct: Option<u8>,
    pub progress_msg: String,
    pub cancel: Arc<AtomicBool>,
}

/// Apply every drained event before requesting a refresh. Returning early on
/// completion would silently discard the remaining events in the batch.
pub fn apply_progress(state: &mut crate::state::AppState, events: Vec<JobProgress>) -> bool {
    let mut refresh = false;
    for event in events {
        match event {
            JobProgress::Started {
                id,
                repo_path,
                label,
                cancel,
            } => {
                state
                    .console_log
                    .push(format!("$ {} (job #{})", label.trim_end_matches('…'), id));
                state.active_ops.push(ActiveOp {
                    id,
                    repo_path,
                    label,
                    progress_pct: None,
                    progress_msg: String::new(),
                    cancel,
                });
            }
            JobProgress::Update { id, message, pct } => {
                if let Some(op) = state.active_ops.iter_mut().find(|o| o.id == id) {
                    op.progress_msg = message;
                    op.progress_pct = pct;
                }
            }
            JobProgress::Done { id } => {
                let current = operation_is_current(state, id);
                let label = operation_label(state, id);
                state.active_ops.retain(|o| o.id != id);
                state.console_log.push(format!("{} done", label));
                refresh |= current;
            }
            JobProgress::Failed { id, error } => {
                let current = operation_is_current(state, id);
                let label = operation_label(state, id);
                state.active_ops.retain(|o| o.id != id);
                let msg = error.user_message();
                state
                    .console_log
                    .push(format!("error: {} — {}", label, msg));
                if current {
                    state.error = Some(msg);
                }
                // A failed command may have changed refs/index or paused on a conflict.
                refresh |= current;
            }
        }
    }
    refresh
}

fn operation_label(state: &crate::state::AppState, id: JobId) -> String {
    state
        .active_ops
        .iter()
        .find(|o| o.id == id)
        .map(|o| {
            format!(
                "{} ({})",
                o.label.trim_end_matches('…'),
                o.repo_path.display()
            )
        })
        .unwrap_or_else(|| format!("job #{}", id))
}

fn operation_is_current(state: &crate::state::AppState, id: JobId) -> bool {
    state
        .active_ops
        .iter()
        .find(|o| o.id == id)
        .is_some_and(|op| state.repo_path.as_ref() == Some(&op.repo_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started(id: JobId) -> JobProgress {
        JobProgress::Started {
            repo_path: "repo-a".into(),
            id,
            label: format!("operation {}", id),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn completion_does_not_drop_later_failure_or_progress() {
        let mut state = crate::state::AppState {
            repo_path: Some("repo-a".into()),
            ..Default::default()
        };
        assert!(apply_progress(
            &mut state,
            vec![
                started(1),
                started(2),
                JobProgress::Done { id: 1 },
                JobProgress::Failed {
                    id: 2,
                    error: GitError::Cancelled
                },
                started(3),
                JobProgress::Update {
                    id: 3,
                    message: "still running".into(),
                    pct: Some(50)
                },
            ]
        ));
        assert_eq!(state.active_ops.len(), 1);
        assert_eq!(state.active_ops[0].id, 3);
        assert_eq!(state.active_ops[0].progress_pct, Some(50));
        assert_eq!(state.error.as_deref(), Some("Operation was cancelled."));
    }

    #[test]
    fn all_completions_are_applied_and_failures_request_refresh() {
        let mut state = crate::state::AppState {
            repo_path: Some("repo-a".into()),
            ..Default::default()
        };
        assert!(apply_progress(
            &mut state,
            vec![
                started(1),
                started(2),
                JobProgress::Done { id: 1 },
                JobProgress::Done { id: 2 },
            ]
        ));
        assert!(state.active_ops.is_empty());
        assert!(apply_progress(
            &mut state,
            vec![
                started(3),
                JobProgress::Failed {
                    id: 3,
                    error: GitError::ConflictDetected {
                        conflicted_files: vec!["file.txt".into()]
                    },
                }
            ]
        ));
        assert!(!apply_progress(&mut state, vec![started(4)]));
    }

    #[test]
    fn completion_for_another_repository_does_not_change_active_error_or_refresh() {
        let mut state = crate::state::AppState {
            repo_path: Some("repo-b".into()),
            error: Some("keep b error".into()),
            ..Default::default()
        };
        assert!(!apply_progress(
            &mut state,
            vec![
                started(1),
                JobProgress::Failed {
                    id: 1,
                    error: GitError::Cancelled
                }
            ]
        ));
        assert_eq!(state.error.as_deref(), Some("keep b error"));
        assert!(state.active_ops.is_empty());
        assert!(state.console_log.last().unwrap().contains("repo-a"));
    }
}

/// Internal wire type sent through the job channel.
pub(crate) struct RawJob {
    pub reservation: crate::git::mutation::Reservation,
    pub kind: JobKind,
    pub id: JobId,
    pub cancel: Arc<AtomicBool>,
    pub repo_path: std::path::PathBuf,
    pub progress_tx: std::sync::mpsc::Sender<JobProgress>,
}
