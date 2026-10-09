//! Keep the native revwalk alive between history pages. The worker owns all
//! libgit2 borrows and exits when the last graph snapshot releases its cursor.
use std::sync::{mpsc, Mutex};

type Request = (usize, mpsc::Sender<Result<Vec<git2::Oid>, String>>);

pub(super) struct Cursor {
    pub path: std::path::PathBuf,
    pub roots: Vec<git2::Oid>,
    pub hidden: Vec<git2::Oid>,
    pub accelerated: bool,
    sender: Mutex<Option<mpsc::Sender<Request>>>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl std::fmt::Debug for Cursor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HistoryCursor")
            .field("roots", &self.roots)
            .finish_non_exhaustive()
    }
}

impl Cursor {
    pub fn can_accelerate(repo: &git2::Repository) -> bool {
        let git_dir = repo.path();
        let common = std::fs::read_to_string(git_dir.join("commondir"))
            .map(|s| git_dir.join(s.trim()))
            .unwrap_or_else(|_| git_dir.into());
        !repo.is_shallow()
            && !common.join("info/grafts").exists()
            && repo
                .config()
                .is_ok_and(|c| c.get_bool("core.commitGraph").unwrap_or(true))
            && (common.join("objects/info/commit-graph").is_file()
                || common
                    .join("objects/info/commit-graphs/commit-graph-chain")
                    .is_file())
    }

    pub fn new(path: std::path::PathBuf, roots: Vec<git2::Oid>, hidden: Vec<git2::Oid>) -> Self {
        let accelerated = git2::Repository::open(&path).is_ok_and(|r| Self::can_accelerate(&r));
        let (sender, receiver) = mpsc::channel::<Request>();
        let worker_roots = roots.clone();
        let worker_hidden = hidden.clone();
        let worker_path = path.clone();
        let worker = std::thread::spawn(move || {
            let run = || -> Result<(), String> {
                if accelerated {
                    return native_pages(&worker_path, &worker_roots, &worker_hidden, &receiver);
                }
                let repo = git2::Repository::open(worker_path).map_err(|e| e.to_string())?;
                let mut walk = repo.revwalk().map_err(|e| e.to_string())?;
                for root in worker_roots {
                    walk.push(root).map_err(|e| e.to_string())?;
                }
                for root in worker_hidden {
                    walk.hide(root).map_err(|e| e.to_string())?;
                }
                walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)
                    .map_err(|e| e.to_string())?;
                let mut loaded = Vec::new();
                let mut finished = false;
                let mut failure = None;
                while let Ok((count, reply)) = receiver.recv() {
                    while loaded.len() < count && !finished {
                        match walk.next() {
                            Some(Ok(oid)) => loaded.push(oid),
                            Some(Err(error)) => {
                                failure = Some(error.to_string());
                                finished = true;
                            }
                            None => finished = true,
                        }
                    }
                    let result = match &failure {
                        Some(error) => Err(error.clone()),
                        None => Ok(loaded[..loaded.len().min(count)].to_vec()),
                    };
                    let _ = reply.send(result);
                }
                Ok(())
            };
            if let Err(error) = run() {
                // Startup failures still answer callers; never leave a waiter hanging.
                while let Ok((_, reply)) = receiver.recv() {
                    let _ = reply.send(Err(error.clone()));
                }
            }
        });
        Self {
            path,
            roots,
            hidden,
            accelerated,
            sender: Mutex::new(Some(sender)),
            worker: Mutex::new(Some(worker)),
        }
    }

    pub fn read(&self, count: usize) -> Result<Vec<git2::Oid>, git2::Error> {
        let (reply, response) = mpsc::channel();
        self.sender
            .lock()
            .unwrap()
            .as_ref()
            .ok_or_else(|| git2::Error::from_str("History cursor closed"))?
            .send((count, reply))
            .map_err(|_| git2::Error::from_str("History worker stopped"))?;
        response
            .recv()
            .map_err(|_| git2::Error::from_str("History worker stopped"))?
            .map_err(|e| git2::Error::from_str(&e))
    }
}

/// Bound every subprocess response, retaining loaded IDs. Fixed OID roots prevent
/// moving refs from shifting the offset between pages. Native Git can use commit
/// generations to avoid walking the full history before returning a small page.
fn native_pages(
    path: &std::path::Path,
    roots: &[git2::Oid],
    hidden: &[git2::Oid],
    requests: &mpsc::Receiver<Request>,
) -> Result<(), String> {
    use std::io::Write;
    let mut revisions = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    for root in roots {
        writeln!(revisions, "{root}").map_err(|e| e.to_string())?;
    }
    for root in hidden {
        writeln!(revisions, "^{root}").map_err(|e| e.to_string())?;
    }
    revisions.flush().map_err(|e| e.to_string())?;
    let mut loaded = Vec::new();
    let mut finished = roots.is_empty();
    let mut failure = None;
    while let Ok((count, reply)) = requests.recv() {
        while loaded.len() < count && !finished {
            let wanted = (count - loaded.len()).min(4096);
            let page = || -> Result<Vec<git2::Oid>, String> {
                let mut command = super::process::command(path);
                command
                    .args([
                        "--no-replace-objects",
                        "rev-list",
                        "--topo-order",
                        "--stdin",
                    ])
                    .arg(format!("--skip={}", loaded.len()))
                    .arg(format!("--max-count={wanted}"));
                let input = std::fs::File::open(revisions.path()).map_err(|e| e.to_string())?;
                let out = super::process::run_with_stdin(
                    &mut command,
                    &std::sync::atomic::AtomicBool::new(false),
                    std::time::Duration::from_secs(60),
                    input.into(),
                )
                .map_err(|e| e.user_message())?;
                if !out.status.success() {
                    return Err(crate::auth::redact::text(&String::from_utf8_lossy(
                        &out.stderr,
                    )));
                }
                let text = std::str::from_utf8(&out.stdout).map_err(|e| e.to_string())?;
                let ids: Vec<_> = text
                    .lines()
                    .map(|s| git2::Oid::from_str(s).map_err(|e| e.to_string()))
                    .collect::<Result<_, _>>()?;
                if ids.len() > wanted {
                    return Err("Git returned an oversized history page.".into());
                }
                Ok(ids)
            };
            match page() {
                Ok(ids) => {
                    finished = ids.len() < wanted;
                    loaded.extend(ids);
                }
                Err(error) => {
                    failure = Some(error);
                    finished = true;
                }
            }
        }
        let result = match &failure {
            Some(error) => Err(error.clone()),
            None => Ok(loaded[..loaded.len().min(count)].to_vec()),
        };
        let _ = reply.send(result);
    }
    Ok(())
}

impl Drop for Cursor {
    fn drop(&mut self) {
        self.sender.get_mut().unwrap().take();
        if let Some(worker) = self.worker.get_mut().unwrap().take() {
            let _ = worker.join();
        }
    }
}
