use super::*;
use crate::git::{
    merge::{do_merge, MergeMode},
    rebase,
    test_support::{snapshot, TestRepo},
};

#[test]
fn t5_directory_rename_and_root_promotion_preserve_ignored_files() {
    for destination in ["", "new/"] {
        for rebase_operation in [false, true] {
            let repo = TestRepo::new();
            repo.commit(".gitignore", b"/cache.env\n/new/cache.env\n");
            std::fs::create_dir(repo.path().join("old")).unwrap();
            let base = repo.commit("old/seed.txt", b"seed\n");
            repo.git_ok(&["checkout", "-b", "incoming"]);
            if !destination.is_empty() {
                std::fs::create_dir(repo.path().join("new")).unwrap();
            }
            repo.git_ok(&["mv", "old/seed.txt", &format!("{destination}seed.txt")]);
            repo.git_ok(&["commit", "-m", "move old directory"]);
            repo.git_ok(&["checkout", "main"]);
            assert_eq!(repo.head(), base);
            std::fs::create_dir_all(repo.path().join("old")).unwrap();
            repo.commit("old/cache.env", b"replayed content\n");
            if !destination.is_empty() {
                std::fs::create_dir_all(repo.path().join("new")).unwrap();
            }
            repo.write(
                &format!("{destination}cache.env"),
                b"local ignored bytes\x00\xff",
            );
            let before = snapshot(repo.path());
            let result = if rebase_operation {
                rebase::do_rebase_onto(repo.path(), "incoming")
            } else {
                do_merge(repo.path(), "incoming", MergeMode::NoFastForward, None)
            };
            assert!(result.unwrap_err().contains("cannot prove"));
            assert_eq!(
                snapshot(repo.path()),
                before,
                "destination={destination} rebase={rebase_operation}"
            );
        }
    }
}

#[test]
fn t5_rebase_transient_add_delete_is_checked_even_when_final_tree_has_no_collision() {
    let repo = TestRepo::new();
    let base = repo.commit(".gitignore", b"/cache.env\n");
    repo.git_ok(&["checkout", "-b", "incoming"]);
    repo.commit("cache.env", b"temporary history\n");
    repo.git_ok(&["rm", "cache.env"]);
    repo.git_ok(&["commit", "-m", "remove transient file"]);
    repo.git_ok(&["checkout", "main"]);
    assert_eq!(repo.head(), base);
    repo.commit("local.txt", b"local\n");
    repo.write("cache.env", b"ignored local bytes\x00\xff");
    let before = snapshot(repo.path());
    assert!(rebase::do_rebase_onto(repo.path(), "incoming")
        .unwrap_err()
        .contains("cannot prove"));
    assert_eq!(snapshot(repo.path()), before);
}

#[test]
fn t5_effective_commit_and_nested_tree_replacements_contribute_names() {
    for tree_replacement in [false, true] {
        let repo = TestRepo::new();
        let base = repo.commit(".gitignore", b"/cache.env\n");
        repo.git_ok(&["checkout", "-b", "incoming"]);
        std::fs::create_dir(repo.path().join("nested")).unwrap();
        let original = repo.commit("nested/file.txt", b"original\n");
        repo.git_ok(&["checkout", "--detach", &base.to_string()]);
        std::fs::create_dir_all(repo.path().join("nested")).unwrap();
        repo.write("nested/file.txt", b"original\n");
        repo.git_ok(&["add", "nested/file.txt"]);
        let replacement = repo.commit("nested/cache.env", b"replacement collision\n");
        repo.git_ok(&["checkout", "incoming"]);
        let git = repo.open();
        let objects = if tree_replacement {
            (
                git.find_commit(original)
                    .unwrap()
                    .tree()
                    .unwrap()
                    .get_path(Path::new("nested"))
                    .unwrap()
                    .id(),
                git.find_commit(replacement)
                    .unwrap()
                    .tree()
                    .unwrap()
                    .get_path(Path::new("nested"))
                    .unwrap()
                    .id(),
            )
        } else {
            (original, replacement)
        };
        repo.git_ok(&["replace", &objects.0.to_string(), &objects.1.to_string()]);
        repo.git_ok(&["checkout", "main"]);
        assert_eq!(repo.head(), base);
        repo.write("cache.env", b"root local ignored bytes\x00\xff");
        let before = snapshot(repo.path());
        let error = Guard::prepare(repo.path(), &["incoming"]).err().unwrap();
        assert!(error.contains("cannot prove"), "{error}");
        assert_eq!(snapshot(repo.path()), before);
    }
}

