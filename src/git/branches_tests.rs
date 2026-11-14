use super::*;
use crate::git::test_support::TestRepo;

#[test]
fn t15_ignored_collision_refuses_local_new_remote_and_detached_checkout_without_changes() {
    use crate::git::test_support::{ignored_collision, snapshot};
    for kind in 0..5 {
        let (repo, target) = ignored_collision();
        if kind == 2 || kind == 3 {
            repo.open()
                .remote("origin", "https://example.invalid/fixture")
                .unwrap();
            repo.open()
                .reference("refs/remotes/origin/topic", target, false, "fixture")
                .unwrap();
            if kind == 3 {
                let git = repo.open();
                git.branch("topic", &git.find_commit(target).unwrap(), false)
                    .unwrap();
                repo.git_ok(&["branch", "--set-upstream-to=origin/topic", "topic"]);
            }
        }
        let before = snapshot(repo.path());
        let result = match kind {
            0 => do_checkout(repo.path(), "destination").map_err(|e| e.to_string()),
            1 => do_create(repo.path(), "new-branch", Some(&target.to_string()), true)
                .map_err(|e| e.to_string()),
            2 | 3 => do_checkout_remote(repo.path(), "origin/topic").map(|_| ()),
            _ => do_checkout_detached(repo.path(), &target.to_string()).map_err(|e| e.to_string()),
        };
        assert!(
            result.is_err(),
            "Ignored collision was overwritten by checkout kind {kind}"
        );
        assert_eq!(snapshot(repo.path()), before, "checkout kind {kind}");
        std::fs::remove_file(repo.path().join("ignored.env")).unwrap();
        match kind {
            0 => do_checkout(repo.path(), "destination").map_err(|e| e.to_string()),
            1 => do_create(repo.path(), "new-branch", Some(&target.to_string()), true)
                .map_err(|e| e.to_string()),
            2 | 3 => do_checkout_remote(repo.path(), "origin/topic").map(|_| ()),
            _ => do_checkout_detached(repo.path(), &target.to_string()).map_err(|e| e.to_string()),
        }
        .unwrap();
        assert_eq!(repo.head(), target);
        assert_eq!(
            std::fs::read(repo.path().join("ignored.env")).unwrap(),
            b"destination tracked bytes\n"
        );
    }
}

#[test]
fn deleted_branch_survives_reload_and_gc_without_overwriting_a_new_branch() {
    let repo = TestRepo::new();
    let tip = repo.commit("file", b"base\n");
    do_create(repo.path(), "topic/nested", None, false).unwrap();
    do_delete(repo.path(), "topic/nested").unwrap();
    repo.git_ok(&["gc", "--prune=now"]);
    let saved = load_deleted(repo.path());
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name, "topic/nested");
    assert_eq!(saved[0].oid, tip.to_string());
    do_create(repo.path(), &saved[0].name, Some(&saved[0].oid), false).unwrap();
    assert!(load_deleted(repo.path()).is_empty());
    assert!(do_create(repo.path(), &saved[0].name, Some(&saved[0].oid), false).is_err());
    assert_eq!(repo.head(), tip);
}

#[test]
fn detached_checkout_preserves_staged_and_unstaged_changes_on_failure() {
    for staged in [false, true] {
        let repo = TestRepo::new();
        let old = repo.commit("file.txt", b"old\n");
        let head = repo.commit("file.txt", b"current\n");
        repo.write("file.txt", b"local work\n");
        if staged {
            repo.git_ok(&["add", "--", "file.txt"]);
        }
        let index = repo.index_bytes("file.txt");
        assert!(do_checkout_detached(repo.path(), &old.to_string()).is_err());
        assert_eq!(repo.head(), head);
        assert!(!repo.open().head_detached().unwrap());
        assert_eq!(repo.index_bytes("file.txt"), index);
        assert_eq!(
            std::fs::read(repo.path().join("file.txt")).unwrap(),
            b"local work\n"
        );
    }
}

#[test]
fn detached_checkout_preserves_untracked_collision_and_supports_clean_checkout() {
    let repo = TestRepo::new();
    let old = repo.commit("file.txt", b"tracked\n");
    repo.git_ok(&["rm", "--", "file.txt"]);
    repo.git_ok(&["commit", "-m", "remove file"]);
    let head = repo.head();
    repo.write("file.txt", b"untracked work\n");
    assert!(do_checkout_detached(repo.path(), &old.to_string()).is_err());
    assert_eq!(repo.head(), head);
    assert_eq!(
        std::fs::read(repo.path().join("file.txt")).unwrap(),
        b"untracked work\n"
    );
    // Preserve the untracked file elsewhere before retrying the safe operation.
    std::fs::rename(repo.path().join("file.txt"), repo.path().join("saved.txt")).unwrap();
    do_checkout_detached(repo.path(), &old.to_string()).unwrap();
    assert!(repo.open().head_detached().unwrap());
    assert_eq!(repo.head(), old);
    assert_eq!(repo.index_bytes("file.txt"), b"tracked\n");
}

fn conflicting_cherry_pick() -> (TestRepo, git2::Oid, git2::Oid) {
    let repo = TestRepo::new();
    repo.commit("file.txt", b"base\n");
    repo.git_ok(&["checkout", "-b", "feature"]);
    let feature = repo.commit("file.txt", b"feature\n");
    repo.git_ok(&["checkout", "main"]);
    let main = repo.commit("file.txt", b"main\n");
    assert!(do_cherry_pick(repo.path(), &feature.to_string()).is_err());
    assert!(repo.open().path().join("CHERRY_PICK_HEAD").exists());
    (repo, feature, main)
}

