//! FIFO mutation reservations shared by local operations and the remote runner.
//! Linked worktrees share a queue because their refs and object store are shared.
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};

#[derive(Default)]
struct Queue {
    state: Mutex<QueueState>,
    ready: Condvar,
}
#[derive(Default)]
struct QueueState {
    next: u64,
    pending: VecDeque<u64>,
}

pub(crate) struct Reservation {
    queue: Arc<Queue>,
    id: u64,
    keys: Vec<PathBuf>,
}

pub(crate) struct ProcessLocks {
    _files: Vec<std::fs::File>,
}

fn key(path: &Path) -> PathBuf {
    let git_dir = git2::Repository::open(path)
        .map(|repo| repo.path().to_path_buf())
        .unwrap_or_else(|_| path.to_path_buf());
    let common = std::fs::read_to_string(git_dir.join("commondir"))
        .map(|name| git_dir.join(name.trim()))
        .unwrap_or(git_dir);
    common.canonicalize().unwrap_or(common)
}

pub(crate) fn reserve(path: &Path) -> Reservation {
    static QUEUES: OnceLock<Mutex<HashMap<PathBuf, Weak<Queue>>>> = OnceLock::new();
    let mut queues = QUEUES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    queues.retain(|_, queue| queue.strong_count() > 0);
    let common = lock_key(key(path));
    let alias = lock_key(canonical_destination(path));
    let existing = queues
        .get(&common)
        .and_then(Weak::upgrade)
        .or_else(|| queues.get(&alias).and_then(Weak::upgrade));
    let queue = match existing {
        Some(queue) => queue,
        None => Arc::new(Queue::default()),
    };
    queues.insert(common.clone(), Arc::downgrade(&queue));
    queues.insert(alias.clone(), Arc::downgrade(&queue));
    let mut state = queue.state.lock().unwrap_or_else(|e| e.into_inner());
    let id = state.next;
    state.next += 1;
    state.pending.push_back(id);
    drop(state);
    let mut keys = vec![common, alias];
    keys.sort();
    keys.dedup();
    Reservation { queue, id, keys }
}

// Normalize keys only; never alter the filesystem path used by an operation.
fn lock_key(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    }
    #[cfg(not(windows))]
    {
        path
    }
}

fn canonical_destination(path: &Path) -> PathBuf {
    if let Ok(path) = path.canonicalize() {
        return path;
    }
    if !path.is_absolute() {
        if let Ok(cwd) = std::env::current_dir() {
            return canonical_destination(&cwd.join(path));
        }
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => canonical_destination(parent).join(name),
        _ => path.into(),
    }
}

impl Reservation {
    /// Same-user Recorde instances share advisory OS locks; Git's own locks still
    /// protect against other Git clients. Stable files are retained to avoid inode races.
    pub(crate) fn lock_process(
        &self,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<ProcessLocks, String> {
        use fs2::FileExt;
        let root = dirs::cache_dir()
            .ok_or("Cannot locate the operation lock directory.")?
            .join("recorde/operation-locks");
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let mut files = Vec::new();
        for key in &self.keys {
            let id =
                git2::Oid::hash_object(git2::ObjectType::Blob, key.to_string_lossy().as_bytes())
                    .map_err(|e| e.to_string())?;
            let mut options = std::fs::OpenOptions::new();
            options.read(true).write(true).create(true).truncate(false);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let file = options
                .open(root.join(format!("{id}.lock")))
                .map_err(|e| e.to_string())?;
            loop {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err(
                        "Operation cancelled while waiting for another Recorde instance.".into(),
                    );
                }
                match file.try_lock_exclusive() {
                    Ok(()) => break,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            || error.raw_os_error()
                                == fs2::lock_contended_error().raw_os_error() =>
                    {
                        std::thread::sleep(std::time::Duration::from_millis(50))
                    }
                    Err(error) => return Err(format!("Cannot lock repository operation: {error}")),
                }
            }
            files.push(file);
        }
        Ok(ProcessLocks { _files: files })
    }