fn paused_with_external_history() -> (TestRepo, git2::Oid) {
    let repo = TestRepo::new();
    repo.commit(".gitignore", b"/cache.env\n/target/\n");
    repo.commit("conflict.txt", b"base\n");
    repo.git_ok(&["checkout", "-b", "external"]);
    let external = repo.commit("cache.env", b"external incoming\n");
    repo.git_ok(&["checkout", "main"]);
    repo.git_ok(&["checkout", "-b", "incoming"]);
    repo.commit("conflict.txt", b"incoming\n");
    repo.git_ok(&["checkout", "main"]);
    repo.commit("conflict.txt", b"local\n");
    assert!(rebase::do_rebase_onto(repo.path(), "incoming").is_err());
    assert!(repo.open().index().unwrap().has_conflicts());
    (repo, external)
}

#[test]
fn t5_edited_fixup_reset_and_merge_operands_include_external_history() {
    for command in [
        "fixup -C EXTERNAL",
        "fixup -c EXTERNAL",
        "reset external",
        "merge -C EXTERNAL incoming external",
        "reset outside#label",
        "reset external\nlabel external",
    ] {
        let (repo, external) = paused_with_external_history();
        if command.contains("outside#label") {
            repo.git_ok(&[
                "update-ref",
                "refs/rewritten/outside#label",
                &external.to_string(),
            ]);
        }
        repo.write("cache.env", b"local ignored bytes\x00\xff");
        let todo = repo.open().path().join("rebase-merge/git-rebase-todo");
        std::fs::write(
            todo,
            format!("{}\n", command.replace("EXTERNAL", &external.to_string())),
        )
        .unwrap();
        let before = snapshot(repo.path());
        let error = SequenceGuard::prepare(repo.path()).err().unwrap();
        assert!(error.contains("cannot prove"), "{command}: {error}");
        assert_eq!(snapshot(repo.path()), before);
    }
}

#[test]
fn t5_sequencer_metadata_and_rewritten_refs_are_rechecked_before_native_work() {
    for change_label in [false, true] {
        let (repo, external) = paused_with_external_history();
        std::fs::create_dir(repo.path().join("target")).unwrap();
        repo.write("target/cache.bin", b"unrelated ignored cache\x00\xff");
        let guard = SequenceGuard::prepare(repo.path()).unwrap();
        if change_label {
            repo.git_ok(&[
                "update-ref",
                "refs/rewritten/new-label",
                &external.to_string(),
            ]);
        } else {
            std::fs::write(
                repo.open().path().join("rebase-merge/git-rebase-todo"),
                b"reset external\n",
            )
            .unwrap();
        }
        let before = snapshot(repo.path());
        assert!(guard
            .validate()
            .unwrap_err()
            .contains("Sequencer state changed"));
        assert_eq!(snapshot(repo.path()), before);
    }
}

#[test]
fn t5_custom_strategy_and_legacy_graft_history_refuse_without_touching_ignored_caches() {
    for kind in 0..3 {
        let repo = TestRepo::new();
        let head = repo.commit(".gitignore", b"/target/\n");
        std::fs::create_dir(repo.path().join("target")).unwrap();
        repo.write("target/cache.bin", b"ignored\x00\xff");
        Guard::prepare(repo.path(), &["HEAD"]).unwrap();
        match kind {
            0 => repo.git_ok(&["config", "pull.twohead", "ours"]),
            1 => repo.git_ok(&["config", "pull.octopus", "custom-executable"]),
            _ => std::fs::write(
                repo.open().commondir().join("info/grafts"),
                format!("{head}\n"),
            )
            .unwrap(),
        }
        let before = snapshot(repo.path());
        assert!(Guard::prepare(repo.path(), &["HEAD"]).is_err());
        assert_eq!(snapshot(repo.path()), before);
    }
}

#[cfg(not(windows))]
#[test]
fn t5_casefold_expansions_and_insensitive_linux_mount_names_are_conservative() {
    assert!(same_name("CACHE.ENV", "cache.env").unwrap());
    assert!(same_name("ff", "\u{fb00}").unwrap());
    assert!(same_name("ss", "ß").unwrap());
    assert!(!same_name("target", "résumé").unwrap());
}

#[test]
fn t5_saved_rebase_strategy_and_options_are_witnessed_and_refused_when_unsupported() {
    for file in ["strategy", "strategy_opts"] {
        let (repo, _) = paused_with_external_history();
        std::fs::create_dir(repo.path().join("target")).unwrap();
        repo.write("target/cache.bin", b"unrelated ignored cache\x00\xff");
        let guard = SequenceGuard::prepare(repo.path()).unwrap();
        let value = if file == "strategy" {
            "ours\n"
        } else {
            "--prefix=elsewhere\n"
        };
        std::fs::write(repo.open().path().join("rebase-merge").join(file), value).unwrap();
        let before = snapshot(repo.path());
        assert!(guard.validate().is_err());
        let error = SequenceGuard::prepare(repo.path()).err().unwrap();
        assert!(error.contains("strategy"), "{error}");
        assert_eq!(snapshot(repo.path()), before);
    }
}

