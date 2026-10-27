use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use crate::git::error::GitError;
use crate::git::job::{JobHandle, JobKind, JobProgress, RawJob};

pub struct GitRunner {
    job_tx: Sender<RawJob>,
    progress_rx: Receiver<JobProgress>,
    progress_tx: Sender<JobProgress>,
    next_id: u64,
    pending: std::collections::HashMap<u64, PathBuf>,
}

impl GitRunner {
    pub fn new() -> Self {
        let (job_tx, job_rx) = mpsc::channel::<RawJob>();
        let (progress_tx, progress_rx) = mpsc::channel::<JobProgress>();

        let worker_progress_tx = progress_tx.clone();
        std::thread::spawn(move || worker_loop(job_rx, worker_progress_tx));

        Self {
            job_tx,
            progress_rx,
            progress_tx,
            next_id: 0,
            pending: Default::default(),
        }
    }

    pub fn submit(&mut self, kind: JobKind, repo_path: PathBuf) -> JobHandle {
        use std::sync::{atomic::AtomicBool, Arc};

        let id = self.next_id;
        self.next_id += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        self.pending.insert(id, repo_path.clone());

        let raw = RawJob {
            reservation: crate::git::mutation::reserve(&repo_path),
            kind,
            id,
            cancel: cancel.clone(),
            repo_path,
            progress_tx: self.progress_tx.clone(),
        };
        let _ = self.job_tx.send(raw);

        JobHandle { id, cancel }
    }

    /// Drain all pending progress events. Called once per frame from `update()`.
    pub fn drain_progress(&mut self) -> Vec<JobProgress> {
        let mut events = Vec::new();
        while let Ok(p) = self.progress_rx.try_recv() {
            if let JobProgress::Done { id } | JobProgress::Failed { id, .. } = &p {
                self.pending.remove(id);
            }
            events.push(p);
        }
        events
    }

    pub fn has_any_jobs(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn has_jobs(&self, path: &std::path::Path) -> bool {
        self.pending.values().any(|p| p == path)
    }
}

fn worker_loop(job_rx: Receiver<RawJob>, progress_tx: Sender<JobProgress>) {
    while let Ok(job) = job_rx.recv() {
        let id = job.id;
        let label = job.kind.label().to_string();
        let cancel = job.cancel.clone();

        let _ = progress_tx.send(JobProgress::Started {
            id,
            repo_path: job.repo_path.clone(),
            label,
            cancel,
            background: matches!(&job.kind, JobKind::BackgroundFetch),
            fetch: matches!(&job.kind, JobKind::FetchAll | JobKind::BackgroundFetch),
        });

        let result = if !job.reservation.wait_cancelled(&job.cancel) {
            Err(GitError::Cancelled)
        } else {
            match job.reservation.lock_process(&job.cancel) {
                Ok(_locks) => dispatch(&job),
                Err(_) if job.cancel.load(std::sync::atomic::Ordering::Relaxed) => {
                    Err(GitError::Cancelled)
                }
                Err(error) => Err(GitError::Command {
                    cmd: "Wait for repository".into(),
                    stderr: error,
                }),
            }
        };

        match result {
            Ok(()) => {
                let _ = progress_tx.send(JobProgress::Done { id });
            }
            Err(e) => {
                let _ = progress_tx.send(JobProgress::Failed { id, error: e });
            }
        }
    }
}

fn dispatch(job: &RawJob) -> Result<(), GitError> {
    use crate::git::remote;
    match &job.kind {
        JobKind::FetchAll | JobKind::BackgroundFetch => {
            remote::run_fetch_all(&job.repo_path, &job.cancel, job.id, &job.progress_tx)
        }
        JobKind::PullBranch => {
            remote::run_pull(&job.repo_path, &job.cancel, job.id, &job.progress_tx)
        }
        JobKind::PullWithRebase => {
            remote::run_pull_with_rebase(&job.repo_path, &job.cancel, job.id, &job.progress_tx)
        }
        JobKind::PullFfOnly => {
            remote::run_pull_ff_only(&job.repo_path, &job.cancel, job.id, &job.progress_tx)
        }
        JobKind::PushBranch => {
            remote::run_push(&job.repo_path, &job.cancel, job.id, &job.progress_tx)
        }
        JobKind::PushWithLease(lease) => remote::run_push_with_lease_progress(
            &job.repo_path,
            lease,
            &job.cancel,
            job.id,
            &job.progress_tx,
        ),
        JobKind::PushSetUpstream => {
            remote::run_push_set_upstream(&job.repo_path, &job.cancel, job.id, &job.progress_tx)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::TestRepo;
    use std::sync::atomic::Ordering;

    #[test]
    fn cancelling_a_queued_push_does_not_change_the_remote() {
        let repo = TestRepo::new();
        let original = repo.commit("file.txt", b"base\n");
        let remote_dir = tempfile::tempdir().unwrap();
        let remote = git2::Repository::init_bare(remote_dir.path()).unwrap();
        repo.git_ok(&[
            "remote",
            "add",
            "origin",
            remote_dir.path().to_str().unwrap(),
        ]);
        repo.git_ok(&["push", "-u", "origin", "main"]);
        repo.commit("file.txt", b"must not push\n");
        let gate = crate::git::mutation::reserve(repo.path());
        let mut runner = GitRunner::new();
        let lease = crate::git::remote::capture_lease(repo.path()).unwrap();
        let handle = runner.submit(JobKind::PushWithLease(lease), repo.path().to_path_buf());
        handle.cancel.store(true, Ordering::Relaxed);
        drop(gate);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut cancelled = false;
        while std::time::Instant::now() < deadline && !cancelled {
            for event in runner.drain_progress() {
                match event {
                    JobProgress::Failed {
                        error: GitError::Cancelled,
                        ..
                    } => cancelled = true,
                    JobProgress::Done { .. } => panic!("cancelled job ran"),
                    JobProgress::Failed { error, .. } => panic!("unexpected failure: {error}"),
                    _ => {}
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(cancelled);
        assert_eq!(remote.refname_to_id("refs/heads/main").unwrap(), original);
    }
}
