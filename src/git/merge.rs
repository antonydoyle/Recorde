use std::path::Path;

use crate::git::error::GitError;
use crate::state::SharedState;

// ── Shared shell helper ───────────────────────────────────────────────────────

/// Run `git -C <repo_path> <args>` and return stderr as a GitError on failure.
pub(crate) fn run_git_seq(repo_path: &Path, args: &[&str]) -> Result<(), GitError> {
    crate::git::process::git(repo_path, args, &std::sync::atomic::AtomicBool::new(false))
        .map(|_| ())
}

// ── Merge ─────────────────────────────────────────────────────────────────────

/// How a merge records history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MergeMode {
    /// Fast-forward when possible, otherwise create a merge commit.
    #[default]
    Default,
    /// Always create a merge commit.
    NoFastForward,
    /// Refuse unless the current branch can simply move forward.
    FastForwardOnly,
    /// Combine the changes into one new commit without merge ancestry.
    Squash,
}

impl MergeMode {
    pub const ALL: [Self; 4] = [
        Self::Default,
        Self::NoFastForward,
        Self::FastForwardOnly,
        Self::Squash,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Fast-forward if possible",
            Self::NoFastForward => "Always create a merge commit",
            Self::FastForwardOnly => "Fast-forward only",
            Self::Squash => "Squash into one commit",
        }
    }

    /// Whether a commit message can be supplied.
    pub fn takes_message(self) -> bool {
        matches!(self, Self::NoFastForward | Self::Squash)
    }
}

/// Merge `branch_name` into the current branch.
/// Aborts with an error if the working tree is dirty.
pub fn merge_branch(branch_name: &str, state: SharedState) {
    merge_branch_with(branch_name, MergeMode::Default, None, state);
}

pub fn merge_branch_with(
    branch_name: &str,
    mode: MergeMode,
    message: Option<String>,
    state: SharedState,
) {
    let name = branch_name.to_string();
    let (path, dirty) = {
        let s = state.lock().unwrap();
        (s.repo_path.clone(), tracked_changes(&s.working_dir_files))
    };
    let Some(path) = path else { return };

    // Untracked files may stay; Git refuses itself if the merge would overwrite one.
    if dirty {
        state.lock().unwrap().error =
            Some("Stash or commit changes to tracked files before merging.".into());
        return;
    }

    {
        let mut s = state.lock().unwrap();
        s.remote_op = Some(format!("Merging {}…", name));
        s.console_log.push(format!("$ git merge {}", name));
    }

    crate::git::mutation::spawn_operation(path, state, move |path| {
        do_merge(path, &name, mode, message.as_deref())
    });
}

fn tracked_changes(files: &[crate::state::WorkingDirFile]) -> bool {
    files
        .iter()
        .any(|f| f.staged_status.is_some() || f.unstaged_status.is_some_and(|s| s != '?'))
}

