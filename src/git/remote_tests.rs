use super::*;
use crate::git::test_support::TestRepo;

fn setup() -> (TestRepo, tempfile::TempDir) {
    let local = TestRepo::new();
    local.commit("base.txt", b"base\n");
    let remote = tempfile::tempdir().unwrap();
    git2::Repository::init_bare(remote.path()).unwrap();
    local.git_ok(&["remote", "add", "upstream", remote.path().to_str().unwrap()]);
    local.git_ok(&["push", "-u", "upstream", "main:trunk"]);
    // An unrelated remote must not be fetched by pull or selected by push.
    local.git_ok(&["remote", "add", "origin", "/recorde-nonexistent-remote"]);
    (local, remote)
}

#[test]
fn branch_push_override_and_lease_target_the_fork_without_changing_pull_upstream() {
    let (local, upstream_dir) = setup();
    let fork_dir = tempfile::tempdir().unwrap();
    let fork = git2::Repository::init_bare(fork_dir.path()).unwrap();
    local.git_ok(&["remote", "add", "fork", fork_dir.path().to_str().unwrap()]);
    local.git_ok(&["push", "fork", "main"]);
    local.git_ok(&["config", "remote.pushDefault", "origin"]);
    local.git_ok(&["config", "branch.main.pushRemote", "fork"]);
    let before = local.head();
    let lease = capture_lease(local.path()).unwrap();
    assert_eq!(lease.remote, "fork");
    assert_eq!(lease.destination, "refs/heads/main");
    assert_eq!(lease.expected, before.to_string());
    let next = local.commit("next", b"next");
    push(local.path(), &AtomicBool::new(false)).unwrap();
    assert_eq!(fork.refname_to_id("refs/heads/main").unwrap(), next);
    assert_eq!(
        git2::Repository::open_bare(upstream_dir.path())
            .unwrap()
            .refname_to_id("refs/heads/trunk")
            .unwrap(),
        before
    );
    assert_eq!(upstream(local.path()).unwrap().0, "upstream");
    save_remote(
        local.path(),
        "personal",
        fork_dir.path().to_str().unwrap(),
        Some("fork"),
    )
    .unwrap();
    assert_eq!(
        local
            .open()
            .config()
            .unwrap()
            .get_string("branch.main.pushRemote")
            .unwrap(),
        "personal"
    );
    remove_remote(local.path(), "personal").unwrap();
    assert!(local
        .open()
        .config()
        .unwrap()
        .get_string("branch.main.pushRemote")
        .is_err());
}

#[test]
fn push_uses_exact_upstream_remote_and_differently_named_branch() {
    let (local, remote) = setup();
    let head = local.commit("local.txt", b"local\n");
    push(local.path(), &AtomicBool::new(false)).unwrap();
    let bare = git2::Repository::open_bare(remote.path()).unwrap();
    assert_eq!(bare.refname_to_id("refs/heads/trunk").unwrap(), head);
    assert!(bare.find_reference("refs/heads/main").is_err());
}

#[test]
fn divergent_pull_strategies_merge_rebase_or_refuse_without_data_loss() {
    for strategy in ["merge", "rebase", "ff-only"] {
        let (local, remote) = setup();
        let peer = TestRepo::new();
        peer.git_ok(&["remote", "add", "upstream", remote.path().to_str().unwrap()]);
        peer.git_ok(&["fetch", "upstream"]);
        peer.git_ok(&["checkout", "-B", "main", "upstream/trunk"]);
        let upstream = peer.commit("peer.txt", b"peer\n");
        peer.git_ok(&["push", "upstream", "main:trunk"]);
        let before = local.commit("local.txt", b"local\n");
        let result = pull(local.path(), &AtomicBool::new(false), strategy);
        if strategy == "ff-only" {
            assert!(result.is_err());
            assert_eq!(local.head(), before);
            assert!(!local.path().join("peer.txt").exists());
        } else {
            result.unwrap();
            assert_eq!(
                std::fs::read(local.path().join("peer.txt")).unwrap(),
                b"peer\n"
            );
            let repo = local.open();
            let head = repo.head().unwrap().peel_to_commit().unwrap();
            if strategy == "merge" {
                assert_eq!(head.parent_count(), 2);
            } else {
                assert_eq!(head.parent_id(0).unwrap(), upstream);
                assert_eq!(head.author().email().ok(), Some("author@example.com"));
            }
        }
        assert_eq!(local.index_bytes("local.txt"), b"local\n");
        assert_eq!(
            std::fs::read(local.path().join("local.txt")).unwrap(),
            b"local\n"
        );
    }
}