#[test]
fn cherry_pick_conflict_can_continue_and_preserves_author() {
    let (repo, _, _) = conflicting_cherry_pick();
    repo.write("file.txt", b"resolved\n");
    repo.git_ok(&["add", "--", "file.txt"]);
    run_git_seq(repo.path(), &["cherry-pick", "--continue", "--no-edit"]).unwrap();
    let opened = repo.open();
    let commit = opened.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(commit.author().email().ok(), Some("author@example.com"));
    assert_eq!(
        commit.committer().email().ok(),
        Some("committer@example.com")
    );
    assert!(!opened.path().join("CHERRY_PICK_HEAD").exists());
    assert_eq!(repo.index_bytes("file.txt"), b"resolved\n");
}

#[test]
fn cherry_pick_conflict_can_abort() {
    let (repo, _, head) = conflicting_cherry_pick();
    run_git_seq(repo.path(), &["cherry-pick", "--abort"]).unwrap();
    assert_eq!(repo.head(), head);
    assert_eq!(
        std::fs::read(repo.path().join("file.txt")).unwrap(),
        b"main\n"
    );
    assert!(!repo.open().index().unwrap().has_conflicts());
}

#[test]
fn successful_cherry_pick_preserves_original_author() {
    let repo = TestRepo::new();
    repo.commit("file.txt", b"base\n");
    repo.git_ok(&["checkout", "-b", "feature"]);
    let feature = repo.commit("file.txt", b"feature\n");
    repo.git_ok(&["checkout", "main"]);
    do_cherry_pick(repo.path(), &feature.to_string()).unwrap();
    let opened = repo.open();
    let commit = opened.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(commit.author().email().ok(), Some("author@example.com"));
    assert_eq!(
        commit.committer().email().ok(),
        Some("committer@example.com")
    );
}

#[test]
fn revert_conflict_can_continue_or_abort() {
    for abort in [false, true] {
        let repo = TestRepo::new();
        repo.commit("file.txt", b"base\n");
        let target = repo.commit("file.txt", b"change\n");
        let head = repo.commit("file.txt", b"later\n");
        assert!(do_revert(repo.path(), &target.to_string()).is_err());
        assert!(repo.open().path().join("REVERT_HEAD").exists());
        if abort {
            run_git_seq(repo.path(), &["revert", "--abort"]).unwrap();
            assert_eq!(repo.head(), head);
            assert_eq!(repo.index_bytes("file.txt"), b"later\n");
        } else {
            repo.write("file.txt", b"resolved\n");
            repo.git_ok(&["add", "--", "file.txt"]);
            run_git_seq(repo.path(), &["revert", "--continue", "--no-edit"]).unwrap();
            assert_ne!(repo.head(), head);
            assert_eq!(repo.index_bytes("file.txt"), b"resolved\n");
        }
        assert!(!repo.open().path().join("REVERT_HEAD").exists());
    }
}

fn with_origin() -> (TestRepo, tempfile::TempDir) {
    let local = TestRepo::new();
    local.commit("base.txt", b"base\n");
    let remote = tempfile::tempdir().unwrap();
    git2::Repository::init_bare(remote.path()).unwrap();
    local.git_ok(&["remote", "add", "origin", remote.path().to_str().unwrap()]);
    local.git_ok(&["push", "-u", "origin", "main"]);
    (local, remote)
}

fn read(repo: &TestRepo, name: &str) -> Vec<u8> {
    std::fs::read(repo.path().join(name)).unwrap()
}

#[test]
fn remote_branch_names_use_the_longest_configured_remote() {
    let repo = TestRepo::new();
    let opened = repo.open();
    opened.remote("origin", "/recorde-nonexistent-a").unwrap();
    opened
        .remote("origin/team", "/recorde-nonexistent-b")
        .unwrap();
    assert_eq!(
        split_remote_branch(&opened, "origin/feature/x").unwrap(),
        ("origin".into(), "feature/x".into())
    );
    assert_eq!(
        split_remote_branch(&opened, "origin/team/topic").unwrap(),
        ("origin/team".into(), "topic".into())
    );
    for unknown in ["upstream/main", "origin/", "origin", "originx/main"] {
        assert!(split_remote_branch(&opened, unknown).is_err(), "{unknown}");
    }
}

