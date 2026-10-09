use super::*;
use crate::git::test_support::TestRepo;
use crate::state::AppState;

#[test]
fn rename_conflicts_report_every_path_and_its_available_sides() {
    let repo = TestRepo::new();
    repo.commit("original", b"base\n");
    repo.git_ok(&["checkout", "-b", "other"]);
    repo.git_ok(&["mv", "original", "theirs"]);
    repo.git_ok(&["commit", "-m", "rename theirs"]);
    repo.git_ok(&["checkout", "main"]);
    repo.git_ok(&["mv", "original", "ours"]);
    repo.git_ok(&["commit", "-m", "rename ours"]);
    assert!(!repo.git(&["merge", "other"]).status.success());
    let conflicts = collect_conflicts(&repo.open());
    assert_eq!(conflicts.len(), 3);
    assert!(conflicts.iter().all(|entry| entry.version.is_some()));
    let ours = conflicts.iter().find(|entry| entry.path == "ours").unwrap();
    assert!(!ours.ours_deleted && ours.theirs_deleted);
    let theirs = conflicts
        .iter()
        .find(|entry| entry.path == "theirs")
        .unwrap();
    assert!(theirs.ours_deleted && !theirs.theirs_deleted);
    assert_eq!(std::fs::read(repo.path().join("ours")).unwrap(), b"base\n");
    assert_eq!(
        std::fs::read(repo.path().join("theirs")).unwrap(),
        b"base\n"
    );
}

#[test]
fn sparse_index_in_linked_worktree_keeps_status_and_other_worktrees_independent() {
    let main = TestRepo::new();
    for name in ["included", "excluded"] {
        std::fs::create_dir(main.path().join(name)).unwrap();
    }
    main.commit("included/file", b"keep\n");
    let original = main.commit("excluded/file", b"hidden\n");
    let root = tempfile::tempdir().unwrap();
    let linked = root.path().join("linked");
    main.git_ok(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
    let git = |args: &[&str]| {
        let mut all = vec!["-C", linked.to_str().unwrap()];
        all.extend_from_slice(args);
        main.git(&all)
    };
    let sparse = git(&[
        "sparse-checkout",
        "set",
        "--cone",
        "--sparse-index",
        "included",
    ]);
    assert!(
        sparse.status.success(),
        "{}",
        String::from_utf8_lossy(&sparse.stderr)
    );
    let repo = git2::Repository::open(&linked).unwrap();
    assert!(capabilities::detect(&linked).is_sparse);
    assert!(capabilities::detect(&linked).has_worktrees);
    assert!(!capabilities::detect(main.path()).is_sparse);
    assert_eq!(collect_status(&repo).unwrap().0, 0);
    std::fs::write(linked.join("included/file"), b"linked edit\n").unwrap();
    let (_, files) = collect_status(&repo).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "included/file");
    assert_eq!(files[0].unstaged_status, Some('M'));
    assert!(git(&["add", "--", "included/file"]).status.success());
    let (_, files) = collect_status(&repo).unwrap();
    assert_eq!(files[0].staged_status, Some('M'));
    let checkpoint = crate::git::recovery::capture(&linked, "Sparse linked snapshot")
        .unwrap()
        .unwrap();
    let snapshot = repo
        .find_commit(git2::Oid::from_str(checkpoint.snapshot.as_ref().unwrap()).unwrap())
        .unwrap();
    assert!(snapshot
        .tree()
        .unwrap()
        .get_path(Path::new("excluded/file"))
        .is_ok());
    assert!(git(&["commit", "-m", "linked change"]).status.success());
    crate::git::lifecycle::set_sparse(&linked, None).unwrap();
    assert_eq!(
        std::fs::read(linked.join("excluded/file")).unwrap(),
        b"hidden\n"
    );
    assert_eq!(main.head(), original);
    assert_eq!(main.index_bytes("included/file"), b"keep\n");
    assert_eq!(
        std::fs::read(main.path().join("excluded/file")).unwrap(),
        b"hidden\n"
    );
}

