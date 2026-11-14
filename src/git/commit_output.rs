//! Ephemeral commit diagnostics, redacted before insertion into application state.
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex, Weak,
};
use std::time::{Duration, Instant};

const STREAM_LIMIT: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitOutcome {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    StoppedWithHeadChange,
}

fn cancelled_message(message: &str) -> bool {
    message.lines().any(|line| {
        matches!(
            line.trim(),
            "Operation was cancelled."
                | "Operation cancelled before it started."
                | "Operation cancelled while waiting for another Recorde instance."
                | "Operation cancelled."
        )
    })
}

#[derive(Clone)]
pub struct CommitOutput {
    pub id: u64,
    pub repo: PathBuf,
    pub job_id: Option<u64>,
    pub started: Option<Instant>,
    pub elapsed: Option<Duration>,
    pub outcome: CommitOutcome,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub detail: String,
    pub index_changed: bool,
}

/// Redact complete captured text before taking a UTF-8-safe display tail. When
/// the runner dropped an earlier prefix, discard its first potentially partial
/// line too: a cut credential prefix must not expose the rest of that line.
pub(crate) fn display_stream(bytes: &[u8], truncated: bool) -> (String, bool) {
    let bytes = if truncated {
        bytes
            .iter()
            .position(|b| *b == b'\n')
            .map_or(&[][..], |n| &bytes[n + 1..])
    } else {
        bytes
    };
    let mut text = crate::auth::redact::text(&String::from_utf8_lossy(bytes));
    let shortened = text.len() > STREAM_LIMIT;
    if shortened {
        let mut start = text.len() - STREAM_LIMIT;
        while !text.is_char_boundary(start) {
            start += 1;
        }
        text = text[start..].to_string();
    }
    (text, shortened || truncated)
}

#[derive(Clone)]
pub(crate) struct OutputHandle {
    id: u64,
    repo: PathBuf,
    state: Weak<Mutex<crate::state::AppState>>,
}

impl OutputHandle {
    pub fn begin(repo: &Path, state: &crate::state::SharedState) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let handle = Self {
            id,
            repo: repo.into(),
            state: Arc::downgrade(state),
        };
        let mut state = state.lock().unwrap();
        state.commit_outputs.insert(
            repo.into(),
            CommitOutput {
                id,
                repo: repo.into(),
                job_id: None,
                started: None,
                elapsed: None,
                outcome: CommitOutcome::Queued,
                stdout: String::new(),
                stderr: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
                detail: String::new(),
                index_changed: false,
            },
        );
        while state.commit_outputs.len() > 10 {
            let oldest = state
                .commit_outputs
                .iter()
                .min_by_key(|(_, r)| r.id)
                .map(|(p, _)| p.clone())
                .unwrap();
            state.commit_outputs.remove(&oldest);
        }
        handle
    }

    fn update(&self, apply: impl FnOnce(&mut CommitOutput)) {
        if let Some(state) = self.state.upgrade() {
            if let Some(report) = state
                .lock()
                .unwrap()
                .commit_outputs
                .get_mut(&self.repo)
                .filter(|r| r.id == self.id)
            {
                apply(report);
            }
        }
    }

    pub fn job(&self, id: u64) {
        self.update(|r| r.job_id = Some(id));
    }
    pub fn started(&self) {
        self.update(|r| {
            r.started = Some(Instant::now());
            r.outcome = CommitOutcome::Running;
        });
    }

    pub fn observed(&self, output: super::process::DiagnosticOutput<'_>) {
        let (stdout, stdout_truncated) = display_stream(output.stdout, output.stdout_truncated);
        let (stderr, stderr_truncated) = display_stream(output.stderr, output.stderr_truncated);
        self.update(|r| {
            r.stdout = stdout;
            r.stderr = stderr;
            r.stdout_truncated = stdout_truncated;
            r.stderr_truncated = stderr_truncated;
            r.elapsed = Some(output.elapsed);
            r.outcome = if output.cancelled {
                CommitOutcome::Cancelled
            } else if output.success == Some(true) {
                CommitOutcome::Succeeded
            } else {
                CommitOutcome::Failed
            };
        });
    }

    pub fn finished(
        &self,
        result: &Result<String, String>,
        head_changed: bool,
        index_changed: bool,
    ) {
        let detail = result
            .as_ref()
            .err()
            .map(|e| display_stream(e.as_bytes(), false).0)
            .unwrap_or_default();
        self.update(|r| {
            r.index_changed = index_changed;
            if result.is_err() && head_changed {
                r.outcome = CommitOutcome::StoppedWithHeadChange;
                r.detail = "HEAD changed while the command stopped. A commit may already exist; inspect history before retrying. The draft is retained.".into();
            } else {
                r.detail = detail;
                if result.is_ok() { r.outcome = CommitOutcome::Succeeded; }
                else if matches!(r.outcome, CommitOutcome::Queued | CommitOutcome::Running) {
                    r.outcome = if cancelled_message(&r.detail) { CommitOutcome::Cancelled } else { CommitOutcome::Failed };
                }
            }
        });
    }

    pub fn finish_if_pending(&self, result: &Result<String, String>) {
        let detail = result
            .as_ref()
            .err()
            .map(|e| display_stream(e.as_bytes(), false).0)
            .unwrap_or_default();
        self.update(|r| {
            if matches!(r.outcome, CommitOutcome::Queued | CommitOutcome::Running) {
                r.outcome = if result.is_ok() {
                    CommitOutcome::Succeeded
                } else if cancelled_message(&detail) {
                    CommitOutcome::Cancelled
                } else {
                    CommitOutcome::Failed
                };
                r.detail = detail;
            }
        });
    }
}