#[test]
fn remote_checkout_creates_tracking_branch_and_never_moves_existing_local_branches() {
    let (local, _remote) = with_origin();
    local.git_ok(&["checkout", "-b", "feature/x"]);
    let feature = local.commit("feature.txt", b"feature\n");
    local.git_ok(&["push", "origin", "feature/x"]);
    local.git_ok(&["checkout", "main"]);
    local.git_ok(&["branch", "-D", "feature/x"]);
    let main = local.head();

    assert_eq!(
        do_checkout_remote(local.path(), "origin/feature/x").unwrap(),
        "feature/x"
    );
    let repo = local.open();
    assert_eq!(repo.head().unwrap().shorthand().unwrap(), "feature/x");
    assert_eq!(local.head(), feature);
    let branch = repo
        .find_branch("feature/x", git2::BranchType::Local)
        .unwrap();
    assert_eq!(
        branch.upstream().unwrap().get().name().unwrap(),
        "refs/remotes/origin/feature/x"
    );

    // A branch that already tracks the remote branch is reused as it is.
    local.git_ok(&["checkout", "main"]);
    assert_eq!(
        do_checkout_remote(local.path(), "origin/feature/x").unwrap(),
        "feature/x"
    );
    assert_eq!(local.head(), feature);

    // A same-named branch that does not track it is refused and left alone.
    local.git_ok(&["checkout", "main"]);
    local.git_ok(&["branch", "--unset-upstream", "feature/x"]);
    local.git_ok(&["branch", "-f", "feature/x", "main"]);
    assert!(do_checkout_remote(local.path(), "origin/feature/x").is_err());
    assert_eq!(repo.head().unwrap().shorthand().unwrap(), "main");
    assert_eq!(repo.refname_to_id("refs/heads/feature/x").unwrap(), main);

    assert!(do_checkout_remote(local.path(), "origin/missing").is_err());
    assert_eq!(local.head(), main);
}

#[test]
fn remote_checkout_refuses_to_overwrite_local_changes() {
    let (local, _remote) = with_origin();
    local.git_ok(&["checkout", "-b", "topic"]);
    local.commit("base.txt", b"topic\n");
    local.git_ok(&["push", "origin", "topic"]);
    local.git_ok(&["checkout", "main"]);
    local.git_ok(&["branch", "-D", "topic"]);
    local.write("base.txt", b"local work\n");
    let main = local.head();
    assert!(do_checkout_remote(local.path(), "origin/topic").is_err());
    assert_eq!(local.head(), main);
    assert_eq!(read(&local, "base.txt"), b"local work\n");
}

#[test]
fn remote_branch_delete_removes_remote_and_tracking_refs_and_keeps_tip_for_restore() {
    let (local, remote_dir) = with_origin();
    local.git_ok(&["checkout", "-b", "topic"]);
    let tip = local.commit("topic.txt", b"topic\n");
    local.git_ok(&["push", "origin", "topic"]);
    local.git_ok(&["checkout", "main"]);
    local.git_ok(&["branch", "-D", "topic"]);
    let bare = git2::Repository::open_bare(remote_dir.path()).unwrap();
    assert_eq!(bare.refname_to_id("refs/heads/topic").unwrap(), tip);

    assert_eq!(
        do_delete_remote_branch(local.path(), "origin/topic").unwrap(),
        ("topic".to_string(), tip.to_string())
    );
    assert!(bare.refname_to_id("refs/heads/topic").is_err());
    assert_eq!(bare.refname_to_id("refs/heads/main").unwrap(), local.head());
    assert!(local
        .open()
        .refname_to_id("refs/remotes/origin/topic")
        .is_err());
    assert!(load_deleted(local.path())
        .iter()
        .any(|d| d.name == "topic" && d.oid == tip.to_string()));
    assert!(do_delete_remote_branch(local.path(), "origin/topic").is_err());

    // A failed push leaves the tracking ref and records nothing as deleted.
    local.git_ok(&["remote", "add", "broken", "/recorde-nonexistent-remote"]);
    let head = local.head().to_string();
    local.git_ok(&["update-ref", "refs/remotes/broken/ghost", &head]);
    assert!(do_delete_remote_branch(local.path(), "broken/ghost").is_err());
    assert!(local
        .open()
        .refname_to_id("refs/remotes/broken/ghost")
        .is_ok());
    assert!(load_deleted(local.path()).iter().all(|d| d.name != "ghost"));
}

#[test]
fn reset_to_commit_modes_move_head_and_recovery_restores_dirty_work() {
    for mode in ResetMode::ALL {
        let repo = TestRepo::new();
        let base = repo.commit("file.txt", b"base\n");
        repo.commit("file.txt", b"middle\n");
        let tip = repo.commit("file.txt", b"tip\n");
        repo.write("file.txt", b"staged\n");
        repo.git_ok(&["add", "file.txt"]);
        repo.write("file.txt", b"unstaged\n");
        repo.write("untracked.txt", b"keep\n");

        do_reset_to_commit(repo.path(), &base.to_string(), mode).unwrap();
        assert_eq!(repo.head(), base, "{mode:?}");
        assert_eq!(repo.open().head().unwrap().shorthand().unwrap(), "main");
        let (index, work): (&[u8], &[u8]) = match mode {
            ResetMode::Soft => (b"staged\n", b"unstaged\n"),
            ResetMode::Mixed => (b"base\n", b"unstaged\n"),
            ResetMode::Hard => (b"base\n", b"base\n"),
        };
        assert_eq!(repo.index_bytes("file.txt"), index, "{mode:?}");
        assert_eq!(read(&repo, "file.txt"), work, "{mode:?}");
        assert_eq!(read(&repo, "untracked.txt"), b"keep\n");

        let saved = crate::git::recovery::list(repo.path());
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].label, format!("{} reset", mode.label()));
        crate::git::recovery::restore(repo.path(), &saved[0]).unwrap();
        assert_eq!(repo.head(), tip, "{mode:?}");
        assert_eq!(repo.index_bytes("file.txt"), b"staged\n");
        assert_eq!(read(&repo, "file.txt"), b"unstaged\n");
    }
}