#[test]
fn pruning_is_explicit_and_cancellation_does_not_push() {
    let (local, _remote) = setup();
    local.git_ok(&["push", "upstream", "main:obsolete"]);
    fetch_remote(local.path(), "upstream", &AtomicBool::new(false), false).unwrap();
    local.git_ok(&["push", "upstream", ":obsolete"]);
    // Restore a stale tracking ref to model deletion by another client.
    local.git_ok(&["update-ref", "refs/remotes/upstream/obsolete", "HEAD"]);
    fetch_remote(local.path(), "upstream", &AtomicBool::new(false), false).unwrap();
    assert!(local
        .open()
        .find_reference("refs/remotes/upstream/obsolete")
        .is_ok());
    fetch_remote(local.path(), "upstream", &AtomicBool::new(false), true).unwrap();
    assert!(local
        .open()
        .find_reference("refs/remotes/upstream/obsolete")
        .is_err());
    assert!(matches!(
        push(local.path(), &AtomicBool::new(true)),
        Err(GitError::Cancelled)
    ));
}

#[test]
fn publishing_requires_a_remote_choice_when_ambiguous() {
    let (local, _remote) = setup();
    assert!(default_push_remote(&local.open()).is_err());
    local.git_ok(&["config", "remote.pushDefault", "upstream"]);
    assert_eq!(default_push_remote(&local.open()).unwrap(), "upstream");
    local.git_ok(&["config", "remote.pushDefault", "missing"]);
    assert!(default_push_remote(&local.open()).is_err());
}

#[test]
fn remote_lifecycle_preserves_selected_remote_after_rename() {
    let repo = TestRepo::new();
    save_remote(
        repo.path(),
        "first",
        "https://example.invalid/one.git",
        None,
    )
    .unwrap();
    repo.git_ok(&["config", "remote.pushDefault", "first"]);
    repo.git_ok(&["config", "recorde.providerRemote", "first"]);
    save_remote(
        repo.path(),
        "renamed",
        "https://example.invalid/two.git",
        Some("first"),
    )
    .unwrap();
    let opened = repo.open();
    assert_eq!(default_push_remote(&opened).unwrap(), "renamed");
    assert_eq!(provider_remote(&opened).unwrap(), "renamed");
    assert_eq!(
        opened.find_remote("renamed").unwrap().url().ok(),
        Some("https://example.invalid/two.git")
    );
    remove_remote(repo.path(), "renamed").unwrap();
    assert!(repo.open().remotes().unwrap().is_empty());
    assert!(repo
        .open()
        .config()
        .unwrap()
        .get_string("recorde.providerRemote")
        .is_err());
}

#[test]
fn explicit_lease_survives_background_fetch_and_rejects_unseen_remote_changes() {
    let (local, remote) = setup();
    let lease = capture_lease(local.path()).unwrap();
    let peer = TestRepo::new();
    peer.git_ok(&["remote", "add", "upstream", remote.path().to_str().unwrap()]);
    peer.git_ok(&["fetch", "upstream"]);
    peer.git_ok(&["checkout", "-B", "main", "upstream/trunk"]);
    let unseen = peer.commit("peer.txt", b"must keep\n");
    peer.git_ok(&["push", "upstream", "main:trunk"]);
    local.commit("local.txt", b"local\n");
    fetch_remote(local.path(), "upstream", &AtomicBool::new(false), false).unwrap();
    assert!(run_push_with_lease(local.path(), &lease, &AtomicBool::new(false)).is_err());
    assert_eq!(
        git2::Repository::open_bare(remote.path())
            .unwrap()
            .refname_to_id("refs/heads/trunk")
            .unwrap(),
        unseen
    );
}
