use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use crate::git::error::GitError;
use crate::git::job::{JobHandle, JobKind, JobProgress, RawJob};

pub struct GitRunner {
    job_tx: Sender<RawJob>,
    progress_rx: Receiver<JobProgress>,
    progress_tx: Sender<JobProgress>,
    next_id: u64,
}

impl GitRunner {
    pub fn new() -> Self {
        let (job_tx, job_rx) = mpsc::channel::<RawJob>();
        let (progress_tx, progress_rx) = mpsc::channel::<JobProgress>();

        let worker_progress_tx = progress_tx.clone();
        std::thread::spawn(move || worker_loop(job_rx, worker_progress_tx));

        Self { job_tx, progress_rx, progress_tx, next_id: 0 }
    }

    pub fn submit(&mut self, kind: JobKind, repo_path: PathBuf) -> JobHandle {
        use std::sync::{Arc, atomic::AtomicBool};

        let id = self.next_id;
        self.next_id += 1;
        let cancel = Arc::new(AtomicBool::new(false));

        let raw = RawJob {
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
    pub fn drain_progress(&self) -> Vec<JobProgress> {
        let mut events = Vec::new();
        while let Ok(p) = self.progress_rx.try_recv() {
            events.push(p);
        }
        events
    }
}

fn worker_loop(job_rx: Receiver<RawJob>, progress_tx: Sender<JobProgress>) {
    while let Ok(job) = job_rx.recv() {
        let id = job.id;
        let label = job.kind.label().to_string();
        let cancel = job.cancel.clone();

        let _ = progress_tx.send(JobProgress::Started { id, label, cancel });

        let result = dispatch(&job);

        match result {
            Ok(()) => { let _ = progress_tx.send(JobProgress::Done { id }); }
            Err(e) => { let _ = progress_tx.send(JobProgress::Failed { id, error: e }); }
        }
    }
}

fn dispatch(job: &RawJob) -> Result<(), GitError> {
    use crate::git::remote;
    match job.kind {
        JobKind::FetchAll        => remote::run_fetch_all(&job.repo_path, &job.cancel, job.id, &job.progress_tx),
        JobKind::PullBranch      => remote::run_pull(&job.repo_path, &job.cancel, job.id, &job.progress_tx),
        JobKind::PullWithRebase  => remote::run_pull_with_rebase(&job.repo_path, &job.cancel, job.id, &job.progress_tx),
        JobKind::PullFfOnly      => remote::run_pull_ff_only(&job.repo_path, &job.cancel, job.id, &job.progress_tx),
        JobKind::PushBranch      => remote::run_push(&job.repo_path, &job.cancel, job.id, &job.progress_tx),
        JobKind::PushWithLease   => remote::run_push_with_lease(&job.repo_path, &job.cancel, job.id, &job.progress_tx),
        JobKind::PushSetUpstream => remote::run_push_set_upstream(&job.repo_path, &job.cancel, job.id, &job.progress_tx),
    }
}