#[test]
fn t5_guard_rechecks_targets_configuration_and_new_ignored_entries() {
    for kind in 0..4 {
        let repo = TestRepo::new();
        let base = repo.commit(".gitignore", b"/cache.env\n/target/\n");
        repo.git_ok(&["checkout", "-b", "incoming"]);
        repo.commit("cache.env", b"incoming\n");
        repo.git_ok(&["checkout", "main"]);
        let guard = Guard::prepare(repo.path(), &["incoming"]).unwrap();
        match kind {
            0 => repo.git_ok(&["branch", "-f", "incoming", &base.to_string()]),
            1 => repo.git_ok(&["config", "merge.renames", "false"]),
            2 => repo.git_ok(&["checkout", "-b", "same-tip"]),
            _ => repo.write("cache.env", b"new ignored collision\x00\xff"),
        }
        let before = snapshot(repo.path());
        assert!(guard.validate().is_err());
        assert_eq!(snapshot(repo.path()), before);
    }
}

#[test]
fn t5_namespace_budget_and_synthetic_conflict_names_fail_closed() {
    let repo = TestRepo::new();
    repo.commit("file", b"base\n");
    let state = witness(&repo.open(), &[]).unwrap();
    let before = snapshot(repo.path());
    let mut budget = Budget::new();
    budget.objects = MAX_OBJECTS;
    assert!(names(&repo.open(), &state, &mut budget)
        .unwrap_err()
        .contains("budget"));
    assert_eq!(snapshot(repo.path()), before);
    let mut vocabulary = BTreeSet::new();
    vocabulary.insert("file~existing".to_owned());
    repo.write("file~existing~HEAD_1", b"ignored collision");
    assert!(check_names(
        repo.path(),
        &["file~existing~HEAD_1".into()],
        &vocabulary,
        &Budget::new()
    )
    .is_err());
}

#[test]
fn t5_paused_rebase_continue_skip_abort_recheck_new_ignored_files() {
    for action in ["continue", "skip", "abort"] {
        let repo = TestRepo::new();
        repo.commit(".gitignore", b"/cache.env\n");
        repo.commit("cache.env", b"historical name\n");
        repo.git_ok(&["rm", "cache.env"]);
        repo.git_ok(&["commit", "-m", "remove"]);
        repo.commit("conflict.txt", b"base\n");
        repo.git_ok(&["checkout", "-b", "incoming"]);
        repo.commit("conflict.txt", b"incoming\n");
        repo.git_ok(&["checkout", "main"]);
        repo.commit("conflict.txt", b"local\n");
        assert!(rebase::do_rebase_onto(repo.path(), "incoming").is_err());
        assert!(repo.open().index().unwrap().has_conflicts());
        repo.write("cache.env", b"new ignored bytes during pause\x00\xff");
        if action == "continue" {
            repo.write("conflict.txt", b"resolved\n");
            repo.git_ok(&["add", "conflict.txt"]);
        }
        let before = snapshot(repo.path());
        let state = crate::state::new_shared_state();
        state.lock().unwrap().repo_path = Some(repo.path().canonicalize().unwrap());
        match action {
            "continue" => rebase::continue_rebase(state.clone()),
            "skip" => rebase::skip_rebase(state.clone()),
            _ => rebase::abort_rebase(state.clone()),
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let s = state.lock().unwrap();
            if s.local_jobs.is_empty() && s.repo_load_path.is_none() && s.commit_graph.is_some() {
                break;
            }
            drop(s);
            assert!(
                Instant::now() < deadline,
                "sequence callback did not finish"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(state
            .lock()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("cannot prove"));
        assert_eq!(snapshot(repo.path()), before, "action={action}");
    }
}

#[cfg(windows)]
#[test]
fn t5_windows_unicode_case_and_existing_short_aliases_are_checked() {
    let repo = TestRepo::new();
    repo.commit(".gitignore", "/dépôt.env\n".as_bytes());
    repo.git_ok(&["checkout", "-b", "incoming"]);
    repo.commit("DÉPÔT.ENV", b"incoming\n");
    repo.git_ok(&["checkout", "main"]);
    repo.write("dépôt.env", b"local ignored\x00\xff");
    let before = snapshot(repo.path());
    assert!(Guard::prepare(repo.path(), &["incoming"]).is_err());
    let mut vocabulary = BTreeSet::new();
    let aliases = aliases(repo.path(), "dépôt.env").unwrap();
    let short = aliases.last().unwrap();
    if same_name(short, "dépôt.env").unwrap() {
        eprintln!("No distinct 8.3 alias on this filesystem; short-only equality exercised without claiming distinct-alias acceptance");
    }
    vocabulary.insert(short.clone());
    assert!(check_names(
        repo.path(),
        &["dépôt.env".into()],
        &vocabulary,
        &Budget::new()
    )
    .is_err());
    assert_eq!(snapshot(repo.path()), before);
}