pub(crate) fn do_merge(
    path: &Path,
    branch: &str,
    mode: MergeMode,
    message: Option<&str>,
) -> Result<(), String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let mut options = git2::StatusOptions::new();
    options.include_untracked(false).include_ignored(false);
    if !repo
        .statuses(Some(&mut options))
        .map_err(|e| e.to_string())?
        .is_empty()
    {
        return Err("Stash or commit changes to tracked files before merging.".into());
    }
    let message = message.map(str::trim).filter(|m| !m.is_empty());
    let ignored = super::ignored::Guard::prepare(path, &[branch])?;
    let target = ignored.operand(0);
    let label = if mode == MergeMode::Squash {
        "Squash merge"
    } else {
        "Merge"
    };
    super::recovery::protect_sequence(path, label, || {
        ignored.validate()?;
        if mode == MergeMode::Squash {
            write_squash_state(&repo, message)?;
        }
        let mut args = vec!["merge", "--no-overwrite-ignore"];
        match mode {
            MergeMode::Default => args.push("--no-edit"),
            MergeMode::NoFastForward => args.extend(["--no-ff", "--no-edit"]),
            MergeMode::FastForwardOnly => args.push("--ff-only"),
            MergeMode::Squash => args.push("--squash"),
        }
        if let (Some(message), MergeMode::NoFastForward) = (message, mode) {
            args.extend(["-m", message]);
        }
        args.extend(["--", &target]);
        if let Err(error) = run_git_seq(path, &args) {
            if mode == MergeMode::Squash
                && !repo.path().join("SQUASH_MSG").exists()
                && !repo.index().map_err(|e| e.to_string())?.has_conflicts()
            {
                clear_squash_state(&repo)?;
            }
            return Err(error.user_message());
        }
        if mode != MergeMode::Squash {
            return Ok(());
        }
        // `merge --squash` only stages the result. Commit it unless the
        // branch was already merged and there is nothing to record.
        let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
        let head = repo
            .head()
            .and_then(|h| h.peel_to_tree())
            .map_err(|e| e.to_string())?;
        let staged = repo
            .diff_tree_to_index(Some(&head), None, None)
            .map_err(|e| e.to_string())?
            .deltas()
            .len();
        if staged == 0 {
            clear_squash_state(&repo)?;
            return Ok(());
        }
        let mut commit = vec!["commit"];
        match message {
            Some(message) => commit.extend(["-m", message]),
            None => commit.push("--no-edit"),
        }
        run_git_seq(path, &commit).map_err(|e| e.user_message())?;
        clear_squash_state(&repo)
    })
}

const SQUASH_STATE: &str = "recorde-squash.json";

#[derive(serde::Serialize, serde::Deserialize)]
struct SquashState {
    head: String,
    branch: Option<String>,
    message: Option<String>,
}

pub(crate) fn squash_in_progress(repo: &git2::Repository) -> bool {
    if repo.state() != git2::RepositoryState::Clean {
        return false;
    }
    if repo.path().join(SQUASH_STATE).exists() {
        return true;
    }
    // Native squash merges have no MERGE_HEAD. Keep recognizing a resolved
    // staged squash until commit consumes SQUASH_MSG.
    repo.path().join("SQUASH_MSG").is_file()
        && repo.index().is_ok_and(|index| {
            index.has_conflicts()
                || repo
                    .head()
                    .and_then(|head| head.peel_to_tree())
                    .ok()
                    .and_then(|tree| {
                        repo.diff_tree_to_index(Some(&tree), Some(&index), None)
                            .ok()
                    })
                    .is_some_and(|diff| diff.deltas().len() > 0)
        })
}

