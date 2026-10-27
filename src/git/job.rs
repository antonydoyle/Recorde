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
    BackgroundFetch,
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
            JobKind::FetchAll | JobKind::BackgroundFetch => "Fetching…",
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
        background: bool,
        fetch: bool,
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
    pub background: bool,
    pub fetch: bool,
}

pub struct BackgroundFetchStatus {
    pub repo_path: std::path::PathBuf,
    pub failed_at: std::time::Instant,
    pub message: String,
    pub consecutive_failures: u32,
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
                background,
                fetch,
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
                    background,
                    fetch,
                });
            }
            JobProgress::Update { id, message, pct } => {
                if let Some(op) = state.active_ops.iter_mut().find(|o| o.id == id) {
                    op.progress_msg = crate::auth::redact::text(&message);
                    op.progress_pct = pct;
                }
            }
            JobProgress::Done { id } => {
                let current = operation_is_current(state, id);
                let label = operation_label(state, id);
                if let Some(op) = state.active_ops.iter().find(|op| op.id == id && op.fetch) {
                    if state
                        .background_fetch
                        .as_ref()
                        .is_some_and(|status| status.repo_path == op.repo_path)
                    {
                        state.background_fetch = None;
                    }
                }
                state.active_ops.retain(|o| o.id != id);
                state.console_log.push(format!("{} done", label));
                refresh |= current;
            }
            JobProgress::Failed { id, error } => {
                let current = operation_is_current(state, id);
                let label = operation_label(state, id);
                let background_path = state
                    .active_ops
                    .iter()
                    .find(|op| op.id == id && op.background)
                    .map(|op| op.repo_path.clone());
                state.active_ops.retain(|o| o.id != id);
                let msg = error.user_message();
                state
                    .console_log
                    .push(format!("error: {} — {}", label, msg));
                if let Some(repo_path) = background_path {
                    let consecutive_failures = state
                        .background_fetch
                        .as_ref()
                        .filter(|status| status.repo_path == repo_path)
                        .map_or(1, |status| status.consecutive_failures.saturating_add(1));
                    state.background_fetch = Some(BackgroundFetchStatus {
                        repo_path,
                        failed_at: std::time::Instant::now(),
                        message: msg,
                        consecutive_failures,
                    });
                } else if current {
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
            background: false,
            fetch: false,
        }
    }

    fn fetch_started(id: JobId, background: bool) -> JobProgress {
        JobProgress::Started {
            repo_path: "repo-a".into(),
            id,
            label: "Fetching…".into(),
            cancel: Arc::new(AtomicBool::new(false)),
            background,
            fetch: true,
        }
    }

    #[test]
    fn background_failure_is_nonblocking_and_counts_consecutive_failures() {
        let mut state = crate::state::AppState {
            repo_path: Some("repo-a".into()),
            ..Default::default()
        };
        for id in 1..=2 {
            apply_progress(
                &mut state,
                vec![
                    fetch_started(id, true),
                    JobProgress::Failed {
                        id,
                        error: GitError::Command {
                            cmd: "fetch".into(),
                            stderr: "offline https://user:secret@example.com/repo".into(),
                        },
                    },
                ],
            );
            assert!(state.error.is_none());
            let status = state.background_fetch.as_ref().unwrap();
            assert_eq!(status.consecutive_failures, id as u32);
            assert!(status.message.contains("offline"));
            assert!(!status.message.contains("secret"));
            assert!(state.console_log.last().unwrap().contains("offline"));
            assert!(!state.console_log.last().unwrap().contains("secret"));
        }
    }

    #[test]
    fn foreground_fetch_failure_still_sets_error() {
        let mut state = crate::state::AppState {
            repo_path: Some("repo-a".into()),
            ..Default::default()
        };
        apply_progress(
            &mut state,
            vec![
                fetch_started(1, false),
                JobProgress::Failed {
                    id: 1,
                    error: GitError::CredentialFailed,
                },
            ],
        );
        assert!(state.error.is_some());
        assert!(state.background_fetch.is_none());
    }

    #[test]
    fn successful_manual_or_background_fetch_clears_background_failure() {
        for background in [false, true] {
            let mut state = crate::state::AppState {
                repo_path: Some("repo-a".into()),
                ..Default::default()
            };
            apply_progress(
                &mut state,
                vec![
                    fetch_started(1, true),
                    JobProgress::Failed {
                        id: 1,
                        error: GitError::CredentialFailed,
                    },
                    fetch_started(2, background),
                    JobProgress::Done { id: 2 },
                ],
            );
            assert!(state.error.is_none());
            assert!(state.background_fetch.is_none());
            assert!(state.active_ops.is_empty());
            apply_progress(
                &mut state,
                vec![
                    fetch_started(3, true),
                    JobProgress::Failed {
                        id: 3,
                        error: GitError::CredentialFailed,
                    },
                ],
            );
            assert_eq!(state.background_fetch.unwrap().consecutive_failures, 1);
        }
    }

    #[test]
    fn fetch_status_is_scoped_to_the_fetch_repository() {
        let mut state = crate::state::AppState {
            repo_path: Some("repo-b".into()),
            ..Default::default()
        };
        assert!(!apply_progress(
            &mut state,
            vec![
                fetch_started(1, true),
                JobProgress::Failed {
                    id: 1,
                    error: GitError::CredentialFailed,
                },
                JobProgress::Started {
                    repo_path: "repo-b".into(),
                    id: 2,
                    label: "Fetching…".into(),
                    cancel: Arc::new(AtomicBool::new(false)),
                    background: false,
                    fetch: true,
                },
            ],
        ));
        apply_progress(&mut state, vec![JobProgress::Done { id: 2 }]);
        assert_eq!(
            state.background_fetch.as_ref().unwrap().repo_path,
            std::path::PathBuf::from("repo-a")
        );
        assert!(state.error.is_none());
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