#[test]
fn sparse_status_omits_absent_directories_and_reports_real_edits() {
    let repo = TestRepo::new();
    for name in ["included", "excluded"] {
        std::fs::create_dir(repo.path().join(name)).unwrap();
    }
    repo.commit("included/file", b"keep\n");
    repo.commit("excluded/file", b"hidden\n");
    crate::git::lifecycle::set_sparse(repo.path(), Some(vec!["included".into()])).unwrap();
    assert_eq!(collect_status(&repo.open()).unwrap().0, 0);
    repo.write("included/file", b"real change\n");
    let (count, files) = collect_status(&repo.open()).unwrap();
    assert_eq!(count, 1);
    assert_eq!(files[0].path, "included/file");
    assert_eq!(files[0].unstaged_status, Some('M'));
    let saved = crate::git::recovery::capture(repo.path(), "Sparse snapshot")
        .unwrap()
        .unwrap();
    let opened = repo.open();
    let snapshot = opened
        .find_commit(git2::Oid::from_str(saved.snapshot.as_ref().unwrap()).unwrap())
        .unwrap();
    assert!(snapshot
        .tree()
        .unwrap()
        .get_path(Path::new("excluded/file"))
        .is_ok());
}

#[test]
fn repository_switch_restores_diff_navigation_and_provider_filters() {
    let a = TestRepo::new();
    a.commit("a.txt", b"base\n");
    a.write("a.txt", b"edited\n");
    let b = TestRepo::new();
    b.commit("b.txt", b"base\n");
    let mut state = AppState::default();
    let id = begin_repo_load(a.path(), &mut state);
    apply_repo_result(
        &mut state,
        a.path().into(),
        id,
        load_repo_inner(a.path()),
        false,
        None,
    );
    state.selected_file = Some("a.txt".into());
    state.diff_context = crate::state::DiffContext::UnstagedFile;
    state.current_diff = Some(std::sync::Arc::new(
        compute_workdir_diff(a.path(), "a.txt", false).unwrap(),
    ));
    state.focused_pane = 4;
    state.pr_filter = crate::providers::PrFilter::Closed;
    state.pr_loaded = true;
    state.issue_filter = crate::providers::IssueFilter::All;
    state.graph_filter.solo_ref = Some("refs/heads/main".into());
    let id = begin_repo_load(b.path(), &mut state);
    apply_repo_result(
        &mut state,
        b.path().into(),
        id,
        load_repo_inner(b.path()),
        false,
        None,
    );
    assert!(state.selected_file.is_none() && state.current_diff.is_none());
    assert_eq!(state.pr_filter, crate::providers::PrFilter::Open);
    let id = begin_repo_load(a.path(), &mut state);
    apply_repo_result(
        &mut state,
        a.path().into(),
        id,
        load_repo_inner(a.path()),
        false,
        None,
    );
    assert_eq!(state.selected_file.as_deref(), Some("a.txt"));
    assert_eq!(state.diff_context, crate::state::DiffContext::UnstagedFile);
    assert_eq!(state.current_diff.as_ref().unwrap().file_path, "a.txt");
    assert_eq!(state.focused_pane, 4);
    assert_eq!(state.pr_filter, crate::providers::PrFilter::Closed);
    assert!(state.pr_loaded);
    assert_eq!(state.issue_filter, crate::providers::IssueFilter::All);
    assert_eq!(
        state.graph_filter.solo_ref.as_deref(),
        Some("refs/heads/main")
    );
}