#[test]
fn reset_to_commit_refuses_unknown_targets_without_side_effects() {
    let repo = TestRepo::new();
    let head = repo.commit("file.txt", b"base\n");
    let missing = "0".repeat(40);
    for target in ["--hard", "HEAD~1", missing.as_str()] {
        assert!(do_reset_to_commit(repo.path(), target, ResetMode::Hard).is_err());
    }
    assert_eq!(repo.head(), head);
    assert!(crate::git::recovery::list(repo.path()).is_empty());
}

#[test]
fn stash_optionally_includes_untracked_files_and_single_file_stash_handles_new_files() {
    let repo = TestRepo::new();
    repo.commit("tracked.txt", b"base\n");
    repo.write("tracked.txt", b"edited\n");
    repo.write("new.txt", b"new\n");
    let count = || collect_stashes(&mut repo.open()).len();

    do_stash_save(repo.path(), "only tracked", false).unwrap();
    assert_eq!(read(&repo, "tracked.txt"), b"base\n");
    assert_eq!(read(&repo, "new.txt"), b"new\n");
    let stashes = collect_stashes(&mut repo.open());
    assert_eq!(stashes.len(), 1);
    assert!(stashes[0].message.ends_with("only tracked"));
    do_stash_apply(repo.path(), &stash_target(&repo, 0), true).unwrap();
    assert_eq!(read(&repo, "tracked.txt"), b"edited\n");
    assert_eq!(count(), 0);

    do_stash_save(repo.path(), "everything", true).unwrap();
    assert!(!repo.path().join("new.txt").exists());
    assert_eq!(read(&repo, "tracked.txt"), b"base\n");
    do_stash_apply(repo.path(), &stash_target(&repo, 0), false).unwrap();
    assert_eq!(read(&repo, "new.txt"), b"new\n");
    assert_eq!(read(&repo, "tracked.txt"), b"edited\n");
    assert_eq!(count(), 1);
    do_stash_drop(repo.path(), &stash_target(&repo, 0)).unwrap();
    assert_eq!(count(), 0);

    repo.write("other.txt", b"other\n");
    do_stash_file(repo.path(), "new.txt").unwrap();
    assert!(!repo.path().join("new.txt").exists());
    assert_eq!(read(&repo, "other.txt"), b"other\n");
    assert_eq!(read(&repo, "tracked.txt"), b"edited\n");
    assert!(collect_stashes(&mut repo.open())[0]
        .message
        .ends_with("Stash new.txt"));
    do_stash_file(repo.path(), "tracked.txt").unwrap();
    assert_eq!(read(&repo, "tracked.txt"), b"base\n");
    assert_eq!(read(&repo, "other.txt"), b"other\n");
    assert_eq!(count(), 2);
    do_stash_apply(repo.path(), &stash_target(&repo, 1), true).unwrap();
    assert_eq!(read(&repo, "new.txt"), b"new\n");
}

#[test]
fn tags_create_push_and_delete_on_the_remote_without_removing_local_tags() {
    let (local, remote_dir) = with_origin();
    let head = local.head();
    do_create_tag(local.path(), "light", None, None).unwrap();
    do_create_tag(
        local.path(),
        "v1.0",
        Some(&head.to_string()),
        Some("  Release 1.0  "),
    )
    .unwrap();
    assert!(do_create_tag(local.path(), "v1.0", None, None).is_err());
    let repo = local.open();
    assert_eq!(repo.refname_to_id("refs/tags/light").unwrap(), head);
    let tag = repo
        .find_reference("refs/tags/v1.0")
        .unwrap()
        .peel_to_tag()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(tag.message_bytes().unwrap_or_default()).trim(),
        "Release 1.0"
    );
    assert_eq!(tag.target_id(), head);

    do_remote_tag(local.path(), "v1.0", false).unwrap();
    let bare = git2::Repository::open_bare(remote_dir.path()).unwrap();
    assert!(bare.find_reference("refs/tags/v1.0").is_ok());
    assert!(bare.find_reference("refs/tags/light").is_err());
    do_remote_tag(local.path(), "v1.0", true).unwrap();
    assert!(bare.find_reference("refs/tags/v1.0").is_err());
    assert!(repo.find_reference("refs/tags/v1.0").is_ok());
}

#[test]
fn rename_and_safe_delete_refuse_missing_or_unmerged_branches() {
    let repo = TestRepo::new();
    repo.commit("file", b"base\n");
    do_create(repo.path(), "old-name", None, false).unwrap();
    do_rename(repo.path(), "old-name", "new-name").unwrap();
    let opened = repo.open();
    assert!(opened
        .find_branch("old-name", git2::BranchType::Local)
        .is_err());
    assert!(opened
        .find_branch("new-name", git2::BranchType::Local)
        .is_ok());
    assert!(do_rename(repo.path(), "missing", "other").is_err());
    assert!(do_rename(repo.path(), "new-name", "main").is_err());

    repo.git_ok(&["checkout", "-b", "unmerged"]);
    let tip = repo.commit("file", b"unmerged\n");
    repo.git_ok(&["checkout", "main"]);
    assert!(do_delete(repo.path(), "unmerged").is_err());
    assert_eq!(opened.refname_to_id("refs/heads/unmerged").unwrap(), tip);
    assert!(load_deleted(repo.path())
        .iter()
        .all(|d| d.name != "unmerged"));
    do_delete(repo.path(), "new-name").unwrap();
    assert!(opened
        .find_branch("new-name", git2::BranchType::Local)
        .is_err());
}