    pub(crate) fn wait_cancelled(&self, cancel: &std::sync::atomic::AtomicBool) -> bool {
        let mut state = self.queue.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return false;
            }
            if state.pending.front() == Some(&self.id) {
                return true;
            }
            state = self
                .queue
                .ready
                .wait_timeout(state, std::time::Duration::from_millis(50))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    #[cfg(test)]
    pub(crate) fn wait(&self) {
        let mut state = self.queue.state.lock().unwrap_or_else(|e| e.into_inner());
        while state.pending.front() != Some(&self.id) {
            state = self
                .queue
                .ready
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
    }
}

#[derive(Clone)]
pub struct LocalJob {
    pub id: u64,
    pub path: PathBuf,
    pub label: String,
    pub running: bool,
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
}

/// Progress is kept beside local jobs so existing LocalJob literals remain compatible.
#[derive(Clone)]
pub(crate) struct JobProgressHandle {
    id: u64,
    state: Weak<Mutex<crate::state::AppState>>,
}

impl JobProgressHandle {
    pub(crate) fn update(&self, message: &str, pct: Option<u8>) {
        if let Some(state) = self.state.upgrade() {
            let mut state = state.lock().unwrap();
            if state.local_jobs.iter().any(|job| job.id == self.id) {
                state.local_job_progress.insert(
                    self.id,
                    (
                        crate::auth::redact::text(message),
                        pct.map(|pct| pct.min(100)),
                    ),
                );
            }
        }
    }
}

pub(crate) fn spawn_job<T: Send + 'static>(
    path: PathBuf,
    label: &str,
    state: crate::state::SharedState,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
    complete: impl FnOnce(Result<T, String>, crate::state::SharedState) + Send + 'static,
) {
    spawn_job_with_progress(path, label, state, move |_| work(), complete);
}

pub(crate) fn spawn_job_with_progress<T: Send + 'static>(
    path: PathBuf,
    label: &str,
    state: crate::state::SharedState,
    work: impl FnOnce(JobProgressHandle) -> Result<T, String> + Send + 'static,
    complete: impl FnOnce(Result<T, String>, crate::state::SharedState) + Send + 'static,
) {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reservation = reserve(&path);
    state.lock().unwrap().local_jobs.push(LocalJob {
        id,
        path,
        label: label.into(),
        running: false,
        cancel: cancel.clone(),
    });
    let progress = JobProgressHandle {
        id,
        state: Arc::downgrade(&state),
    };
    std::thread::spawn(move || {
        let locks = if reservation.wait_cancelled(&cancel) {
            reservation.lock_process(&cancel)
        } else {
            Err("Operation cancelled before it started.".into())
        };
        let result = match locks {
            Ok(_locks) => {
                if let Some(job) = state
                    .lock()
                    .unwrap()
                    .local_jobs
                    .iter_mut()
                    .find(|job| job.id == id)
                {
                    job.running = true;
                }
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    super::process::with_cancellation(cancel, || work(progress))
                }))
                .unwrap_or_else(|_| {
                    Err(
                        "Operation stopped unexpectedly; inspect repository state before retrying."
                            .into(),
                    )
                })
            }
            Err(error) => Err(error),
        };
        drop(reservation);
        {
            let mut state = state.lock().unwrap();
            state.local_jobs.retain(|job| job.id != id);
            state.local_job_progress.remove(&id);
        }
        complete(result, state);
    });
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let mut state = self.queue.state.lock().unwrap_or_else(|e| e.into_inner());
        state.pending.retain(|&id| id != self.id);
        self.queue.ready.notify_all();
    }
}

/// Reserve before spawning so click/submission order determines execution order.
#[cfg(test)]
pub(crate) fn spawn<T: Send + 'static>(
    path: PathBuf,
    work: impl FnOnce() -> T + Send + 'static,
) -> std::thread::JoinHandle<T> {
    let reservation = reserve(&path);
    std::thread::spawn(move || {
        reservation.wait();
        work()
    })
}

