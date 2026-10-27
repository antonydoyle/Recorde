//! Bounded native traversal with fixed roots and a retained page cache.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Mutex,
};

type Request = (usize, mpsc::Sender<Result<Vec<git2::Oid>, String>>);

pub(super) struct Cursor {
    pub path: std::path::PathBuf,
    pub roots: Vec<git2::Oid>,
    pub hidden: Vec<git2::Oid>,
    pub accelerated: bool,
    pub replacements: std::collections::BTreeMap<git2::Oid, git2::Oid>,
    failed: AtomicBool,
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
        super::replacement::refs(repo).is_ok_and(|r| r.is_empty())
            && !repo.is_shallow()
            && !common.join("info/grafts").exists()
            && repo
                .config()
                .is_ok_and(|c| c.get_bool("core.commitGraph").unwrap_or(true))
            && (common.join("objects/info/commit-graph").is_file()
                || common
                    .join("objects/info/commit-graphs/commit-graph-chain")
                    .is_file())
    }

    pub fn new(
        path: std::path::PathBuf,
        roots: Vec<git2::Oid>,
        hidden: Vec<git2::Oid>,
        replacements: std::collections::BTreeMap<git2::Oid, git2::Oid>,
    ) -> Self {
        let accelerated = git2::Repository::open(&path).is_ok_and(|r| Self::can_accelerate(&r));
        let worker_replacements = replacements.clone();
        let (sender, receiver) = mpsc::channel::<Request>();
        let worker_roots = roots.clone();
        let worker_hidden = hidden.clone();
        let worker_path = path.clone();
        let worker = std::thread::spawn(move || {
            let run = || -> Result<(), String> {
                native_pages(
                    &worker_path,
                    &worker_roots,
                    &worker_hidden,
                    accelerated,
                    &worker_replacements,
                    &receiver,
                )
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
            replacements,
            failed: AtomicBool::new(false),
            sender: Mutex::new(Some(sender)),
            worker: Mutex::new(Some(worker)),
        }
    }

    pub fn has_failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    pub fn read(&self, count: usize) -> Result<Vec<git2::Oid>, git2::Error> {
        let result = (|| {
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
        })();
        if result.is_err() {
            self.failed.store(true, Ordering::Release);
        }
        result
    }
}

/// Bound every subprocess response, retaining loaded IDs. Fixed OID roots prevent
/// moving refs from shifting the offset between pages. Native Git can use commit
/// generations to avoid walking the full history before returning a small page.
fn native_pages(
    path: &std::path::Path,
    roots: &[git2::Oid],
    hidden: &[git2::Oid],
    accelerated: bool,
    replacements: &std::collections::BTreeMap<git2::Oid, git2::Oid>,
    requests: &mpsc::Receiver<Request>,
) -> Result<(), String> {
    use std::io::Write;
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
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
                command.arg("rev-list").arg("--stdin");
                if accelerated {
                    command.arg("--topo-order");
                }
                command
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
                if super::replacement::refs(&repo).map_err(|e| e.to_string())? != *replacements {
                    return Err("Replacement refs changed; reload history.".into());
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
            None => {
                let ids = loaded[..loaded.len().min(count)].to_vec();
                if accelerated {
                    Ok(ids)
                } else {
                    order_loaded(&repo, ids, replacements)
                }
            }
        };
        let _ = reply.send(result);
    }
    Ok(())
}

// A cold repository has no generation numbers: requesting --topo-order would
// scan all ancestors before yielding its first row. Sort only the fetched DAG.
// More pages may refine ordering under skewed clocks; commit identity is stable.
fn order_loaded(
    repo: &git2::Repository,
    ids: Vec<git2::Oid>,
    replacements: &std::collections::BTreeMap<git2::Oid, git2::Oid>,
) -> Result<Vec<git2::Oid>, String> {
    use std::collections::{BinaryHeap, HashMap};
    let rows: HashMap<_, _> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let mut children = vec![0usize; ids.len()];
    let mut parents = vec![Vec::new(); ids.len()];
    for (i, id) in ids.iter().enumerate() {
        let commit = repo
            .find_commit(super::replacement::resolve(*id, replacements).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        for parent in commit.parent_ids() {
            if let Some(&p) = rows.get(&parent) {
                children[p] += 1;
                parents[i].push(p);
            }
        }
    }
    let mut ready = BinaryHeap::new();
    for (i, n) in children.iter().enumerate() {
        if *n == 0 {
            ready.push(std::cmp::Reverse(i));
        }
    }
    let mut result = Vec::with_capacity(ids.len());
    while let Some(std::cmp::Reverse(i)) = ready.pop() {
        result.push(ids[i]);
        for &p in &parents[i] {
            children[p] -= 1;
            if children[p] == 0 {
                ready.push(std::cmp::Reverse(p));
            }
        }
    }
    if result.len() != ids.len() {
        return Err("Replacement history contains an ancestry cycle.".into());
    }
    Ok(result)
}

impl Drop for Cursor {
    fn drop(&mut self) {
        self.sender.get_mut().unwrap().take();
        if let Some(worker) = self.worker.get_mut().unwrap().take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cursor_retains_the_callers_replacement_witness() {
        let fixture = crate::git::test_support::TestRepo::new();
        let head = fixture.commit("file", b"head");
        let repo = fixture.open();
        let signature = repo.signature().unwrap();
        let replacement = repo
            .commit(
                None,
                &signature,
                &signature,
                "replacement",
                &repo.find_commit(head).unwrap().tree().unwrap(),
                &[],
            )
            .unwrap();
        // The view captured its empty replacement map before another process
        // installed this ref. A cursor must reject, not silently recapture it.
        repo.reference(
            &format!("refs/replace/{head}"),
            replacement,
            true,
            "fixture",
        )
        .unwrap();
        let cursor = super::Cursor::new(repo.path().into(), vec![head], vec![], Default::default());
        assert!(cursor
            .read(2)
            .unwrap_err()
            .message()
            .contains("Replacement refs changed"));
        assert!(cursor.has_failed());
        assert_eq!(fixture.head(), head);
        assert_eq!(fixture.index_bytes("file"), b"head");
    }
}