#[test]
fn several_commits_are_cherry_picked_oldest_first_and_reverted_newest_first() {
    let repo = TestRepo::new();
    let base = repo.commit("file.txt", b"1\n");
    repo.git_ok(&["checkout", "-b", "topic"]);
    let first = repo.commit("file.txt", b"1\n2\n");
    let second = repo.commit("file.txt", b"1\n2\n3\n");
    repo.git_ok(&["checkout", "main"]);
    let picks = [first.to_string(), second.to_string()];
    do_sequence_many(repo.path(), &picks, false).unwrap();
    assert_eq!(read(&repo, "file.txt"), b"1\n2\n3\n");
    let git = repo.open();
    let head = git.find_commit(repo.head()).unwrap();
    assert_eq!(head.parent(0).unwrap().parent_id(0).unwrap(), base);
    assert_eq!(
        crate::git::recovery::undo_history(repo.path())
            .next(false)
            .unwrap()
            .1,
        "Undo Cherry-pick 2 commits"
    );

    // Reverting both in order newest first applies cleanly.
    let applied: Vec<String> = vec![
        head.parent_id(0).unwrap().to_string(),
        head.id().to_string(),
    ];
    do_sequence_many(repo.path(), &applied, true).unwrap();
    assert_eq!(read(&repo, "file.txt"), b"1\n");
    assert!(do_sequence_many(repo.path(), &[], false).is_err());
    assert!(do_sequence_many(repo.path(), &["not-a-commit".into()], false).is_err());
}

#[test]
fn merge_commits_are_refused_in_a_multi_commit_cherry_pick() {
    let repo = TestRepo::new();
    repo.commit("a.txt", b"a\n");
    repo.git_ok(&["checkout", "-b", "side"]);
    repo.commit("b.txt", b"b\n");
    repo.git_ok(&["checkout", "main"]);
    repo.commit("c.txt", b"c\n");
    repo.git_ok(&["merge", "--no-ff", "--no-edit", "side"]);
    let merge = repo.head();
    repo.git_ok(&["checkout", "-b", "other", "HEAD~1"]);
    let before = repo.head();
    let error = do_sequence_many(repo.path(), &[merge.to_string()], false).unwrap_err();
    assert!(error.contains("merge commit"), "{error}");
    assert_eq!(repo.head(), before);
}

fn sequence_preview_fixture() -> (TestRepo, CommitSequencePreview) {
    let repo = TestRepo::new();
    repo.commit("base", b"base");
    repo.git_ok(&["checkout", "-b", "topic"]);
    let first = repo.commit("first", b"first").to_string();
    let second = repo.commit("second", b"second").to_string();
    repo.git_ok(&["checkout", "main"]);
    let preview =
        CommitSequencePreview::capture(repo.path(), 0, vec![first, second], false).unwrap();
    (repo, preview)
}

#[test]
fn multi_commit_request_only_opens_a_frozen_confirmation() {
    let (repo, preview) = sequence_preview_fixture();
    let head = repo.head();
    let state = crate::state::new_shared_state();
    state.lock().unwrap().repo_path = Some(repo.path().canonicalize().unwrap());
    cherry_pick_many(preview.oids.clone(), state.clone());
    let s = state.lock().unwrap();
    let crate::state::ModalKind::ConfirmCommitSequence { preview: captured } = &s.modal else {
        panic!("multi-commit request must open confirmation");
    };
    assert_eq!(captured.oids, preview.oids);
    assert_eq!(captured.head_ref, "refs/heads/main");
    assert_eq!(captured.head_oid, head.to_string());
    assert!(s.local_jobs.is_empty());
    assert_eq!(repo.head(), head);
    assert!(!repo.path().join("first").exists());
}

#[test]
fn stale_multi_commit_destination_never_mutates_history_or_worktree() {
    for moved_head in [false, true] {
        let (repo, preview) = sequence_preview_fixture();
        if moved_head {
            repo.commit("other", b"other");
        } else {
            repo.git_ok(&["checkout", "-b", "same-tip-other-branch"]);
        }
        let head = repo.head();
        let index = std::fs::read(repo.path().join(".git/index")).unwrap();
        assert!(preview
            .execute(repo.path())
            .unwrap_err()
            .contains("HEAD changed"));
        assert_eq!(repo.head(), head);
        assert_eq!(
            std::fs::read(repo.path().join(".git/index")).unwrap(),
            index
        );
        assert!(!repo.path().join("first").exists());
        assert!(crate::git::recovery::undo_history(repo.path())
            .next(false)
            .is_none());
    }
    let (repo, preview) = sequence_preview_fixture();
    let other = TestRepo::new();
    let other_head = other.commit("other", b"keep");
    assert!(preview
        .execute(other.path())
        .unwrap_err()
        .contains("Repository changed"));
    assert_eq!(other.head(), other_head);
    assert_eq!(repo.head().to_string(), preview.head_oid);
}