/// Small menu actions share the queue and publish errors only to their own view.
pub(crate) fn spawn_operation(
    path: PathBuf,
    state: crate::state::SharedState,
    work: impl FnOnce(&Path) -> Result<(), String> + Send + 'static,
) {
    let (epoch, label) = {
        let mut s = state.lock().unwrap();
        (
            s.repo_epoch,
            s.remote_op
                .take()
                .unwrap_or_else(|| "Update repository".into()),
        )
    };
    let work_path = path.clone();
    spawn_job(
        path.clone(),
        &label,
        state,
        move || work(&work_path),
        move |result, state| {
            let current = {
                let mut s = state.lock().unwrap();
                let current = s.repo_path.as_ref() == Some(&path) && s.repo_epoch == epoch;
                if current {
                    s.remote_op = None;
                }
                if let Err(error) = result {
                    let error = crate::auth::redact::text(&error);
                    s.console_log
                        .push(format!("error ({}): {}", path.display(), error));
                    if current {
                        s.error = Some(error);
                    }
                }
                current
            };
            if current {
                crate::git::operations::refresh_repo(state);
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::TestRepo;

    #[test]
    fn local_job_cancellation_reaches_an_unlimited_running_process() {
        if let Some(marker) = std::env::var_os("RECORDE_UNLIMITED_LOCAL_JOB") {
            std::fs::write(marker, b"started").unwrap();
            // Bound the fixture even if the parent's cancellation assertion fails.
            std::thread::sleep(std::time::Duration::from_secs(3));
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("started");
        let child_marker = marker.clone();
        let state = crate::state::new_shared_state();
        let (tx, rx) = std::sync::mpsc::channel();
        spawn_job(
            root.path().into(),
            "Commit hook cancellation fixture",
            state.clone(),
            move || {
                let mut command =
                    super::super::process::background_command(std::env::current_exe().unwrap());
                command
                    .args([
                        "--exact",
                        concat!(
                            "git::mutation::tests::",
                            "local_job_cancellation_reaches_an_unlimited_running_process"
                        ),
                        "--nocapture",
                    ])
                    .env("RECORDE_UNLIMITED_LOCAL_JOB", child_marker);
                super::super::process::run_with_deadline(
                    &mut command,
                    &std::sync::atomic::AtomicBool::new(false),
                    super::super::process::Deadline::None,
                )
                .map(|_| ())
                .map_err(|error| error.user_message())
            },
            move |result, _| tx.send(result).unwrap(),
        );
        let cancel = state.lock().unwrap().local_jobs[0].cancel.clone();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        let result = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(marker.exists(), "fixture did not start");
        assert!(result.unwrap_err().contains("cancelled"));
        assert!(state.lock().unwrap().local_jobs.is_empty());
    }

    #[test]
    fn local_job_progress_is_published_and_retired_on_completion() {
        let root = tempfile::tempdir().unwrap();
        let state = crate::state::new_shared_state();
        let (progress_tx, progress_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        spawn_job_with_progress(
            root.path().into(),
            "Clone fixture",
            state.clone(),
            move |progress| {
                progress.update("Receiving objects", Some(45));
                progress_tx.send(()).unwrap();
                release_rx
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .map_err(|error| error.to_string())
            },
            move |result, _| done_tx.send(result).unwrap(),
        );
        progress_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        {
            let state = state.lock().unwrap();
            let job = &state.local_jobs[0];
            assert_eq!(
                state.local_job_progress.get(&job.id),
                Some(&("Receiving objects".into(), Some(45)))
            );
        }
        release_tx.send(()).unwrap();
        done_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        let state = state.lock().unwrap();
        assert!(state.local_jobs.is_empty());
        assert!(state.local_job_progress.is_empty());
    }

    #[test]
    fn separate_process_waits_for_repository_lock_and_resumes_after_release() {
        use std::sync::atomic::AtomicBool;
        let test_name = "git::mutation::tests::separate_process_waits_for_repository_lock_and_resumes_after_release";
        if let Some(path) = std::env::var_os("RECORDE_MUTATION_LOCK_CHILD") {
            let path = PathBuf::from(path);
            let reservation = reserve(&path);
            std::fs::write(path.with_extension("ready"), b"ready").unwrap();
            let _locks = reservation.lock_process(&AtomicBool::new(false)).unwrap();
            std::fs::write(path.with_extension("acquired"), b"acquired").unwrap();
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("repository");
        // Capture a destination reservation before init; the child sees an initialized repository.
        let reservation = reserve(&path);
        let locks = reservation.lock_process(&AtomicBool::new(false)).unwrap();
        git2::Repository::init(&path).unwrap();
        let mut command = crate::git::process::background_command(std::env::current_exe().unwrap());
        command
            .args(["--exact", test_name, "--nocapture"])
            .env("RECORDE_MUTATION_LOCK_CHILD", &path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        struct ReapChild(std::process::Child);
        impl Drop for ReapChild {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        // Assertions can unwind before the explicit timeout; reap on every path.
        let mut child = ReapChild(command.spawn().unwrap());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while !path.with_extension("ready").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(path.with_extension("ready").exists());
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!path.with_extension("acquired").exists());
        drop(locks);
        while child.0.try_wait().unwrap().is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if child.0.try_wait().unwrap().is_none() {
            let _ = child.0.kill();
            panic!("child lock did not resume");
        }
        assert!(child.0.wait().unwrap().success());
        assert!(path.with_extension("acquired").exists());
    }

    #[test]
    fn waiting_for_another_instance_can_be_cancelled() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let repo = TestRepo::new();
        let first = reserve(repo.path());
        let held = first.lock_process(&AtomicBool::new(false)).unwrap();
        let second = reserve(repo.path());
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let worker = std::thread::spawn(move || second.lock_process(&signal).err().unwrap());
        cancel.store(true, Ordering::Relaxed);
        assert!(worker.join().unwrap().contains("cancelled"));
        drop(held);
    }
    #[test]
    fn clone_destination_retains_queue_after_repository_creation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("new-repo");
        let first = reserve(&path);
        git2::Repository::init(&path).unwrap();
        let second = reserve(&path);
        assert!(Arc::ptr_eq(&first.queue, &second.queue));
    }

    #[test]
    fn queued_local_job_cancels_before_running_and_retires_its_status() {
        let repo = TestRepo::new();
        let gate = reserve(repo.path());
        let state = crate::state::new_shared_state();
        let (tx, rx) = std::sync::mpsc::channel();
        spawn_job(
            repo.path().into(),
            "Test operation",
            state.clone(),
            || -> Result<(), String> {
                panic!("cancelled job must never execute");
            },
            move |result, _| {
                tx.send(result).unwrap();
            },
        );
        state.lock().unwrap().local_jobs[0]
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
            .unwrap_err()
            .contains("cancelled"));
        assert!(state.lock().unwrap().local_jobs.is_empty());
        drop(gate);
    }

    #[test]
    fn writes_are_ordered_across_path_aliases_and_cancelled_reservations() {
        let repo = TestRepo::new();
        repo.commit("file.txt", b"base\n");
        let first = reserve(repo.path());
        let skipped = reserve(&repo.path().join("."));
        let last = reserve(repo.path());
        assert!(Arc::ptr_eq(&first.queue, &last.queue));
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            last.wait();
            tx.send(()).unwrap();
        });
        first.wait();
        drop(skipped);
        assert!(rx.try_recv().is_err());
        drop(first);
        rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        handle.join().unwrap();
    }
    #[test]
    fn linked_worktrees_share_queue_but_unrelated_repositories_do_not() {
        let repo = TestRepo::new();
        repo.commit("file.txt", b"base\n");
        let linked = tempfile::tempdir().unwrap();
        let worktree = linked.path().join("linked");
        repo.git_ok(&[
            "worktree",
            "add",
            "-b",
            "linked",
            worktree.to_str().unwrap(),
        ]);
        let other = TestRepo::new();
        let a = reserve(repo.path());
        let b = reserve(&worktree);
        let c = reserve(other.path());
        assert!(Arc::ptr_eq(&a.queue, &b.queue));
        assert!(!Arc::ptr_eq(&a.queue, &c.queue));
        c.wait();
    }
    #[test]
    fn staging_and_commit_execute_in_submission_order() {
        let repo = TestRepo::new();
        repo.commit("file.txt", b"base\n");
        repo.write("file.txt", b"new\n");
        let path = repo.path().to_path_buf();
        let first = spawn(path.clone(), move || {
            crate::git::merge::run_git_seq(&path, &["add", "--", "file.txt"]).unwrap();
        });
        let path = repo.path().to_path_buf();
        let second = spawn(path.clone(), move || {
            crate::git::merge::run_git_seq(&path, &["commit", "-m", "ordered"]).unwrap();
        });
        first.join().unwrap();
        second.join().unwrap();
        let opened = repo.open();
        let tree = opened.head().unwrap().peel_to_tree().unwrap();
        let entry = tree.get_path(Path::new("file.txt")).unwrap();
        assert_eq!(opened.find_blob(entry.id()).unwrap().content(), b"new\n");
    }
}
