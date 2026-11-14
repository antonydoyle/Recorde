use super::{
    merge::{do_merge, MergeMode},
    rebase, recovery,
    test_support::{snapshot, TestRepo},
};

fn step(repo: &TestRepo, redo: bool) {
    // Reopen the persisted journal, as a fresh application process would.
    let _reopened = repo.open();
    let (id, _) = recovery::undo_history(repo.path())
        .next(redo)
        .expect("Expected a supported recovery step");
    recovery::restore_step(repo.path(), &id, redo).unwrap();
}

fn topic(diverged: bool) -> TestRepo {
    let repo = TestRepo::new();
    repo.commit(".gitignore", b"/ignored.env\n/target/\n/node_modules/\n");
    repo.commit("base.txt", b"base\n");
    repo.git_ok(&["checkout", "-b", "topic"]);
    repo.commit("topic.txt", b"topic\n");
    repo.git_ok(&["checkout", "main"]);
    if diverged {
        repo.commit("main.txt", b"main\n");
    }
    repo.git_ok(&["branch", "unrelated"]);
    repo.write("untracked.bin", b"untracked\r\n\xff\x00");
    repo.write("ignored.env", b"ignored\r\n\xff\x00");
    for cache in ["target", "node_modules"] {
        std::fs::create_dir(repo.path().join(cache)).unwrap();
        repo.write(
            &format!("{cache}/cache.bin"),
            b"ignored build cache\x00\xff",
        );
    }
    repo
}

#[test]
fn t5_all_merge_modes_roundtrip_exact_refs_index_tree_files_and_restart() {
    for (mode, diverged) in [
        (MergeMode::Default, false),
        (MergeMode::Default, true),
        (MergeMode::NoFastForward, false),
        (MergeMode::FastForwardOnly, false),
        (MergeMode::Squash, true),
    ] {
        let repo = topic(diverged);
        let before = snapshot(repo.path());
        do_merge(repo.path(), "topic", mode, Some("Reviewed merge matrix")).unwrap();
        let after = snapshot(repo.path());
        assert_ne!(before, after);
        if mode == MergeMode::NoFastForward {
            let entries = recovery::list(repo.path());
            recovery::set_pinned(repo.path(), &entries[0].id, true).unwrap();
            repo.git_ok(&["gc", "--prune=now"]);
            let untracked =
                git2::Oid::hash_object(git2::ObjectType::Blob, b"untracked\r\n\xff\x00").unwrap();
            assert_eq!(
                repo.open().find_blob(untracked).unwrap().content(),
                b"untracked\r\n\xff\x00"
            );
        }
        step(&repo, false);
        assert_eq!(
            snapshot(repo.path()),
            before,
            "Undo {mode:?} diverged={diverged}"
        );
        step(&repo, true);
        assert_eq!(
            snapshot(repo.path()),
            after,
            "Redo {mode:?} diverged={diverged}"
        );
    }
}

#[test]
fn t5_branch_and_interactive_rebase_roundtrip_exact_state() {
    for interactive in [false, true] {
        let repo = topic(true);
        let before = snapshot(repo.path());
        if interactive {
            let git = repo.open();
            let head = git.head().unwrap().peel_to_commit().unwrap();
            let upstream = head.parent_id(0).unwrap();
            let entry = crate::state::RebaseEntry {
                oid: head.id().to_string(),
                summary: "fixture".into(),
                action: crate::state::RebaseAction::Reword,
                message: "Reviewed replacement message".into(),
            };
            rebase::PlanTarget::capture(repo.path(), 0)
                .unwrap()
                .execute_guarded(&upstream.to_string(), &[entry], || Ok(()))
                .unwrap();
        } else {
            rebase::do_rebase_onto(repo.path(), "topic").unwrap();
        }
        let after = snapshot(repo.path());
        assert_ne!(before, after);
        step(&repo, false);
        assert_eq!(
            snapshot(repo.path()),
            before,
            "rebase interactive={interactive} undo"
        );
        step(&repo, true);
        assert_eq!(
            snapshot(repo.path()),
            after,
            "rebase interactive={interactive} redo"
        );
    }
}

#[test]
fn t5_dirty_rebase_refuses_index_or_worktree_changes_without_mutation() {
    for staged in [false, true] {
        let repo = topic(true);
        repo.write("base.txt", b"later draft\n");
        if staged {
            repo.git_ok(&["add", "base.txt"]);
        }
        let before = snapshot(repo.path());
        assert!(rebase::do_rebase_onto(repo.path(), "topic").is_err());
        assert_eq!(snapshot(repo.path()), before);
        assert!(recovery::undo_history(repo.path()).next(false).is_none());
    }
}

#[test]
fn t5_stale_merge_undo_refuses_symbolic_head_tip_index_tracked_and_untracked_edits() {
    for kind in 0..5 {
        let repo = topic(false);
        do_merge(repo.path(), "topic", MergeMode::Default, None).unwrap();
        let (id, _) = recovery::undo_history(repo.path()).next(false).unwrap();
        match kind {
            0 => repo.git_ok(&["checkout", "-b", "same-tip"]),
            1 => {
                repo.commit("later.txt", b"later commit\n");
            }
            2 => {
                repo.write("base.txt", b"later staged\n");
                repo.git_ok(&["add", "base.txt"]);
            }
            3 => repo.write("base.txt", b"later unstaged\n"),
            _ => repo.write("untracked.bin", b"later untracked\r\n\xff\x00"),
        }
        let before = snapshot(repo.path());
        assert!(
            recovery::restore_step(repo.path(), &id, false).is_err(),
            "stale kind{kind}"
        );
        assert_eq!(snapshot(repo.path()), before, "stale kind{kind}");
    }
}