#[test]
fn restored_worktree_diff_is_reloaded_after_offscreen_edits() {
    let a = TestRepo::new();
    a.commit("a.txt", b"base\n");
    a.write("a.txt", b"first\n");
    let b = TestRepo::new();
    b.commit("b.txt", b"base\n");
    let a_path = a.path().canonicalize().unwrap();
    let b_path = b.path().canonicalize().unwrap();
    let state = crate::state::new_shared_state();
    {
        let mut s = state.lock().unwrap();
        let id = begin_repo_load(a_path.as_path(), &mut s);
        apply_repo_result(
            &mut s,
            a_path.as_path().into(),
            id,
            load_repo_inner(a_path.as_path()),
            false,
            None,
        );
        s.selected_file = Some("a.txt".into());
        s.diff_context = crate::state::DiffContext::UnstagedFile;
        s.current_diff = Some(std::sync::Arc::new(
            compute_workdir_diff(a_path.as_path(), "a.txt", false).unwrap(),
        ));
        let id = begin_repo_load(b_path.as_path(), &mut s);
        apply_repo_result(
            &mut s,
            b_path.as_path().into(),
            id,
            load_repo_inner(b_path.as_path()),
            false,
            None,
        );
    }
    a.write("a.txt", b"changed while away\n");
    load_repo(a_path.as_path().into(), state.clone());
    // Native Windows subprocess startup is slower under the full parallel suite.
    // This is a completion guard, not a performance assertion.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if state
            .lock()
            .unwrap()
            .current_diff
            .as_ref()
            .is_some_and(|diff| {
                diff.hunks
                    .iter()
                    .flat_map(|h| &h.lines)
                    .any(|line| line.content == "changed while away\n")
            })
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "restored diff was not refreshed: {:?}",
            state.lock().unwrap().error
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn older_repository_load_cannot_replace_a_newer_selection() {
    let a = TestRepo::new();
    a.commit("a.txt", b"a\n");
    let b = TestRepo::new();
    b.commit("b.txt", b"b\n");
    let mut state = AppState::default();
    let a_id = begin_repo_load(a.path(), &mut state);
    let a_result = load_repo_inner(a.path());
    let b_id = begin_repo_load(b.path(), &mut state);
    apply_repo_result(
        &mut state,
        b.path().into(),
        b_id,
        load_repo_inner(b.path()),
        false,
        None,
    );
    apply_repo_result(&mut state, a.path().into(), a_id, a_result, false, None);
    assert_eq!(state.repo_path.as_deref(), Some(b.path()));
    assert_eq!(
        state.commit_graph.as_ref().unwrap().nodes[0].oid,
        b.head().to_string()
    );
    assert!(state.repo_load_path.is_none());
    apply_repo_result(
        &mut state,
        a.path().into(),
        a_id,
        Err(git2::Error::from_str("obsolete failure")),
        false,
        None,
    );
    assert!(state.error.is_none());
}

#[test]
fn refresh_cannot_cancel_an_explicit_repository_switch() {
    let mut state = AppState {
        repo_path: Some("repo-a".into()),
        ..Default::default()
    };
    let id = begin_repo_load(Path::new("repo-b"), &mut state);
    assert!(begin_repo_refresh(&mut state).is_none());
    assert_eq!(state.repo_load_id, id);
    assert_eq!(state.repo_load_path.as_deref(), Some(Path::new("repo-b")));
}

#[test]
fn same_repository_refresh_preserves_errors_but_switch_resets_owned_views() {
    let repo = TestRepo::new();
    repo.commit("file.txt", b"content\n");
    let mut state = AppState {
        repo_path: Some(repo.path().into()),
        error: Some("hook rejected".into()),
        selected_file: Some("file.txt".into()),
        pr_loading: true,
        ..Default::default()
    };
    let id = begin_repo_load(repo.path(), &mut state);
    apply_repo_result(
        &mut state,
        repo.path().into(),
        id,
        load_repo_inner(repo.path()),
        false,
        None,
    );
    assert_eq!(state.error.as_deref(), Some("hook rejected"));
    assert_eq!(state.selected_file.as_deref(), Some("file.txt"));
    let other = TestRepo::new();
    other.commit("other.txt", b"other\n");
    let id = begin_repo_load(other.path(), &mut state);
    apply_repo_result(
        &mut state,
        other.path().into(),
        id,
        load_repo_inner(other.path()),
        false,
        None,
    );
    assert!(state.selected_file.is_none());
    assert!(!state.pr_loading);
    assert!(state.error.is_none());
}

#[test]
fn repository_epoch_rejects_results_after_switching_away_and_back() {
    let scope = RepoScope {
        path: "repo-a".into(),
        epoch: 1,
    };
    let mut state = AppState {
        repo_path: Some("repo-a".into()),
        repo_epoch: 1,
        ..Default::default()
    };
    assert!(scope.is_current(&state));
    state.repo_epoch = 3;
    assert!(!scope.is_current(&state));
}

#[test]
fn switching_back_restores_each_repositories_selection_and_filter() {
    let a = TestRepo::new();
    let oid = a.commit("a.txt", b"a\n").to_string();
    let b = TestRepo::new();
    b.commit("b.txt", b"b\n");
    let mut state = AppState::default();
    let load = |state: &mut AppState, path: &Path| {
        let id = begin_repo_load(path, state);
        apply_repo_result(state, path.into(), id, load_repo_inner(path), false, None);
    };
    load(&mut state, a.path());
    state.selected_commit = Some(oid.clone());
    state.search_query = "a search".into();
    state.show_all_branches = false;
    load(&mut state, b.path());
    state.search_query = "b search".into();
    state.show_all_branches = true;
    load(&mut state, a.path());
    assert_eq!(state.selected_commit, Some(oid));
    assert_eq!(state.search_query, "a search");
    assert!(!state.show_all_branches);
    load(&mut state, b.path());
    assert_eq!(state.search_query, "b search");
    assert!(state.show_all_branches);
}
#[test]
fn provider_requests_reject_out_of_order_and_changed_context_results() {
    let mut state = crate::state::AppState {
        repo_path: Some(std::path::PathBuf::from("fixture")),
        repo_epoch: 1,
        ..Default::default()
    };
    let old = ProviderScope::begin(&mut state, ProviderRead::PullRequests).unwrap();
    let current = ProviderScope::begin(&mut state, ProviderRead::PullRequests).unwrap();
    let issues = ProviderScope::begin(&mut state, ProviderRead::Issues).unwrap();
    assert!(!old.is_current(&state));
    assert!(current.is_current(&state));
    assert!(issues.is_current(&state));
    let detail = ProviderScope::begin(&mut state, ProviderRead::Details).unwrap();
    let refresh = ProviderScope::begin(&mut state, ProviderRead::PullRequests).unwrap();
    assert!(!detail.is_current(&state));
    assert!(!state.pr_detail_loading);
    state.pr_loaded = true;
    state.repo_views.insert(
        "other".into(),
        crate::state::RepoView {
            pr_loaded: true,
            ..Default::default()
        },
    );
    state.invalidate_provider_data();
    assert!(!state.pr_loaded && !state.repo_views.values().any(|view| view.pr_loaded));
    assert!(!refresh.is_current(&state));
    assert!(!issues.is_current(&state));
    assert!(!state.pr_loading && !state.issue_loading);
    let fresh = ProviderScope::begin(&mut state, ProviderRead::Releases).unwrap();
    state.repo_epoch += 1;
    assert!(!fresh.is_current(&state));
}

#[test]
fn status_capabilities_invalidate_requests_when_provider_remote_changes() {
    let repo = TestRepo::new();
    repo.commit("file", b"base");
    repo.git_ok(&[
        "remote",
        "add",
        "origin",
        "https://github.com/example/first.git",
    ]);
    let mut state = AppState {
        repo_path: Some(repo.path().into()),
        ..Default::default()
    };
    apply_capabilities(&mut state, capabilities::detect(repo.path()));
    let pending = ProviderScope::begin(&mut state, ProviderRead::PullRequests).unwrap();
    state.pr_loaded = true;
    repo.git_ok(&[
        "remote",
        "set-url",
        "origin",
        "https://github.com/example/second.git",
    ]);
    apply_capabilities(&mut state, capabilities::detect(repo.path()));
    assert!(!state.pr_loaded);
    assert!(!pending.is_current(&state));
}

#[test]
fn commit_comparison_preserves_rename_and_binary_patch_metadata() {
    let repo = TestRepo::new();
    repo.commit("old.txt", b"unchanged content\n");
    let base = repo.commit("image.bin", b"before\0bytes").to_string();
    repo.git_ok(&["mv", "old.txt", "new[1].txt"]);
    let tip = repo.commit("image.bin", b"after\0bytes").to_string();
    let files = compute_compare_files(repo.path(), &base, &tip).unwrap();
    assert!(files
        .iter()
        .any(|f| f.path == "new[1].txt" && f.status == 'R'));
    let rename = compute_compare_diff(repo.path(), &base, &tip, "new[1].txt").unwrap();
    assert!(rename.is_rename);
    assert!(String::from_utf8_lossy(&rename.raw_patch).contains("rename from old.txt"));
    let binary = compute_compare_diff(repo.path(), &base, &tip, "image.bin").unwrap();
    assert!(binary.is_binary);
    assert!(String::from_utf8_lossy(&binary.raw_patch).contains("GIT binary patch"));
}
