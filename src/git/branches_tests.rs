use super::*;
use crate::git::test_support::TestRepo;

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
    assert_eq!(commit.committer().email().ok(), Some("committer@example.com"));
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
    assert_eq!(commit.committer().email().ok(), Some("committer@example.com"));
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