pub(crate) fn head(path: &Path) -> Option<(Option<String>, Option<git2::Oid>)> {
    let repo = git2::Repository::open(path).ok()?;
    let reference = repo.find_reference("HEAD").ok()?;
    Some((
        reference
            .symbolic_target()
            .ok()
            .flatten()
            .map(str::to_string),
        repo.head().ok().and_then(|r| r.target()),
    ))
}

pub(crate) fn index(path: &Path) -> Option<Vec<(Vec<u8>, git2::Oid, u32, u16)>> {
    let repo = git2::Repository::open(path).ok()?;
    let index = repo.index().ok()?;
    Some(
        index
            .iter()
            .map(|e| (e.path, e.id, e.mode, e.flags & 0x3000))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_output_redacts_bounds_and_drops_a_truncated_partial_secret_line() {
        crate::auth::redact::remember("t11-output-secret");
        let source = format!(
            "{}\nhttps://user:password@example.invalid/repo?token=query-secret t11-output-secret\n",
            "x".repeat(32_000)
        );
        let (text, truncated) = display_stream(source.as_bytes(), false);
        assert!(truncated && text.len() <= STREAM_LIMIT);
        for secret in ["password", "query-secret", "t11-output-secret"] {
            assert!(!text.contains(secret));
        }
        let (text, truncated) = display_stream(
            b"partial-secret-with-prefix-cut\nsafe retained line\n",
            true,
        );
        assert_eq!(text, "safe retained line\n");
        assert!(truncated);
        assert!(display_stream(b"partial-secret-no-newline", true)
            .0
            .is_empty());
    }

    #[test]
    fn commit_output_stale_attempt_cannot_replace_newer_or_other_repository_report() {
        let state = crate::state::new_shared_state();
        let old = OutputHandle::begin(Path::new("repo-a"), &state);
        let newest = OutputHandle::begin(Path::new("repo-a"), &state);
        let other = OutputHandle::begin(Path::new("repo-b"), &state);
        old.finished(&Err("old error".into()), true, true);
        newest.finished(&Ok("success".into()), true, false);
        other.finished(&Err("failure".into()), false, false);
        let state = state.lock().unwrap();
        assert_eq!(
            state.commit_outputs[Path::new("repo-a")].outcome,
            CommitOutcome::Succeeded
        );
        assert_eq!(
            state.commit_outputs[Path::new("repo-b")].outcome,
            CommitOutcome::Failed
        );
        assert!(state.commit_outputs[Path::new("repo-a")].detail.is_empty());
    }

    #[test]
    fn commit_output_hook_text_cannot_turn_failure_into_cancellation() {
        let state = crate::state::new_shared_state();
        let report = OutputHandle::begin(Path::new("repo-a"), &state);
        report.observed(super::super::process::DiagnosticOutput {
            stdout: &[],
            stderr: b"not cancelled: hook rejected",
            stdout_truncated: false,
            stderr_truncated: false,
            elapsed: Duration::from_millis(2),
            success: Some(false),
            cancelled: false,
        });
        report.finished(&Err("not cancelled: hook rejected".into()), false, false);
        assert_eq!(
            state.lock().unwrap().commit_outputs[Path::new("repo-a")].outcome,
            CommitOutcome::Failed
        );
    }

    #[test]
    fn commit_output_head_witness_detects_symbolic_switch_at_same_commit() {
        let repo = crate::git::test_support::TestRepo::new();
        let oid = repo.commit("file", b"same commit\n");
        let before = head(repo.path()).unwrap();
        repo.open()
            .reference("refs/heads/other", oid, false, "fixture")
            .unwrap();
        repo.git_ok(&["symbolic-ref", "HEAD", "refs/heads/other"]);
        let after = head(repo.path()).unwrap();
        assert_eq!(before.1, after.1);
        assert_ne!(before.0, after.0);
    }
}