#[test]
fn multi_commit_rechecks_replacements_dirty_files_and_full_distinct_ids() {
    let (repo, preview) = sequence_preview_fixture();
    let head = repo.head();
    for ids in [
        vec![preview.oids[0][..7].into()],
        vec![preview.oids[0].clone(); 2],
        vec!["--abort".into()],
    ] {
        assert!(CommitSequencePreview::capture(repo.path(), 0, ids, false).is_err());
    }
    repo.write("base", b"staged");
    repo.git_ok(&["add", "base"]);
    repo.write("base", b"unstaged");
    repo.write("untracked", b"keep");
    let index = std::fs::read(repo.path().join(".git/index")).unwrap();
    assert!(preview
        .execute(repo.path())
        .unwrap_err()
        .contains("Commit or stash"));
    repo.git_ok(&["replace", &preview.oids[0], &head.to_string()]);
    assert!(preview
        .execute(repo.path())
        .unwrap_err()
        .contains("replacement objects"));
    assert_eq!(repo.head(), head);
    assert_eq!(
        std::fs::read(repo.path().join(".git/index")).unwrap(),
        index
    );
    assert_eq!(read(&repo, "base"), b"unstaged");
    assert_eq!(read(&repo, "untracked"), b"keep");
    assert!(!repo.path().join("first").exists());
}

#[test]
fn queued_multi_commit_rechecks_epoch_and_head_before_running() {
    for change_epoch in [false, true] {
        let (repo, preview) = sequence_preview_fixture();
        let state = crate::state::new_shared_state();
        state.lock().unwrap().repo_path = Some(repo.path().canonicalize().unwrap());
        let reservation = crate::git::mutation::reserve(repo.path());
        preview.submit(state.clone());
        if change_epoch {
            state.lock().unwrap().repo_epoch += 1;
        } else {
            repo.git_ok(&["checkout", "-b", "different-branch"]);
        }
        let head = repo.head();
        drop(reservation);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !state.lock().unwrap().local_jobs.is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(state.lock().unwrap().local_jobs.is_empty());
        assert_eq!(repo.head(), head);
        assert!(!repo.path().join("first").exists());
    }
}

fn stash_target(repo: &TestRepo, index: usize) -> StashTarget {
    let stashes = collect_stashes(&mut repo.open());
    StashTarget::capture(repo.path(), 0, index, &stashes[index].oid).unwrap()
}

fn three_stashes() -> TestRepo {
    let repo = TestRepo::new();
    repo.commit("file", b"base\n");
    for message in ["first", "second", "third"] {
        repo.write("file", message.as_bytes());
        do_stash_save(repo.path(), message, false).unwrap();
    }
    repo
}

#[test]
fn queued_pop_then_stale_drop_refuses_to_remove_a_different_stash() {
    let repo = three_stashes();
    let before = collect_stashes(&mut repo.open());
    let pop = stash_target(&repo, 0);
    let drop_target = stash_target(&repo, 1);
    let hold = crate::git::mutation::reserve(repo.path());
    let path = repo.path().to_path_buf();
    let first =
        crate::git::mutation::spawn(path.clone(), move || do_stash_apply(&path, &pop, true));
    let path = repo.path().to_path_buf();
    let second =
        crate::git::mutation::spawn(path.clone(), move || do_stash_drop(&path, &drop_target));
    drop(hold);
    first.join().unwrap().unwrap();
    assert!(second
        .join()
        .unwrap()
        .unwrap_err()
        .contains("stash list changed"));
    let after = collect_stashes(&mut repo.open());
    assert_eq!(
        after.iter().map(|stash| &stash.oid).collect::<Vec<_>>(),
        before[1..]
            .iter()
            .map(|stash| &stash.oid)
            .collect::<Vec<_>>()
    );
    assert_eq!(read(&repo, "file"), b"third");
}

#[test]
fn shifted_stash_refuses_apply_pop_drop_and_branch_without_changes() {
    let repo = three_stashes();
    let stale = stash_target(&repo, 1);
    do_stash_drop(repo.path(), &stash_target(&repo, 0)).unwrap();
    let before = collect_stashes(&mut repo.open());
    let head = repo.head();
    let index = std::fs::read(repo.path().join(".git/index")).unwrap();
    for pop in [false, true] {
        assert!(do_stash_apply(repo.path(), &stale, pop)
            .unwrap_err()
            .contains("stash list changed"));
    }
    assert!(do_stash_drop(repo.path(), &stale)
        .unwrap_err()
        .contains("stash list changed"));
    assert!(do_stash_branch(repo.path(), "wrong-stash", &stale)
        .unwrap_err()
        .contains("stash list changed"));
    assert!(repo
        .open()
        .find_branch("wrong-stash", git2::BranchType::Local)
        .is_err());
    assert_eq!(repo.head(), head);
    assert_eq!(read(&repo, "file"), b"base\n");
    assert_eq!(
        std::fs::read(repo.path().join(".git/index")).unwrap(),
        index
    );
    assert_eq!(
        collect_stashes(&mut repo.open())
            .iter()
            .map(|stash| &stash.oid)
            .collect::<Vec<_>>(),
        before.iter().map(|stash| &stash.oid).collect::<Vec<_>>()
    );
}

#[test]
fn removed_stash_survives_gc_and_can_be_restored_to_the_stash_list() {
    let repo = three_stashes();
    let target = stash_target(&repo, 1);
    let recovery_ref = target.recovery_ref();
    do_stash_drop(repo.path(), &target).unwrap();
    repo.git_ok(&["gc", "--prune=now"]);
    assert_eq!(
        repo.open()
            .refname_to_id(&recovery_ref)
            .unwrap()
            .to_string(),
        target.oid
    );
    repo.git_ok(&[
        "stash",
        "store",
        "-m",
        "Recovered by Recorde",
        &recovery_ref,
    ]);
    assert_eq!(collect_stashes(&mut repo.open())[0].oid, target.oid);
    do_stash_apply(repo.path(), &stash_target(&repo, 0), false).unwrap();
    assert_eq!(read(&repo, "file"), b"second");
}

