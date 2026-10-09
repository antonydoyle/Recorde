//! Keep the native revwalk alive between history pages. The worker owns all
//! libgit2 borrows and exits when the last graph snapshot releases its cursor.
use std::sync::{mpsc, Mutex};

type Request = (usize, mpsc::Sender<Result<Vec<git2::Oid>, String>>);

pub(super) struct Cursor {
    pub path: std::path::PathBuf,
    pub roots: Vec<git2::Oid>,
    pub hidden: Vec<git2::Oid>,
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
    pub fn new(path: std::path::PathBuf, roots: Vec<git2::Oid>, hidden: Vec<git2::Oid>) -> Self {
        let (sender, receiver) = mpsc::channel::<Request>();
        let worker_roots = roots.clone();
        let worker_hidden = hidden.clone();
        let worker_path = path.clone();
        let worker = std::thread::spawn(move || {
            let run = || -> Result<(), String> {
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

impl Drop for Cursor {
    fn drop(&mut self) {
        self.sender.get_mut().unwrap().take();
        if let Some(worker) = self.worker.get_mut().unwrap().take() {
            let _ = worker.join();
        }
    }
}
