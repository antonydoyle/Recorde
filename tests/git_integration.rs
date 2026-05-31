use std::path::Path;
use std::sync::{Arc, Mutex};

use recorde::git::capabilities;
use recorde::git::runner::GitRunner;
use recorde::git::job::JobProgress;
use recorde::state::{AppState, SharedState};

// ── TestRepo helper ───────────────────────────────────────────────────────────

struct TestRepo {
    dir: tempfile::TempDir,
}

impl TestRepo {
    fn new() -> Self {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let repo = git2::Repository::init(dir.path()).expect("git init");

        // Configure a minimal identity so commits work
        let mut config = repo.config().expect("config");
        config.set_str("user.name", "Test User").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        drop(config);

        Self { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn add_and_commit(&self, message: &str, files: &[(&str, &str)]) -> git2::Oid {
        let repo = git2::Repository::open(self.path()).expect("open");
        let mut index = repo.index().expect("index");

        for (name, content) in files {
            let file_path = self.path().join(name);
            std::fs::write(&file_path, content).expect("write file");
            index.add_path(Path::new(name)).expect("index add");
        }
        index.write().expect("index write");

        let tree_id = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_id).expect("tree");
        let sig = repo.signature().expect("sig");

        let parent_commit = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent_commit.as_ref().map(|c| vec![c]).unwrap_or_default();

        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
            .expect("commit")
    }

    #[allow(dead_code)]
    fn shared_state(&self) -> SharedState {
        let state = AppState {
            repo_path: Some(self.path().to_owned()),
            show_all_branches: true,
            ..Default::default()
        };
        Arc::new(Mutex::new(state))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[test]
fn test_capability_detection_clean_repo() {
    let repo = TestRepo::new();
    repo.add_and_commit("initial commit", &[("README.md", "hello")]);

    let caps = capabilities::detect(repo.path());

    assert!(!caps.is_bare, "normal repo should not be bare");
    assert!(!caps.is_detached_head, "should be on a branch");
    assert!(!caps.merge_in_progress);
    assert!(!caps.rebase_in_progress);
    assert!(!caps.cherry_pick_in_progress);
    assert!(!caps.revert_in_progress);
    assert!(!caps.has_submodules);
}

#[test]
fn test_capability_detection_merge_in_progress() {
    let repo = TestRepo::new();
    repo.add_and_commit("initial", &[("file.txt", "base")]);

    // Simulate a merge-in-progress by writing a MERGE_HEAD file
    let git_dir = git2::Repository::open(repo.path())
        .expect("open")
        .path()
        .to_owned();
    std::fs::write(git_dir.join("MERGE_HEAD"), "0000000000000000000000000000000000000000\n")
        .expect("write MERGE_HEAD");

    let caps = capabilities::detect(repo.path());
    assert!(caps.merge_in_progress, "should detect MERGE_HEAD");
}

#[test]
fn test_capability_detection_cherry_pick_in_progress() {
    let repo = TestRepo::new();
    repo.add_and_commit("initial", &[("file.txt", "base")]);

    let git_dir = git2::Repository::open(repo.path())
        .expect("open")
        .path()
        .to_owned();
    std::fs::write(
        git_dir.join("CHERRY_PICK_HEAD"),
        "0000000000000000000000000000000000000000\n",
    )
    .expect("write CHERRY_PICK_HEAD");

    let caps = capabilities::detect(repo.path());
    assert!(caps.cherry_pick_in_progress);
}

#[test]
fn test_runner_submit_and_done_for_no_remotes() {
    // A repo with no remotes: fetch_all should succeed immediately (no-op).
    let repo = TestRepo::new();
    repo.add_and_commit("initial", &[("file.txt", "hello")]);

    let mut runner = GitRunner::new();
    let path = repo.path().to_owned();

    // Submit a fetch — should complete quickly since there are no remotes
    runner.submit(recorde::git::job::JobKind::FetchAll, path);

    // Poll for up to 2 seconds for Done or Failed
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut got_done = false;
    let mut got_failed = false;

    while std::time::Instant::now() < deadline {
        for event in runner.drain_progress() {
            match event {
                JobProgress::Done { .. } => got_done = true,
                JobProgress::Failed { .. } => got_failed = true,
                _ => {}
            }
        }
        if got_done || got_failed {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // No remotes → fetch_all should succeed (Done), not fail
    assert!(got_done, "expected Done for fetch on repo with no remotes");
    assert!(!got_failed, "expected no failure");
}

#[test]
fn test_runner_cancel_token() {
    use std::sync::atomic::Ordering;
    let repo = TestRepo::new();
    repo.add_and_commit("initial", &[("file.txt", "hello")]);

    let mut runner = GitRunner::new();
    let handle = runner.submit(
        recorde::git::job::JobKind::FetchAll,
        repo.path().to_owned(),
    );

    // Immediately signal cancel
    handle.cancel.store(true, Ordering::Relaxed);

    // Should still get a terminal event (Done or Failed/Cancelled) within 2s
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut got_terminal = false;

    while std::time::Instant::now() < deadline {
        for event in runner.drain_progress() {
            match event {
                JobProgress::Done { .. } | JobProgress::Failed { .. } => got_terminal = true,
                _ => {}
            }
        }
        if got_terminal { break; }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    assert!(got_terminal, "should receive terminal event even after cancel");
}