#[test]
fn branch_from_stash_applies_exact_object_and_retains_removed_stash() {
    let repo = three_stashes();
    let target = stash_target(&repo, 1);
    let before = collect_stashes(&mut repo.open());
    do_stash_branch(repo.path(), "from-stash", &target).unwrap();
    assert_eq!(
        repo.open().head().unwrap().shorthand().unwrap(),
        "from-stash"
    );
    assert_eq!(read(&repo, "file"), b"second");
    let after = collect_stashes(&mut repo.open());
    assert_eq!(
        after.iter().map(|stash| &stash.oid).collect::<Vec<_>>(),
        vec![&before[0].oid, &before[2].oid]
    );
    assert_eq!(
        repo.open()
            .refname_to_id(&target.recovery_ref())
            .unwrap()
            .to_string(),
        target.oid
    );
}

#[test]
fn t5_conflicted_pop_keeps_the_reviewed_and_unrelated_stash_objects() {
    let repo = TestRepo::new();
    repo.commit("file.txt", b"base\n");
    repo.write("file.txt", b"stashed change\n");
    repo.git_ok(&["stash", "push", "-m", "reviewed stash"]);
    repo.write("unrelated.txt", b"unrelated saved bytes\n");
    repo.git_ok(&[
        "stash",
        "push",
        "--include-untracked",
        "-m",
        "unrelated stash",
    ]);
    repo.commit("file.txt", b"divergent committed change\n");
    let target = stash_target(&repo, 1);
    let before = collect_stashes(&mut repo.open());
    assert!(do_stash_apply(repo.path(), &target, true).is_err());
    let after = collect_stashes(&mut repo.open());
    assert_eq!(
        after.iter().map(|s| &s.oid).collect::<Vec<_>>(),
        before.iter().map(|s| &s.oid).collect::<Vec<_>>()
    );
    assert_eq!(after[1].oid, target.oid);
    assert!(repo.open().index().unwrap().has_conflicts());
    assert!(read(&repo, "file.txt").windows(7).any(|p| p == b"<<<<<<<"));
    assert!(!repo.path().join("unrelated.txt").exists());
}

#[test]
fn t15_stash_branch_refuses_ignored_base_collision_and_keeps_stash() {
    use crate::git::test_support::{ignored_collision, snapshot};
    let (repo, _) = ignored_collision();
    let ignored = std::fs::read(repo.path().join("ignored.env")).unwrap();
    std::fs::remove_file(repo.path().join("ignored.env")).unwrap();
    repo.git_ok(&["checkout", "destination"]);
    repo.write(".gitignore", b"/ignored.env\n/another\n");
    repo.git_ok(&["stash", "push", "-m", "stash on tracked collision base"]);
    repo.git_ok(&["checkout", "main"]);
    repo.write("ignored.env", &ignored);
    let target = stash_target(&repo, 0);
    let before = snapshot(repo.path());
    assert!(
        do_stash_branch(repo.path(), "from-stash", &target).is_err(),
        "Ignored base collision was overwritten by stash branch"
    );
    assert_eq!(snapshot(repo.path()), before);
    assert_eq!(collect_stashes(&mut repo.open())[0].oid, target.oid);
}

#[test]
fn stash_target_rejects_wrong_identity_repository_and_queued_epoch() {
    let repo = three_stashes();
    let target = stash_target(&repo, 0);
    assert!(StashTarget::capture(repo.path(), 0, 0, &target.oid[..7]).is_err());
    assert!(StashTarget::capture(repo.path(), 0, 0, &stash_target(&repo, 1).oid).is_err());
    let other = three_stashes();
    assert!(do_stash_drop(other.path(), &target)
        .unwrap_err()
        .contains("Repository changed"));
    assert_eq!(collect_stashes(&mut other.open()).len(), 3);
    let state = crate::state::new_shared_state();
    state.lock().unwrap().repo_path = Some(repo.path().canonicalize().unwrap());
    let hold = crate::git::mutation::reserve(repo.path());
    stash_drop(target.clone(), state.clone());
    state.lock().unwrap().repo_epoch += 1;
    drop(hold);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !state.lock().unwrap().local_jobs.is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(state.lock().unwrap().local_jobs.is_empty());
    let after = collect_stashes(&mut repo.open());
    assert_eq!(after.len(), 3);
    assert_eq!(after[0].oid, target.oid);
    assert!(repo.open().find_reference(&target.recovery_ref()).is_err());
}