fn write_squash_state(repo: &git2::Repository, message: Option<&str>) -> Result<(), String> {
    use std::io::Write;
    let head = repo.head().map_err(|e| e.to_string())?;
    let state = SquashState {
        head: head
            .target()
            .ok_or("Squash requires an existing HEAD.")?
            .to_string(),
        branch: (!repo.head_detached().map_err(|e| e.to_string())?)
            .then(|| head.name().ok().map(str::to_owned))
            .flatten(),
        message: message.map(str::to_owned),
    };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(repo.path().join(SQUASH_STATE))
        .map_err(|e| format!("Cannot record squash operation: {e}"))?;
    file.write_all(&serde_json::to_vec(&state).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}

fn read_squash_state(repo: &git2::Repository) -> Result<Option<SquashState>, String> {
    let path = repo.path().join(SQUASH_STATE);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 1_048_576 {
        return Err("Cannot verify squash operation metadata.".into());
    }
    let state: SquashState =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let head = repo.head().map_err(|e| e.to_string())?;
    let branch = if repo.head_detached().map_err(|e| e.to_string())? {
        None
    } else {
        head.name().ok().map(str::to_owned)
    };
    if head.target().map(|oid| oid.to_string()).as_deref() != Some(state.head.as_str())
        || branch != state.branch
    {
        return Err(
            "HEAD or branch changed since the squash began; preserve your work before recovering."
                .into(),
        );
    }
    Ok(Some(state))
}

fn clear_squash_state(repo: &git2::Repository) -> Result<(), String> {
    for name in [SQUASH_STATE, "SQUASH_MSG"] {
        match std::fs::remove_file(repo.path().join(name)) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

pub(crate) fn finish_squash(path: &Path, abort: bool) -> Result<(), String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    if !squash_in_progress(&repo) {
        return Err("No squash merge is in progress.".into());
    }
    let state = read_squash_state(&repo)?;
    if abort {
        // HEAD has not moved in a squash. reset --merge preserves unrelated
        // unstaged and untracked files and refuses unsafe overwrites.
        run_git_seq(path, &["reset", "--merge", "HEAD"]).map_err(|e| e.user_message())?;
    } else {
        if repo.index().map_err(|e| e.to_string())?.has_conflicts() {
            return Err("Resolve and stage every squash conflict before continuing.".into());
        }
        let tree = repo
            .head()
            .and_then(|head| head.peel_to_tree())
            .map_err(|e| e.to_string())?;
        if repo
            .diff_tree_to_index(Some(&tree), None, None)
            .map_err(|e| e.to_string())?
            .deltas()
            .len()
            > 0
        {
            let mut args = vec!["commit"];
            match state.as_ref().and_then(|state| state.message.as_deref()) {
                Some(message) => args.extend(["-m", message]),
                None => args.push("--no-edit"),
            }
            run_git_seq(path, &args).map_err(|e| e.user_message())?;
        }
    }
    clear_squash_state(&repo)
}

/// Move a local branch that is not checked out forward to its upstream.
/// Refuses unless the branch can move forward without merging.
pub fn fast_forward_branch(branch: &str, state: SharedState) {
    let name = branch.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    {
        let mut s = state.lock().unwrap();
        s.remote_op = Some(format!("Fast-forwarding {name}…"));
        s.console_log.push(format!("$ fast-forward {name}"));
    }
    crate::git::mutation::spawn_operation(path, state, move |path| {
        do_fast_forward_branch(path, &name).map(|_| ())
    });
}

/// Returns the new tip.
pub(crate) fn do_fast_forward_branch(path: &Path, branch: &str) -> Result<String, String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let local = repo
        .find_branch(branch, git2::BranchType::Local)
        .map_err(|_| format!("Branch {branch} does not exist."))?;
    if local.is_head() {
        return Err(format!(
            "{branch} is checked out. Pull to fast-forward the current branch."
        ));
    }
    let reference = format!("refs/heads/{branch}");
    ensure_branch_not_checked_out(path, &reference)?;
    let upstream = local
        .upstream()
        .map_err(|_| format!("{branch} has no upstream branch. Set one first."))?;
    let old = local.get().target().ok_or("Branch has no commit.")?;
    let new = upstream.get().target().ok_or("Upstream has no commit.")?;
    if old == new {
        return Err(format!("{branch} is already up to date."));
    }
    if !repo
        .graph_descendant_of(new, old)
        .map_err(|e| e.to_string())?
    {
        return Err(format!(
            "{branch} has commits that are not on its upstream. Check it out and pull or rebase."
        ));
    }
    let new = new.to_string();
    ensure_branch_not_checked_out(path, &reference)?;
    run_git_seq(
        path,
        &[
            "update-ref",
            "-m",
            "fast-forward",
            &reference,
            &new,
            &old.to_string(),
        ],
    )
    .map_err(|e| e.user_message())?;
    Ok(new)
}

/// Includes the main worktree even when called from a linked worktree. Ref
/// updates do not perform this check themselves. Unverifiable/prunable entries
/// block the operation instead of being silently treated as detached.
pub(crate) fn ensure_branch_not_checked_out(path: &Path, reference: &str) -> Result<(), String> {
    let output = super::process::git(
        path,
        &["worktree", "list", "--porcelain", "-z"],
        &std::sync::atomic::AtomicBool::new(false),
    )
    .map_err(|e| e.user_message())?;
    if output.stdout.len() >= 1_048_576 || !output.stdout.ends_with(&[0, 0]) {
        return Err("Cannot verify the complete worktree listing.".into());
    }
    let target = format!("branch {reference}");
    for record in output
        .stdout
        .split(|byte| *byte == 0)
        .collect::<Vec<_>>()
        .split(|field| field.is_empty())
    {
        if record.is_empty() {
            continue;
        }
        let root = record[0]
            .strip_prefix(b"worktree ")
            .ok_or("Invalid worktree ownership record.")?;
        if record.iter().any(|field| *field == target.as_bytes()) {
            return Err(format!("{reference} is checked out in another worktree."));
        }
        if record.iter().any(|field| field.starts_with(b"prunable")) {
            return Err(
                "A worktree is unavailable; repair or prune it before moving branch refs.".into(),
            );
        }
        let root =
            std::str::from_utf8(root).map_err(|_| "Cannot verify a non-UTF-8 worktree path.")?;
        let worktree =
            git2::Repository::open(root).map_err(|_| format!("Cannot verify worktree {root}."))?;
        let head = worktree
            .find_reference("HEAD")
            .map_err(|_| format!("Cannot verify HEAD in worktree {root}."))?;
        if head
            .symbolic_target()
            .map_err(|_| format!("Cannot read HEAD in worktree {root}."))?
            == Some(reference)
        {
            return Err(format!("{reference} is checked out in worktree {root}."));
        }
    }
    Ok(())
}

pub fn continue_merge(state: SharedState) {
    seq_op("merge --continue", state, |path| {
        let repo = git2::Repository::open(path)?;
        if squash_in_progress(&repo) {
            return finish_squash(path, false).map_err(|stderr| GitError::Command {
                cmd: "Continue squash merge".into(),
                stderr,
            });
        }
        run_git_seq(path, &["merge", "--continue"])
    });
}

pub fn abort_merge(state: SharedState) {
    seq_op("merge --abort", state, |path| {
        let repo = git2::Repository::open(path)?;
        if squash_in_progress(&repo) {
            return finish_squash(path, true).map_err(|stderr| GitError::Command {
                cmd: "Abort squash merge".into(),
                stderr,
            });
        }
        run_git_seq(path, &["merge", "--abort"])
    });
}

// ── Shared pattern for continue/abort/skip ops ────────────────────────────────

pub(crate) fn seq_op_pub(
    label: &'static str,
    state: SharedState,
    f: impl FnOnce(&Path) -> Result<(), GitError> + Send + 'static,
) {
    seq_op(label, state, f);
}

fn seq_op(
    label: &'static str,
    state: SharedState,
    f: impl FnOnce(&Path) -> Result<(), GitError> + Send + 'static,
) {
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    {
        let mut s = state.lock().unwrap();
        s.remote_op = Some(format!("{}…", label));
        s.console_log.push(format!("$ git {}", label));
    }
    crate::git::mutation::spawn_operation(path, state, move |path| {
        if matches!(label, "cherry-pick" | "revert") {
            super::recovery::protect_sequence(path, label, || f(path).map_err(|e| e.user_message()))
        } else {
            let ignored = if label.starts_with("merge") || label.starts_with("rebase") {
                Some(super::ignored::SequenceGuard::prepare(path)?)
            } else {
                None
            };
            super::recovery::continue_sequence(path, || {
                if let Some(ignored) = &ignored {
                    ignored.validate()?;
                }
                f(path).map_err(|e| e.user_message())
            })
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::TestRepo;

    /// main has `base`; topic adds one commit on top of it.
    fn topic() -> (TestRepo, git2::Oid, git2::Oid) {
        let repo = TestRepo::new();
        let base = repo.commit("base.txt", b"base\n");
        repo.git_ok(&["checkout", "-b", "topic"]);
        let work = repo.commit("topic.txt", b"topic\n");
        repo.git_ok(&["checkout", "main"]);
        (repo, base, work)
    }

    fn head_commit(repo: &TestRepo) -> git2::Oid {
        repo.head()
    }

    fn squash_conflict() -> (TestRepo, git2::Oid) {
        let repo = TestRepo::new();
        repo.commit("conflict.txt", b"base\n");
        repo.git_ok(&["checkout", "-b", "topic"]);
        repo.commit("conflict.txt", b"topic\n");
        repo.git_ok(&["checkout", "main"]);
        let head = repo.commit("conflict.txt", b"main\n");
        repo.write("untracked.txt", b"preserve\n");
        assert!(do_merge(
            repo.path(),
            "topic",
            MergeMode::Squash,
            Some("Reviewed squash message")
        )
        .is_err());
        assert!(squash_in_progress(&repo.open()));
        assert!(repo.open().index().unwrap().has_conflicts());
        assert!(!repo.open().path().join("MERGE_HEAD").exists());
        assert!(super::super::recovery::undo_history(repo.path())
            .next(false)
            .is_none());
        (repo, head)
    }

    #[test]
    fn squash_conflict_continue_finishes_original_snapshot_and_supports_undo_redo() {
        let (repo, head) = squash_conflict();
        let capabilities = super::super::capabilities::detect(repo.path());
        assert!(capabilities.merge_in_progress && capabilities.squash_merge_in_progress);
        let count = super::super::recovery::list(repo.path()).len();
        assert!(finish_squash(repo.path(), false).is_err());
        repo.write("conflict.txt", b"resolved\n");
        repo.git_ok(&["add", "conflict.txt"]);
        assert!(
            squash_in_progress(&repo.open()),
            "resolved squash stays in progress"
        );
        super::super::recovery::continue_sequence(repo.path(), || {
            finish_squash(repo.path(), false)
        })
        .unwrap();
        assert!(!squash_in_progress(&repo.open()));
        assert_eq!(super::super::recovery::list(repo.path()).len(), count);
        let git = repo.open();
        let commit = git.find_commit(repo.head()).unwrap();
        assert_eq!(commit.parent_count(), 1);
        assert_eq!(commit.parent_id(0).unwrap(), head);
        assert_eq!(
            commit.summary().ok().flatten(),
            Some("Reviewed squash message")
        );
        let (id, label) = super::super::recovery::undo_history(repo.path())
            .next(false)
            .unwrap();
        assert_eq!(label, "Undo Squash merge");
        super::super::recovery::restore_step(repo.path(), &id, false).unwrap();
        assert_eq!(repo.head(), head);
        assert_eq!(
            std::fs::read(repo.path().join("conflict.txt")).unwrap(),
            b"main\n"
        );
        let (id, _) = super::super::recovery::undo_history(repo.path())
            .next(true)
            .unwrap();
        super::super::recovery::restore_step(repo.path(), &id, true).unwrap();
        assert_eq!(
            std::fs::read(repo.path().join("conflict.txt")).unwrap(),
            b"resolved\n"
        );
        assert_eq!(
            std::fs::read(repo.path().join("untracked.txt")).unwrap(),
            b"preserve\n"
        );
    }

    #[test]
    fn squash_abort_clears_conflicts_and_finalizes_unchanged_snapshot() {
        let (repo, head) = squash_conflict();
        super::super::recovery::continue_sequence(repo.path(), || finish_squash(repo.path(), true))
            .unwrap();
        assert_eq!(repo.head(), head);
        assert!(!repo.open().index().unwrap().has_conflicts());
        assert_eq!(repo.index_bytes("conflict.txt"), b"main\n");
        assert_eq!(
            std::fs::read(repo.path().join("conflict.txt")).unwrap(),
            b"main\n"
        );
        assert_eq!(
            std::fs::read(repo.path().join("untracked.txt")).unwrap(),
            b"preserve\n"
        );
        assert!(!squash_in_progress(&repo.open()));
        assert!(super::super::recovery::list(repo.path())
            .last()
            .unwrap()
            .after_state
            .is_some());
        assert!(super::super::recovery::undo_history(repo.path())
            .next(false)
            .is_none());
    }

    #[test]
    fn squash_resolution_to_ours_can_complete_without_an_empty_commit() {
        let (repo, head) = squash_conflict();
        repo.write("conflict.txt", b"main\n");
        repo.git_ok(&["add", "conflict.txt"]);
        assert!(squash_in_progress(&repo.open()));
        super::super::recovery::continue_sequence(repo.path(), || {
            finish_squash(repo.path(), false)
        })
        .unwrap();
        assert_eq!(repo.head(), head);
        assert!(!squash_in_progress(&repo.open()));
        assert!(super::super::recovery::undo_history(repo.path())
            .next(false)
            .is_none());
    }

    #[test]
    fn squash_abort_refuses_a_changed_head_without_touching_files() {
        let (repo, _) = squash_conflict();
        repo.git_ok(&["update-ref", "refs/heads/main", "topic"]);
        let before = std::fs::read(repo.path().join("conflict.txt")).unwrap();
        assert!(finish_squash(repo.path(), true)
            .unwrap_err()
            .contains("HEAD or branch"));
        assert_eq!(
            std::fs::read(repo.path().join("conflict.txt")).unwrap(),
            before
        );
        assert!(repo.open().index().unwrap().has_conflicts());
    }

    #[test]
    fn squash_commit_hook_failure_keeps_message_and_original_sequence_for_retry() {
        let (repo, base, _) = topic();
        let hook = repo.path().join("empty-hooks/pre-commit");
        std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert!(do_merge(
            repo.path(),
            "topic",
            MergeMode::Squash,
            Some("Retained message")
        )
        .is_err());
        assert_eq!(repo.head(), base);
        assert!(squash_in_progress(&repo.open()));
        assert!(super::super::recovery::undo_history(repo.path())
            .next(false)
            .is_none());
        std::fs::remove_file(hook).unwrap();
        super::super::recovery::continue_sequence(repo.path(), || {
            finish_squash(repo.path(), false)
        })
        .unwrap();
        let git = repo.open();
        let commit = git.find_commit(repo.head()).unwrap();
        assert_eq!(commit.summary().ok().flatten(), Some("Retained message"));
        assert_eq!(commit.parent_id(0).unwrap(), base);
        assert!(super::super::recovery::undo_history(repo.path())
            .next(false)
            .is_some());
    }

    #[test]
    fn fast_forward_from_linked_worktree_refuses_main_worktree_branch() {
        let (repo, base, _) = topic();
        repo.git_ok(&["branch", "--set-upstream-to=topic", "main"]);
        let directory = tempfile::tempdir().unwrap();
        let linked = directory.path().join("linked");
        repo.git_ok(&[
            "worktree",
            "add",
            "-b",
            "linked",
            linked.to_str().unwrap(),
            &base.to_string(),
        ]);
        repo.write("base.txt", b"local staged work\n");
        repo.git_ok(&["add", "base.txt"]);
        repo.write("base.txt", b"local unstaged work\n");
        assert!(do_fast_forward_branch(&linked, "main")
            .unwrap_err()
            .contains("checked out"));
        assert_eq!(repo.head(), base);
        assert_eq!(repo.index_bytes("base.txt"), b"local staged work\n");
        assert_eq!(
            std::fs::read(repo.path().join("base.txt")).unwrap(),
            b"local unstaged work\n"
        );
    }

    #[test]
    fn fast_forward_all_worktree_check_allows_detached_but_refuses_missing_worktrees() {
        let (repo, base, work) = topic();
        repo.git_ok(&["branch", "stale", &base.to_string()]);
        repo.git_ok(&["branch", "--set-upstream-to=topic", "stale"]);
        let directory = tempfile::tempdir().unwrap();
        let linked = directory.path().join("detached");
        repo.git_ok(&[
            "worktree",
            "add",
            "--detach",
            linked.to_str().unwrap(),
            &base.to_string(),
        ]);
        assert_eq!(
            do_fast_forward_branch(repo.path(), "stale").unwrap(),
            work.to_string()
        );
        repo.git_ok(&["update-ref", "refs/heads/stale", &base.to_string()]);
        std::fs::rename(&linked, directory.path().join("moved")).unwrap();
        assert!(do_fast_forward_branch(repo.path(), "stale").is_err());
        assert_eq!(
            repo.open()
                .find_reference("refs/heads/stale")
                .unwrap()
                .target(),
            Some(base)
        );
    }

    #[test]
    fn merge_modes_fast_forward_create_commit_or_squash() {
        let (repo, _, work) = topic();
        do_merge(repo.path(), "topic", MergeMode::Default, None).unwrap();
        assert_eq!(head_commit(&repo), work, "fast-forwards when possible");

        let (repo, base, work) = topic();
        do_merge(
            repo.path(),
            "topic",
            MergeMode::NoFastForward,
            Some("Merge topic work"),
        )
        .unwrap();
        let git = repo.open();
        let merge = git.find_commit(head_commit(&repo)).unwrap();
        assert_eq!(merge.parent_ids().collect::<Vec<_>>(), [base, work]);
        assert_eq!(merge.summary().ok().flatten(), Some("Merge topic work"));

        let (repo, base, work) = topic();
        do_merge(
            repo.path(),
            "topic",
            MergeMode::Squash,
            Some("Squashed topic"),
        )
        .unwrap();
        let git = repo.open();
        let squash = git.find_commit(head_commit(&repo)).unwrap();
        assert_eq!(squash.parent_ids().collect::<Vec<_>>(), [base]);
        assert_eq!(squash.summary().ok().flatten(), Some("Squashed topic"));
        assert_ne!(squash.id(), work);
        assert!(repo.path().join("topic.txt").exists());
        assert_eq!(git.state(), git2::RepositoryState::Clean);
        // Squashing an already merged branch has nothing to commit.
        do_merge(repo.path(), "topic", MergeMode::Squash, None).unwrap();
        assert_eq!(head_commit(&repo), squash.id());
    }

    #[test]
    fn fast_forward_only_refuses_diverged_history_without_changes() {
        let (repo, _, _) = topic();
        let main = repo.commit("main.txt", b"main\n");
        let before = crate::git::test_support::snapshot(repo.path());
        assert!(do_merge(repo.path(), "topic", MergeMode::FastForwardOnly, None).is_err());
        assert_eq!(crate::git::test_support::snapshot(repo.path()), before);
        assert_eq!(head_commit(&repo), main);
        assert_eq!(repo.open().state(), git2::RepositoryState::Clean);
        // A successful merge can be undone as one step.
        do_merge(repo.path(), "topic", MergeMode::NoFastForward, None).unwrap();
        let history = crate::git::recovery::undo_history(repo.path());
        let (id, label) = history.next(false).unwrap();
        assert_eq!(label, "Undo Merge");
        crate::git::recovery::restore_step(repo.path(), &id, false).unwrap();
        assert_eq!(head_commit(&repo), main);
    }

    #[test]
    fn branch_fast_forward_moves_only_a_branch_that_is_behind() {
        let (repo, base, work) = topic();
        // `stale` tracks topic but is still at base.
        repo.git_ok(&["branch", "stale", &base.to_string()]);
        repo.git_ok(&["branch", "--set-upstream-to=topic", "stale"]);
        assert_eq!(
            do_fast_forward_branch(repo.path(), "stale").unwrap(),
            work.to_string()
        );
        let git = repo.open();
        let tip = |name: &str| {
            git.find_branch(name, git2::BranchType::Local)
                .unwrap()
                .get()
                .target()
                .unwrap()
        };
        assert_eq!(tip("stale"), work);
        assert!(do_fast_forward_branch(repo.path(), "stale")
            .unwrap_err()
            .contains("up to date"));

        // Diverged: the branch keeps its own commit.
        repo.git_ok(&["checkout", "stale"]);
        let own = repo.commit("own.txt", b"own\n");
        repo.git_ok(&["checkout", "main"]);
        repo.git_ok(&["checkout", "topic"]);
        repo.commit("more.txt", b"more\n");
        repo.git_ok(&["checkout", "main"]);
        assert!(do_fast_forward_branch(repo.path(), "stale").is_err());
        assert_eq!(tip("stale"), own);

        // The checked-out branch and branches without upstream are refused.
        assert!(do_fast_forward_branch(repo.path(), "main").is_err());
        assert!(do_fast_forward_branch(repo.path(), "topic").is_err());
        assert_eq!(head_commit(&repo), base);
    }
}
