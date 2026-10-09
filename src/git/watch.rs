//! Debounced native filesystem notifications, with focus and periodic recovery.
use notify::{RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refresh {
    Worktree,
    History,
}
pub(crate) struct RepoWatch {
    path: PathBuf,
    _watcher: Option<notify::RecommendedWatcher>,
    flags: Arc<AtomicU8>,
    pending: u8,
    last_event: Instant,
    pending_since: Instant,
    last_fallback: Instant,
}

fn classify(path: &Path, git_dirs: &[PathBuf]) -> u8 {
    for git_dir in git_dirs {
        if let Ok(relative) = path.strip_prefix(git_dir) {
            let first = relative
                .components()
                .next()
                .map(|c| c.as_os_str().to_string_lossy());
            return match first.as_deref() {
                Some("objects" | "logs" | "recorde") => 0,
                Some("index" | "index.lock") => 1,
                _ => 2,
            };
        }
    }
    1
}

fn classify_event(event: &notify::Event, git_dirs: &[PathBuf]) -> u8 {
    // Coalesced/dropped events can omit individual paths. A full refresh also
    // reloads working changes, so it is the safe response to a rescan request.
    if event.need_rescan() {
        2
    } else if matches!(event.kind, notify::EventKind::Access(_)) {
        0
    } else {
        event
            .paths
            .iter()
            .fold(0, |flags, path| flags | classify(path, git_dirs))
    }
}

impl RepoWatch {
    pub(crate) fn new(path: PathBuf) -> Self {
        let flags = Arc::new(AtomicU8::new(0));
        let mut git_dirs = Vec::new();
        if let Ok(repo) = git2::Repository::open(&path) {
            let dir = repo.path().to_path_buf();
            if let Ok(common) = std::fs::read_to_string(dir.join("commondir")) {
                let common = dir.join(common.trim());
                git_dirs.push(common.canonicalize().unwrap_or(common));
            }
            git_dirs.push(dir.canonicalize().unwrap_or(dir));
        }
        let callback_dirs = git_dirs.clone();
        let signal = flags.clone();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                let flags = match event {
                    Ok(event) => classify_event(&event, &callback_dirs),
                    Err(_) => 2,
                };
                signal.fetch_or(flags, Ordering::Relaxed);
            })
            .ok();
        if let Some(watcher) = &mut watcher {
            let _ = watcher.watch(&path, RecursiveMode::Recursive);
            for dir in &git_dirs {
                if !dir.starts_with(&path) {
                    let _ = watcher.watch(dir, RecursiveMode::Recursive);
                }
            }
        }
        let now = Instant::now();
        Self {
            path,
            _watcher: watcher,
            flags,
            pending: 0,
            last_event: now,
            pending_since: now,
            last_fallback: now,
        }
    }
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
    pub(crate) fn poll(&mut self, busy: bool, regained_focus: bool) -> Option<Refresh> {
        let now = Instant::now();
        let mut incoming = self.flags.swap(0, Ordering::Relaxed);
        if regained_focus || now.duration_since(self.last_fallback) > Duration::from_secs(30) {
            incoming |= 2;
            self.last_fallback = now;
        }
        if incoming != 0 {
            if self.pending == 0 {
                self.pending_since = now;
            }
            self.pending |= incoming;
            self.last_event = now;
        }
        if busy || self.pending == 0 {
            return None;
        }
        if now.duration_since(self.last_event) < Duration::from_millis(250)
            && now.duration_since(self.pending_since) < Duration::from_secs(2)
        {
            return None;
        }
        let result = if self.pending & 2 != 0 {
            Refresh::History
        } else {
            Refresh::Worktree
        };
        self.pending = 0;
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classifies_index_refs_and_worktree_changes_and_retains_busy_events() {
        let dirs = vec![PathBuf::from("repo/.git")];
        assert_eq!(
            classify(Path::new("repo/.git/refs/heads/nested/topic"), &dirs),
            2
        );
        assert_eq!(classify(Path::new("repo/.git/index"), &dirs), 1);
        assert_eq!(classify(Path::new("repo/.git/objects/a/b"), &dirs), 0);
        assert_eq!(classify(Path::new("repo/new-file"), &dirs), 1);
        let rescan =
            notify::Event::new(notify::EventKind::Other).set_flag(notify::event::Flag::Rescan);
        assert_eq!(classify_event(&rescan, &dirs), 2);
        let dir = tempfile::tempdir().unwrap();
        let mut watcher = RepoWatch::new(dir.path().into());
        watcher.flags.store(1, Ordering::Relaxed);
        assert!(watcher.poll(true, false).is_none());
        watcher.last_event -= Duration::from_secs(1);
        assert!(matches!(
            watcher.poll(false, false),
            Some(Refresh::Worktree)
        ));
        assert!(watcher.poll(false, true).is_none());
        watcher.last_event -= Duration::from_secs(1);
        assert!(matches!(watcher.poll(false, false), Some(Refresh::History)));
    }
    #[test]
    fn native_notifications_report_working_file_and_nested_ref_changes() {
        let repo = crate::git::test_support::TestRepo::new();
        repo.commit("file.txt", b"base\n");
        let mut watcher = RepoWatch::new(repo.path().to_path_buf());
        assert!(watcher._watcher.is_some());
        let await_refresh = |watcher: &mut RepoWatch, expected: Option<Refresh>| {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(refresh) = watcher.poll(false, false) {
                    if expected.is_none() || expected == Some(refresh) {
                        break refresh;
                    }
                }
                assert!(
                    Instant::now() < deadline,
                    "filesystem event was not delivered"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        repo.write("file.txt", b"external save\n");
        // FSEvents may coalesce setup/metadata events with the file write. Both
        // refresh kinds reload the worktree; precise classification is tested above.
        let expected = if cfg!(target_os = "macos") {
            None
        } else {
            Some(Refresh::Worktree)
        };
        await_refresh(&mut watcher, expected);
        repo.git_ok(&["branch", "nested/topic"]);
        await_refresh(&mut watcher, Some(Refresh::History));
    }
}