fn incoming_ignored_collision() -> TestRepo {
    let repo = TestRepo::new();
    let base = repo.commit(".gitignore", b"/ignored.env\n");
    repo.commit("ignored.env", b"incoming tracked bytes\n");
    repo.git_ok(&["branch", "incoming"]);
    repo.git_ok(&["reset", "--hard", &base.to_string()]);
    repo.write("ignored.env", b"local ignored bytes\r\n\xff\x00");
    repo
}

#[test]
fn t5_merge_modes_refuse_ignored_incoming_collision_without_changes() {
    for mode in MergeMode::ALL {
        let repo = incoming_ignored_collision();
        let before = snapshot(repo.path());
        let result = do_merge(repo.path(), "incoming", mode, None);
        assert!(
            result.is_err(),
            "Ignored incoming collision accepted by {mode:?}; actual bytes: {:?}",
            std::fs::read(repo.path().join("ignored.env")).unwrap()
        );
        assert_eq!(snapshot(repo.path()), before, "merge mode {mode:?}");
    }
}

#[test]
fn t5_git_flow_finish_refuses_ignored_merge_collision_and_keeps_all_branches() {
    let repo = incoming_ignored_collision();
    super::gitflow::init(repo.path(), &super::gitflow::FlowConfig::default()).unwrap();
    repo.git_ok(&["branch", "feature/work", "incoming"]);
    repo.git_ok(&["checkout", "develop"]);
    let before = snapshot(repo.path());
    let result = super::gitflow::finish(repo.path(), super::gitflow::Kind::Feature, "work");
    assert!(
        result.is_err(),
        "Ignored incoming collision accepted by Git Flow merge; actual bytes: {:?}",
        std::fs::read(repo.path().join("ignored.env")).unwrap()
    );
    assert_eq!(snapshot(repo.path()), before);
}

#[test]
fn t5_rebase_refuses_ignored_incoming_collision_without_changes() {
    let repo = incoming_ignored_collision();
    repo.commit("local.txt", b"local branch work\n");
    let before = snapshot(repo.path());
    let result = rebase::do_rebase_onto(repo.path(), "incoming");
    assert!(
        result.is_err(),
        "Ignored incoming collision accepted by branch rebase; actual bytes: {:?}",
        std::fs::read(repo.path().join("ignored.env")).unwrap()
    );
    assert_eq!(snapshot(repo.path()), before);
}

fn wait_jobs(state: &crate::state::SharedState) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let state = state.lock().unwrap();
        let finished = state.local_jobs.is_empty()
            && state.repo_load_path.is_none()
            && state.commit_graph.is_some();
        drop(state);
        if finished || std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let state = state.lock().unwrap();
    assert!(
        state.local_jobs.is_empty()
            && state.repo_load_path.is_none()
            && state.commit_graph.is_some(),
        "Production sequence operation and refresh did not finish"
    );
    assert!(state.error.is_none(), "Sequence failed: {:?}", state.error);
}

#[test]
fn t5_ordinary_merge_and_rebase_conflict_continue_abort_survive_restart() {
    for (rebase_operation, abort) in [(false, false), (false, true), (true, false), (true, true)] {
        let repo = TestRepo::new();
        repo.commit(".gitignore", b"/ignored.env\n");
        repo.commit("conflict.txt", b"base\n");
        repo.git_ok(&["checkout", "-b", "topic"]);
        repo.commit("conflict.txt", b"topic\n");
        repo.git_ok(&["checkout", "main"]);
        repo.commit("conflict.txt", b"main\n");
        repo.git_ok(&["branch", "unrelated"]);
        repo.write("untracked.bin", b"keep untracked\r\n\xff\x00");
        repo.write("ignored.env", b"keep ignored\r\n\xff\x00");
        let before = snapshot(repo.path());
        let result = if rebase_operation {
            rebase::do_rebase_onto(repo.path(), "topic")
        } else {
            do_merge(repo.path(), "topic", MergeMode::Default, None)
        };
        assert!(result.is_err());
        assert!(repo.open().index().unwrap().has_conflicts());
        let records = recovery::list(repo.path());
        assert_eq!(records.len(), 1);
        assert!(recovery::undo_history(repo.path()).next(false).is_none());
        let state = crate::state::new_shared_state();
        state.lock().unwrap().repo_path = Some(repo.path().canonicalize().unwrap());
        if abort {
            if rebase_operation {
                rebase::abort_rebase(state.clone());
            } else {
                super::merge::abort_merge(state.clone());
            }
            wait_jobs(&state);
            assert_eq!(
                snapshot(repo.path()),
                before,
                "abort rebase={rebase_operation}"
            );
            assert!(recovery::undo_history(repo.path()).next(false).is_none());
        } else {
            repo.write("conflict.txt", b"resolved\n");
            repo.git_ok(&["add", "conflict.txt"]);
            if rebase_operation {
                rebase::continue_rebase(state.clone());
            } else {
                super::merge::continue_merge(state.clone());
            }
            wait_jobs(&state);
            assert_eq!(repo.open().state(), git2::RepositoryState::Clean);
            assert_eq!(recovery::list(repo.path()).len(), 1);
            let after = snapshot(repo.path());
            step(&repo, false);
            assert_eq!(
                snapshot(repo.path()),
                before,
                "conflict undo rebase={rebase_operation}"
            );
            step(&repo, true);
            assert_eq!(
                snapshot(repo.path()),
                after,
                "conflict redo rebase={rebase_operation}"
            );
        }
    }
}