#[test]
fn remote_branch_delete_lease_refuses_an_unfetched_advance() {
    let (local, remote_dir) = with_origin();
    local.git_ok(&["checkout", "-b", "topic"]);
    let known = local.commit("topic", b"known\n");
    local.git_ok(&["push", "origin", "topic"]);
    local.git_ok(&["checkout", "main"]);
    let bare = git2::Repository::open_bare(remote_dir.path()).unwrap();
    let parent = bare.find_commit(known).unwrap();
    let signature = git2::Signature::now("Another author", "other@example.com").unwrap();
    let advanced = bare
        .commit(
            Some("refs/heads/topic"),
            &signature,
            &signature,
            "unfetched advance",
            &parent.tree().unwrap(),
            &[&parent],
        )
        .unwrap();
    assert!(do_delete_remote_branch(local.path(), "origin/topic").is_err());
    assert_eq!(bare.refname_to_id("refs/heads/topic").unwrap(), advanced);
    assert_eq!(
        local
            .open()
            .refname_to_id("refs/remotes/origin/topic")
            .unwrap(),
        known
    );
    assert!(load_deleted(local.path())
        .iter()
        .all(|entry| entry.name != "topic"));
}

#[test]
fn reset_preview_rejects_changed_branch_tip_repository_and_detached_state() {
    for change in ["branch", "tip", "detach"] {
        let repo = TestRepo::new();
        let base = repo.commit("file", b"base\n");
        let tip = repo.commit("file", b"tip\n");
        let preview = ResetPreview::capture(repo.path(), 0, &base.to_string()).unwrap();
        match change {
            "branch" => repo.git_ok(&["checkout", "-b", "other"]),
            "tip" => {
                repo.commit("new", b"new");
            }
            _ => repo.git_ok(&["checkout", "--detach", &tip.to_string()]),
        }
        let head = repo.head();
        repo.write("file", b"local draft\n");
        let index = std::fs::read(repo.path().join(".git/index")).unwrap();
        assert!(preview
            .execute(repo.path(), ResetMode::Hard)
            .unwrap_err()
            .contains("HEAD changed"));
        assert_eq!(repo.head(), head);
        assert_eq!(read(&repo, "file"), b"local draft\n");
        assert_eq!(
            std::fs::read(repo.path().join(".git/index")).unwrap(),
            index
        );
        assert!(crate::git::recovery::list(repo.path()).is_empty());
        let other = TestRepo::new();
        let other_head = other.commit("other", b"keep");
        assert!(preview
            .execute(other.path(), ResetMode::Hard)
            .unwrap_err()
            .contains("Repository changed"));
        assert_eq!(other.head(), other_head);
    }
}

#[test]
fn reset_revalidates_branch_after_recovery_snapshot_before_touching_files() {
    let repo = TestRepo::new();
    let base = repo.commit("file", b"base\n");
    let tip = repo.commit("file", b"tip\n");
    let preview = ResetPreview::capture(repo.path(), 0, &base.to_string()).unwrap();
    repo.write("file", b"draft\n");
    let result =
        crate::git::recovery::protect(repo.path(), "Snapshot with concurrent checkout", || {
            repo.git_ok(&["checkout", "-b", "other"]);
            preview.reset_after_snapshot(repo.path(), ResetMode::Hard)
        });
    assert!(result.unwrap_err().contains("HEAD changed"));
    assert_eq!(repo.head(), tip);
    assert_eq!(repo.open().head().unwrap().shorthand().unwrap(), "other");
    assert_eq!(read(&repo, "file"), b"draft\n");
}

#[test]
fn reset_rechecks_session_after_snapshot_and_supports_explicit_detached_head() {
    let repo = TestRepo::new();
    let base = repo.commit("file", b"base\n");
    let tip = repo.commit("file", b"tip\n");
    repo.git_ok(&["checkout", "--detach", &tip.to_string()]);
    let preview = ResetPreview::capture(repo.path(), 0, &base.to_string()).unwrap();
    assert_eq!(preview.head_ref, "HEAD");
    let checks = std::cell::Cell::new(0);
    let result = preview.execute_guarded(repo.path(), ResetMode::Hard, || {
        checks.set(checks.get() + 1);
        if checks.get() > 1 {
            Err("Repository session changed after snapshot".into())
        } else {
            Ok(())
        }
    });
    assert_eq!(checks.get(), 2);
    assert!(result
        .unwrap_err()
        .contains("session changed after snapshot"));
    assert_eq!(repo.head(), tip);
    assert_eq!(read(&repo, "file"), b"tip\n");
    preview.execute(repo.path(), ResetMode::Hard).unwrap();
    assert_eq!(repo.head(), base);
    assert!(repo.open().head_detached().unwrap());
}

#[test]
fn queued_reset_rechecks_repository_epoch_and_branch() {
    for change_epoch in [false, true] {
        let repo = TestRepo::new();
        let base = repo.commit("file", b"base\n");
        let tip = repo.commit("file", b"tip\n");
        let preview = ResetPreview::capture(repo.path(), 0, &base.to_string()).unwrap();
        let state = crate::state::new_shared_state();
        state.lock().unwrap().repo_path = Some(repo.path().canonicalize().unwrap());
        let hold = crate::git::mutation::reserve(repo.path());
        preview.submit(ResetMode::Hard, state.clone());
        if change_epoch {
            state.lock().unwrap().repo_epoch += 1;
        } else {
            repo.git_ok(&["checkout", "-b", "other"]);
        }
        repo.write("file", b"draft\n");
        drop(hold);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !state.lock().unwrap().local_jobs.is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(state.lock().unwrap().local_jobs.is_empty());
        assert_eq!(repo.head(), tip);
        assert_eq!(read(&repo, "file"), b"draft\n");
        assert!(crate::git::recovery::list(repo.path()).is_empty());
    }
}
