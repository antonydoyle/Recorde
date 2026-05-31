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

// ── Patch round-trip tests ────────────────────────────────────────────────────

#[test]
fn test_stage_hunk_round_trip() {
    // Two separate hunks: stage only the first, verify second stays unstaged.
    let repo = TestRepo::new();
    // Create base file with two sections separated by context
    repo.add_and_commit("base", &[("file.txt",
        "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10\n")]);

    // Modify lines far apart so git produces two hunks
    let modified = "CHANGED1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nCHANGED2\nline10\n";
    std::fs::write(repo.path().join("file.txt"), modified).unwrap();

    // Get the unstaged diff
    let git_repo = git2::Repository::open(repo.path()).unwrap();
    let mut diff_opts = git2::DiffOptions::new();
    diff_opts.pathspec("file.txt");
    let diff = git_repo.diff_index_to_workdir(None, Some(&mut diff_opts)).unwrap();

    // Collect hunks using Rc<RefCell<>> to share state across closures
    use std::rc::Rc;
    use std::cell::RefCell;
    let raw: Rc<RefCell<Vec<(String, Vec<recorde::state::DiffLine>)>>> = Rc::new(RefCell::new(Vec::new()));
    let raw_h = Rc::clone(&raw);
    let raw_l = Rc::clone(&raw);

    diff.foreach(
        &mut |_, _| true,
        None,
        Some(&mut |_, h| {
            let header = std::str::from_utf8(h.header()).unwrap_or("").trim_end().to_string();
            raw_h.borrow_mut().push((header, Vec::new()));
            true
        }),
        Some(&mut |_, _, line| {
            if let Some(last) = raw_l.borrow_mut().last_mut() {
                last.1.push(recorde::state::DiffLine {
                    origin: line.origin(),
                    content: std::str::from_utf8(line.content()).unwrap_or("").to_string(),
                    old_lineno: line.old_lineno(),
                    new_lineno: line.new_lineno(),
                });
            }
            true
        }),
    ).unwrap();
    drop(raw_h); drop(raw_l);
    let hunks: Vec<recorde::state::DiffHunk> = Rc::try_unwrap(raw).unwrap().into_inner()
        .into_iter()
        .map(|(header, lines)| recorde::state::DiffHunk { header, lines })
        .collect();

    assert!(hunks.len() >= 2, "expected at least 2 hunks, got {}", hunks.len());

    // Stage just the first hunk
    let patch = recorde::git::patch::hunk_patch("file.txt", &hunks[0]);
    recorde::git::patch::apply_to_index(repo.path(), &patch).expect("stage hunk 0 failed");

    // Check staged diff: should have CHANGED1 staged
    let git_repo2 = git2::Repository::open(repo.path()).unwrap();
    let head_tree = git_repo2.head().unwrap().peel_to_commit().unwrap().tree().unwrap();
    let staged_diff = git_repo2.diff_tree_to_index(Some(&head_tree), None, None).unwrap();
    let stats = staged_diff.stats().unwrap();
    assert!(stats.files_changed() == 1, "staged diff should show 1 file");

    // The staged diff should contain CHANGED1 but not CHANGED2
    let mut staged_content = String::new();
    staged_diff.print(git2::DiffFormat::Patch, |_, _, line| {
        staged_content.push_str(std::str::from_utf8(line.content()).unwrap_or(""));
        true
    }).unwrap();
    assert!(staged_content.contains("CHANGED1"), "CHANGED1 should be staged");
    assert!(!staged_content.contains("CHANGED2"), "CHANGED2 should NOT be staged yet");
}

#[test]
fn test_discard_hunk_round_trip() {
    let repo = TestRepo::new();
    repo.add_and_commit("base", &[("discard.txt", "aaa\nbbb\nccc\n")]);

    // Modify the file
    std::fs::write(repo.path().join("discard.txt"), "MODIFIED\nbbb\nccc\n").unwrap();

    // Get the diff
    let git_repo = git2::Repository::open(repo.path()).unwrap();
    let mut opts = git2::DiffOptions::new();
    opts.pathspec("discard.txt");
    let diff = git_repo.diff_index_to_workdir(None, Some(&mut opts)).unwrap();

    use std::rc::Rc;
    use std::cell::RefCell;
    let raw2: Rc<RefCell<Vec<(String, Vec<recorde::state::DiffLine>)>>> = Rc::new(RefCell::new(Vec::new()));
    let raw2h = Rc::clone(&raw2);
    let raw2l = Rc::clone(&raw2);
    diff.foreach(
        &mut |_, _| true,
        None,
        Some(&mut |_, h| {
            let hdr = std::str::from_utf8(h.header()).unwrap_or("").trim_end().to_string();
            raw2h.borrow_mut().push((hdr, Vec::new()));
            true
        }),
        Some(&mut |_, _, line| {
            if let Some(last) = raw2l.borrow_mut().last_mut() {
                last.1.push(recorde::state::DiffLine {
                    origin: line.origin(),
                    content: std::str::from_utf8(line.content()).unwrap_or("").to_string(),
                    old_lineno: line.old_lineno(),
                    new_lineno: line.new_lineno(),
                });
            }
            true
        }),
    ).unwrap();
    drop(raw2h); drop(raw2l);
    let mut hunks2 = Rc::try_unwrap(raw2).unwrap().into_inner();
    let (hdr, lines) = hunks2.remove(0);
    let hunk = recorde::state::DiffHunk { header: hdr, lines };
    let patch = recorde::git::patch::hunk_patch("discard.txt", &hunk);

    // Discard (reverse apply to workdir)
    recorde::git::patch::discard_from_workdir(repo.path(), &patch).expect("discard failed");

    // File should be restored
    let content = std::fs::read_to_string(repo.path().join("discard.txt")).unwrap();
    assert_eq!(content, "aaa\nbbb\nccc\n", "file should be restored to original");
}
