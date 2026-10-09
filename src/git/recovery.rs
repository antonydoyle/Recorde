//! Durable checkpoints for destructive local operations. No automatic remote undo.
use std::path::Path;
use std::sync::atomic::AtomicBool;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Checkpoint {
    pub id: String,
    pub label: String,
    pub before: String,
    pub after: Option<String>,
    pub branch: Option<String>,
    #[serde(default)]
    pub head_target_changed: bool,
    #[serde(default)]
    pub after_branch: Option<String>,
    #[serde(default)]
    sequence: Option<SequenceWitness>,
    pub snapshot: Option<String>,
    #[serde(default)]
    pub after_state: Option<TrackedState>,
    #[serde(default)]
    pub before_state: Option<TrackedState>,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct SequenceWitness {
    head: (Option<String>, Option<String>),
    state: String,
    control: String,
}

fn sequence_witness(repo: &git2::Repository) -> Result<Option<SequenceWitness>, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    if repo.state() == git2::RepositoryState::Clean {
        return Ok(None);
    }
    let mut digest = Sha256::new();
    for name in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "ORIG_HEAD",
        "sequencer/head",
        "sequencer/todo",
        "rebase-merge/orig-head",
        "rebase-merge/head-name",
        "rebase-merge/onto",
        "rebase-merge/git-rebase-todo",
        "rebase-merge/done",
        "rebase-apply/orig-head",
        "rebase-apply/head-name",
        "rebase-apply/onto",
        "rebase-apply/next",
        "rebase-apply/last",
    ] {
        digest.update(name.as_bytes());
        let target = repo.path().join(name);
        match std::fs::symlink_metadata(&target) {
            Ok(metadata)
                if metadata.is_file()
                    && !metadata.file_type().is_symlink()
                    && metadata.len() <= 8_388_608 =>
            {
                let mut bytes = Vec::new();
                std::fs::File::open(&target)
                    .map_err(|e| e.to_string())?
                    .take(8_388_609)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                if bytes.len() > 8_388_608 {
                    return Err("Sequencer control file exceeds recovery limit.".into());
                }
                digest.update([1]);
                digest.update((bytes.len() as u64).to_le_bytes());
                digest.update(bytes);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => digest.update([0]),
            _ => return Err("Cannot verify sequencer control files for recovery.".into()),
        }
    }
    Ok(Some(SequenceWitness {
        head: head_state(repo)?,
        state: format!("{:?}", repo.state()),
        control: format!("{:x}", digest.finalize()),
    }))
}

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TrackedState {
    index: String,
    worktree: String,
}

fn tracked_state(path: &Path, repo: &git2::Repository) -> Result<TrackedState, String> {
    let output = super::process::git(
        path,
        &["stash", "create", "Recorde state check"],
        &AtomicBool::new(false),
    )
    .map_err(|e| e.user_message())?;
    let oid = String::from_utf8_lossy(&output.stdout);
    if oid.trim().is_empty() {
        let tree = repo
            .head()
            .and_then(|h| h.peel_to_tree())
            .map_err(|e| e.to_string())?
            .id()
            .to_string();
        Ok(TrackedState {
            index: tree.clone(),
            worktree: tree,
        })
    } else {
        let commit = repo
            .find_commit(git2::Oid::from_str(oid.trim()).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        Ok(TrackedState {
            index: commit
                .parent(1)
                .map_err(|e| e.to_string())?
                .tree_id()
                .to_string(),
            worktree: commit.tree_id().to_string(),
        })
    }
}

// A separate index lets native Git apply attributes, modes and symlink rules
// without changing the user's index or HEAD. Ignored files are never added.
fn complete_state(path: &Path, repo: &git2::Repository) -> Result<TrackedState, String> {
    let output = super::process::git(path, &["write-tree"], &AtomicBool::new(false))
        .map_err(|e| e.user_message())?;
    let index = String::from_utf8_lossy(&output.stdout).trim().to_string();
    git2::Oid::from_str(&index).map_err(|e| e.to_string())?;
    let temporary = tempfile::tempdir().map_err(|e| e.to_string())?;
    let index_path = temporary.path().join("index");
    let run = |args: &[&str]| -> Result<String, String> {
        let mut command = super::process::command(path);
        command.env("GIT_INDEX_FILE", &index_path).args(args);
        let output = super::process::run(
            &mut command,
            &AtomicBool::new(false),
            std::time::Duration::from_secs(600),
        )
        .map_err(|e| e.user_message())?;
        if !output.status.success() {
            return Err(crate::auth::redact::text(&String::from_utf8_lossy(
                &output.stderr,
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    };
    if repo.path().join("index").exists() {
        std::fs::copy(repo.path().join("index"), &index_path).map_err(|e| e.to_string())?;
    } else {
        run(&["read-tree", &index])?;
    }
    run(&["add", "--all", "--", "."])?;
    let worktree = run(&["write-tree"])?;
    git2::Oid::from_str(&worktree).map_err(|e| e.to_string())?;
    Ok(TrackedState { index, worktree })
}

fn head_state(repo: &git2::Repository) -> Result<(Option<String>, Option<String>), String> {
    let reference = repo.find_reference("HEAD").map_err(|e| e.to_string())?;
    let branch = reference
        .symbolic_target()
        .ok()
        .flatten()
        .map(str::to_string);
    let oid = match reference.resolve() {
        Ok(head) => head.target().map(|id| id.to_string()),
        Err(e)
            if e.code() == git2::ErrorCode::NotFound
                || e.code() == git2::ErrorCode::UnbornBranch =>
        {
            None
        }
        Err(e) => return Err(e.to_string()),
    };
    Ok((oid, branch))
}

fn directory(repo: &git2::Repository) -> std::path::PathBuf {
    repo.path().join("recorde/recovery")
}
fn save(repo: &git2::Repository, checkpoint: &Checkpoint) -> Result<(), String> {
    use std::io::Write;
    let dir = directory(repo);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(&dir).map_err(|e| e.to_string())?;
    file.write_all(
        serde_json::to_string(checkpoint)
            .map_err(|e| e.to_string())?
            .as_bytes(),
    )
    .map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(dir.join(format!("{}.json", checkpoint.id)))
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn capture(path: &Path, label: &str) -> Result<Option<Checkpoint>, String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let (before, branch) = head_state(&repo)?;
    let before = before.unwrap_or_default();
    let before_state = complete_state(path, &repo)?;
    let signature = git2::Signature::now("Recorde recovery", "recovery@recorde.local")
        .map_err(|e| e.to_string())?;
    let index_tree = repo
        .find_tree(git2::Oid::from_str(&before_state.index).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let worktree = repo
        .find_tree(git2::Oid::from_str(&before_state.worktree).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let parent = if before.is_empty() {
        None
    } else {
        Some(
            repo.find_commit(git2::Oid::from_str(&before).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?,
        )
    };
    let parents: Vec<_> = parent.iter().collect();
    let index_commit = repo
        .commit(
            None,
            &signature,
            &signature,
            "Recorde saved index",
            &index_tree,
            &parents,
        )
        .map_err(|e| e.to_string())?;
    let index_commit = repo.find_commit(index_commit).map_err(|e| e.to_string())?;
    let snapshot = Some(
        repo.commit(
            None,
            &signature,
            &signature,
            "Recorde saved worktree",
            &worktree,
            &[&index_commit],
        )
        .map_err(|e| e.to_string())?
        .to_string(),
    );
    let id = format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos(),
        std::process::id()
    );
    // A named ref keeps the snapshot and its parent commits alive through GC.
    let oid =
        git2::Oid::from_str(snapshot.as_deref().unwrap_or(&before)).map_err(|e| e.to_string())?;
    repo.reference(&format!("refs/recorde/recovery/{id}"), oid, false, label)
        .map_err(|e| e.to_string())?;
    let checkpoint = Checkpoint {
        id,
        label: label.into(),
        before,
        after: None,
        branch,
        head_target_changed: false,
        after_branch: None,
        sequence: None,
        snapshot,
        after_state: None,
        before_state: Some(before_state),
        pinned: false,
    };
    save(&repo, &checkpoint)?;
    Ok(Some(checkpoint))
}

pub fn protect<T>(
    path: &Path,
    label: &str,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    protect_inner(path, label, false, operation)
}

pub fn protect_sequence<T>(
    path: &Path,
    label: &str,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    if repo.state() != git2::RepositoryState::Clean {
        return Err("Finish the current operation first.".into());
    }
    protect_inner(path, label, true, operation)
}

fn protect_inner<T>(
    path: &Path,
    label: &str,
    sequence: bool,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let checkpoint = capture(path, label)?;
    let result = operation();
    if let Some(mut checkpoint) = checkpoint {
        let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
        checkpoint.after = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .map(|oid| oid.to_string());
        checkpoint.after_state = complete_state(path, &repo).ok();
        checkpoint.after_branch = head_state(&repo)?.1;
        checkpoint.head_target_changed = checkpoint.after_branch != checkpoint.branch;
        if sequence {
            checkpoint.sequence = sequence_witness(&repo).ok().flatten();
        }
        // The mutation already happened. A journal update failure must not turn
        // a successful commit into a retryable commit failure (and retain its draft).
        if let Err(error) = save(&repo, &checkpoint) {
            log::warn!("Recovery journal could not record the completed operation: {error}");
        }
        if result.is_ok() {
            if let Err(error) = prune(path) {
                log::warn!("Recovery retention: {error}");
            }
        }
    }
    result
}

/// Complete only the checkpoint whose native operation and HEAD still match.
/// Manual conflict file edits/staging are allowed; external sequencer edits are not.
pub fn continue_sequence<T>(
    path: &Path,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let before = git2::Repository::open(path)
        .ok()
        .and_then(|r| sequence_witness(&r).ok().flatten());
    let pending = before.and_then(|before| {
        list(path)
            .into_iter()
            .find(|entry| entry.sequence.as_ref() == Some(&before))
    });
    let result = operation();
    if let Some(mut checkpoint) = pending {
        let update = (|| -> Result<(), String> {
            let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
            checkpoint.sequence = sequence_witness(&repo)?;
            let (head, branch) = head_state(&repo)?;
            checkpoint.after = head;
            checkpoint.after_branch = branch;
            checkpoint.head_target_changed = checkpoint.after_branch != checkpoint.branch;
            checkpoint.after_state = if checkpoint.sequence.is_none() {
                Some(complete_state(path, &repo)?)
            } else {
                None
            };
            save(&repo, &checkpoint)
        })();
        if let Err(error) = update {
            log::warn!("Sequencer recovery snapshot could not be finalized: {error}");
        }
    }
    result
}

pub fn list(path: &Path) -> Vec<Checkpoint> {
    let Ok(repo) = git2::Repository::open(path) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(directory(&repo)) else {
        return Vec::new();
    };
    let mut entries: Vec<Checkpoint> = entries
        .flatten()
        .filter(|e| {
            e.file_type().is_ok_and(|t| t.is_file()) && e.metadata().is_ok_and(|m| m.len() <= 65536)
        })
        .filter_map(|e| std::fs::read(e.path()).ok())
        .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
        .collect();
    entries.sort_by(|a, b| b.id.cmp(&a.id));
    entries
}

/// Disabled by default. Always retain the latest 20, pinned and incomplete entries.
pub fn prune(path: &Path) -> Result<(), String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let days = repo
        .config()
        .map_err(|e| e.to_string())?
        .get_i64("recorde.recoveryRetentionDays")
        .unwrap_or(0);
    if days <= 0 {
        return Ok(());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let age = (days as u128).saturating_mul(86_400_000_000_000);
    for checkpoint in list(path).into_iter().skip(20) {
        let created = checkpoint
            .id
            .split('-')
            .next()
            .and_then(|v| v.parse::<u128>().ok());
        if !checkpoint.pinned
            && checkpoint.after_state.is_some()
            && created.is_some_and(|time| now.saturating_sub(time) > age)
        {
            remove(path, &checkpoint)?;
        }
    }
    Ok(())
}

pub fn set_pinned(path: &Path, id: &str, pinned: bool) -> Result<(), String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let mut checkpoint = list(path)
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or("Snapshot no longer exists.")?;
    checkpoint.pinned = pinned;
    save(&repo, &checkpoint)
}

pub fn restore(path: &Path, checkpoint: &Checkpoint) -> Result<(), String> {
    if !checkpoint.before.is_empty() {
        git2::Oid::from_str(&checkpoint.before).map_err(|_| "Invalid recovery commit.")?;
    } else if checkpoint.before_state.is_none() {
        return Err("Invalid recovery commit.".into());
    }
    if let Some(snapshot) = &checkpoint.snapshot {
        git2::Oid::from_str(snapshot).map_err(|_| "Invalid recovery snapshot.")?;
    }
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    if repo.state() != git2::RepositoryState::Clean {
        return Err(
            "Finish or abort the current Git operation before restoring a snapshot.".into(),
        );
    }
    let (head, branch) = head_state(&repo)?;
    let expected_branch = if checkpoint.head_target_changed {
        &checkpoint.after_branch
    } else {
        &checkpoint.branch
    };
    if checkpoint.after != head || &branch != expected_branch {
        return Err("HEAD or branch changed after this operation. Recover its saved ref through the reflog/branch workflow instead.".into());
    }
    if checkpoint.head_target_changed {
        verify_switch_target(path, &repo, checkpoint)?;
    }
    if repo
        .config()
        .ok()
        .is_some_and(|c| c.get_bool("core.sparseCheckout").unwrap_or(false))
    {
        return Err("Disable sparse checkout before restoring a recovery snapshot.".into());
    }
    let mut options = git2::StatusOptions::new();
    options.include_untracked(true).recurse_untracked_dirs(true);
    let statuses = repo
        .statuses(Some(&mut options))
        .map_err(|e| e.to_string())?;
    if checkpoint.before_state.is_none()
        && statuses
            .iter()
            .any(|entry| entry.status().contains(git2::Status::WT_NEW))
    {
        return Err("Move or stash untracked files before restoring a snapshot.".into());
    }
    if let Some(expected) = &checkpoint.after_state {
        let actual = if checkpoint.before_state.is_some() {
            complete_state(path, &repo)?
        } else {
            tracked_state(path, &repo)?
        };
        if &actual != expected {
            return Err("Files or the index changed after this operation. Preserve those edits before recovering the saved snapshot.".into());
        }
    } else if checkpoint.before_state.is_some() || !statuses.is_empty() {
        return Err(
            "This snapshot has no completed state check and cannot be restored automatically."
                .into(),
        );
    }
    ensure_no_untracked_collisions(&repo, checkpoint)?;
    let label = match checkpoint.label.strip_prefix("Undo ") {
        Some(label) => format!("Redo {label}"),
        None => format!(
            "Undo {}",
            checkpoint
                .label
                .strip_prefix("Redo ")
                .unwrap_or(&checkpoint.label)
        ),
    };
    protect(path, &label, || restore_contents(path, checkpoint))
}

fn ensure_no_untracked_collisions(
    repo: &git2::Repository,
    checkpoint: &Checkpoint,
) -> Result<(), String> {
    let root = repo.workdir().ok_or("Recovery requires a worktree.")?;
    let mut index = repo.index().map_err(|e| e.to_string())?;
    let mut trees = Vec::new();
    if let Some(state) = &checkpoint.before_state {
        // Paths already present in the verified completed worktree may be restored.
        let current = checkpoint
            .after_state
            .as_ref()
            .ok_or("Missing completed state.")?;
        let current_tree = repo
            .find_tree(git2::Oid::from_str(&current.worktree).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        index.read_tree(&current_tree).map_err(|e| e.to_string())?;
        for oid in [&state.index, &state.worktree] {
            trees.push(
                repo.find_tree(git2::Oid::from_str(oid).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?,
            );
        }
    } else {
        for oid in std::iter::once(&checkpoint.before).chain(checkpoint.snapshot.iter()) {
            trees.push(
                repo.find_commit(git2::Oid::from_str(oid).map_err(|e| e.to_string())?)
                    .and_then(|c| c.tree())
                    .map_err(|e| e.to_string())?,
            );
        }
    }
    for tree in trees {
        let mut paths = Vec::new();
        tree.walk(git2::TreeWalkMode::PreOrder, |prefix, entry| {
            if entry.kind() != Some(git2::ObjectType::Tree) {
                if let Ok(name) = entry.name() {
                    paths.push(format!("{prefix}{name}"));
                }
            }
            git2::TreeWalkResult::Ok
        })
        .map_err(|e| e.to_string())?;
        for file in paths {
            let target = Path::new(&file);
            let mut candidate = Some(target);
            while let Some(path) = candidate.filter(|path| !path.as_os_str().is_empty()) {
                if let Ok(metadata) = std::fs::symlink_metadata(root.join(path)) {
                    if (path == target || !metadata.is_dir()) && index.get_path(path, 0).is_none() {
                        return Err(format!("Untracked or ignored path would be overwritten: {}. Move it before restoring.", path.display()));
                    }
                }
                candidate = path.parent();
            }
        }
    }
    Ok(())
}

fn restore_contents(path: &Path, checkpoint: &Checkpoint) -> Result<(), String> {
    if let Some(saved) = &checkpoint.before_state {
        let current = checkpoint
            .after_state
            .as_ref()
            .ok_or("Missing completed state.")?;
        let run = |args: &[&str]| {
            super::process::git(path, args, &AtomicBool::new(false))
                .map(|_| ())
                .map_err(|e| e.user_message())
        };
        // Materialize only files included in the verified snapshot; ignored files
        // are excluded and collisions were checked before taking the inverse snapshot.
        run(&["read-tree", &current.worktree])?;
        run(&["read-tree", "--reset", "-u", &saved.worktree])?;
        run(&["read-tree", &saved.index])?;
        let old = checkpoint
            .after
            .as_deref()
            .unwrap_or("0000000000000000000000000000000000000000");
        if checkpoint.head_target_changed {
            if let Some(branch) = &checkpoint.branch {
                run(&[
                    "symbolic-ref",
                    "-m",
                    "Recorde recovery: restore checkout",
                    "HEAD",
                    branch,
                ])?;
            } else {
                run(&[
                    "update-ref",
                    "--no-deref",
                    "-m",
                    "Recorde recovery: restore checkout",
                    "HEAD",
                    &checkpoint.before,
                    old,
                ])?;
            }
        } else if checkpoint.before.is_empty() {
            if let Some(branch) = &checkpoint.branch {
                run(&["update-ref", "-d", branch, old])?;
            }
        } else {
            run(&["update-ref", "HEAD", &checkpoint.before, old])?;
        }
        return Ok(());
    }
    super::process::git(
        path,
        &["reset", "--hard", &checkpoint.before],
        &AtomicBool::new(false),
    )
    .map_err(|e| e.user_message())?;
    if let Some(snapshot) = &checkpoint.snapshot {
        super::process::git(
            path,
            &["stash", "apply", "--index", snapshot],
            &AtomicBool::new(false),
        )
        .map_err(|e| e.user_message())?;
    }
    Ok(())
}

fn verify_switch_target(
    path: &Path,
    repo: &git2::Repository,
    checkpoint: &Checkpoint,
) -> Result<(), String> {
    if let Some(branch) = &checkpoint.branch {
        if !branch.starts_with("refs/heads/") || !git2::Reference::is_valid_name(branch) {
            return Err("Invalid saved branch name.".into());
        }
        let actual = match repo.find_reference(branch) {
            Ok(reference) => reference.target().map(|id| id.to_string()),
            Err(error) if error.code() == git2::ErrorCode::NotFound => None,
            Err(error) => return Err(error.to_string()),
        };
        if actual.as_deref().unwrap_or_default() != checkpoint.before {
            return Err("The original branch moved or was deleted. Restore its saved commit to a new branch instead.".into());
        }
        let output = super::process::git(
            path,
            &["worktree", "list", "--porcelain", "-z"],
            &AtomicBool::new(false),
        )
        .map_err(|e| e.user_message())?;
        if output.stdout.len() >= 1_048_576 {
            return Err("Worktree listing is too large to verify branch ownership.".into());
        }
        let target = format!("branch {branch}");
        if output
            .stdout
            .split(|b| *b == 0)
            .any(|field| field == target.as_bytes())
        {
            return Err("The original branch is checked out in another worktree.".into());
        }
    } else if checkpoint.before.is_empty() {
        return Err("Missing detached recovery commit.".into());
    }
    Ok(())
}

pub fn remove(path: &Path, checkpoint: &Checkpoint) -> Result<(), String> {
    if checkpoint.id.is_empty()
        || !checkpoint
            .id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'-')
    {
        return Err("Invalid recovery identifier.".into());
    }
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let reference = format!("refs/recorde/recovery/{}", checkpoint.id);
    match repo.find_reference(&reference) {
        Ok(mut reference) => {
            let expected =
                git2::Oid::from_str(checkpoint.snapshot.as_deref().unwrap_or(&checkpoint.before))
                    .map_err(|e| e.to_string())?;
            if reference.target() != Some(expected) {
                return Err("Recovery reference changed; reload snapshots before deleting.".into());
            }
            reference.delete().map_err(|e| e.to_string())?;
        }
        Err(error) if error.code() == git2::ErrorCode::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    match std::fs::remove_file(directory(&repo).join(format!("{}.json", checkpoint.id))) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::TestRepo;

    #[test]
    fn conflict_sequence_checkpoint_survives_restart_and_undo_redo() {
        let repo = TestRepo::new();
        repo.commit("file", b"base\n");
        repo.git_ok(&["checkout", "-b", "side"]);
        let picked = repo.commit("file", b"side\n");
        repo.git_ok(&["checkout", "main"]);
        let main = repo.commit("file", b"main\n");
        let run = |args: &[&str]| {
            super::super::merge::run_git_seq(repo.path(), args).map_err(|e| e.user_message())
        };
        assert!(protect_sequence(repo.path(), "cherry-pick", || run(&[
            "cherry-pick",
            &picked.to_string()
        ]))
        .is_err());
        assert!(list(repo.path())[0].sequence.is_some());
        repo.write("file", b"resolved\n");
        repo.git_ok(&["add", "file"]);
        continue_sequence(repo.path(), || run(&["cherry-pick", "--continue"])).unwrap();
        let finished = repo.head();
        let saved = list(repo.path()).remove(0);
        assert!(saved.sequence.is_none());
        assert!(saved.after_state.is_some());
        restore(repo.path(), &saved).unwrap();
        assert_eq!(repo.head(), main);
        assert_eq!(repo.index_bytes("file"), b"main\n");
        let inverse = list(repo.path()).remove(0);
        restore(repo.path(), &inverse).unwrap();
        assert_eq!(repo.head(), finished);
        assert_eq!(
            std::fs::read(repo.path().join("file")).unwrap(),
            b"resolved\n"
        );
    }

    #[test]
    fn externally_changed_sequence_does_not_finalize_an_old_checkpoint() {
        let repo = TestRepo::new();
        repo.commit("file", b"base\n");
        repo.git_ok(&["checkout", "-b", "side"]);
        let picked = repo.commit("file", b"side\n");
        repo.git_ok(&["checkout", "main"]);
        repo.commit("file", b"main\n");
        let run = |args: &[&str]| {
            super::super::merge::run_git_seq(repo.path(), args).map_err(|e| e.user_message())
        };
        assert!(protect_sequence(repo.path(), "cherry-pick", || run(&[
            "cherry-pick",
            &picked.to_string()
        ]))
        .is_err());
        let mut entry = list(repo.path()).remove(0);
        entry.sequence.as_mut().unwrap().control = "outdated operation".into();
        save(&repo.open(), &entry).unwrap();
        continue_sequence(repo.path(), || run(&["cherry-pick", "--abort"])).unwrap();
        assert!(list(repo.path())[0].after_state.is_none());
        assert!(restore(repo.path(), &list(repo.path())[0]).is_err());
    }

    #[test]
    fn checkout_recovery_restores_branch_and_dirty_index_without_moving_other_refs() {
        let repo = TestRepo::new();
        let main = repo.commit("common", b"base\n");
        repo.git_ok(&["checkout", "-b", "other"]);
        let other = repo.commit("other", b"other\n");
        repo.git_ok(&["checkout", "main"]);
        repo.write("common", b"staged\n");
        repo.git_ok(&["add", "common"]);
        repo.write("common", b"unstaged\n");
        repo.write("scratch", b"untracked\n");
        protect(repo.path(), "Switch branch", || {
            repo.git_ok(&["checkout", "other"]);
            Ok(())
        })
        .unwrap();
        let saved = list(repo.path()).remove(0);
        restore(repo.path(), &saved).unwrap();
        assert_eq!(repo.open().head().unwrap().shorthand().unwrap(), "main");
        assert_eq!(repo.head(), main);
        assert_eq!(repo.index_bytes("common"), b"staged\n");
        assert_eq!(
            std::fs::read(repo.path().join("common")).unwrap(),
            b"unstaged\n"
        );
        assert_eq!(
            std::fs::read(repo.path().join("scratch")).unwrap(),
            b"untracked\n"
        );
        assert!(!repo.path().join("other").exists());
        assert_eq!(
            repo.open()
                .find_reference("refs/heads/other")
                .unwrap()
                .target(),
            Some(other)
        );
        let undo = list(repo.path()).remove(0);
        restore(repo.path(), &undo).unwrap();
        assert_eq!(repo.open().head().unwrap().shorthand().unwrap(), "other");
        assert_eq!(repo.head(), other);
        assert_eq!(repo.index_bytes("common"), b"staged\n");
        assert_eq!(
            std::fs::read(repo.path().join("other")).unwrap(),
            b"other\n"
        );
        // A branch moved elsewhere must not be moved back by checkout recovery.
        repo.git_ok(&["branch", "-f", "main", &other.to_string()]);
        assert!(restore(repo.path(), &saved).is_err());
        assert_eq!(
            repo.open()
                .find_reference("refs/heads/main")
                .unwrap()
                .target(),
            Some(other)
        );
    }

    #[test]
    fn checkout_recovery_refuses_linked_worktree_and_restores_detached_head() {
        let repo = TestRepo::new();
        let base = repo.commit("file", b"base\n");
        repo.git_ok(&["branch", "other"]);
        protect(repo.path(), "Switch branch", || {
            repo.git_ok(&["checkout", "other"]);
            Ok(())
        })
        .unwrap();
        let saved = list(repo.path()).remove(0);
        let root = tempfile::tempdir().unwrap();
        let linked = root.path().join("linked");
        repo.git_ok(&["worktree", "add", linked.to_str().unwrap(), "main"]);
        assert!(restore(repo.path(), &saved).is_err());
        assert_eq!(repo.open().head().unwrap().shorthand().unwrap(), "other");
        repo.git_ok(&["checkout", "--detach", &base.to_string()]);
        protect(repo.path(), "Switch from detached", || {
            repo.git_ok(&["checkout", "other"]);
            Ok(())
        })
        .unwrap();
        let saved = list(repo.path()).remove(0);
        restore(repo.path(), &saved).unwrap();
        assert!(repo.open().head_detached().unwrap());
        assert_eq!(repo.head(), base);
        assert_eq!(repo.index_bytes("file"), b"base\n");
    }
    #[test]
    fn retention_keeps_recent_pinned_and_incomplete_snapshots() {
        let repo = TestRepo::new();
        repo.commit("file", b"base\n");
        let opened = repo.open();
        opened
            .config()
            .unwrap()
            .set_i64("recorde.recoveryRetentionDays", 30)
            .unwrap();
        let mut template = capture(repo.path(), "Old snapshot").unwrap().unwrap();
        template.after_state = template.before_state.clone();
        // Synthetic old identifiers avoid sleeping or changing the machine clock.
        for n in 100..124 {
            let mut entry = template.clone();
            entry.id = format!("{n}-1");
            entry.pinned = n == 100;
            if n == 101 {
                entry.after_state = None;
            }
            opened
                .reference(
                    &format!("refs/recorde/recovery/{}", entry.id),
                    git2::Oid::from_str(entry.snapshot.as_ref().unwrap()).unwrap(),
                    false,
                    "fixture",
                )
                .unwrap();
            save(&opened, &entry).unwrap();
        }
        remove(repo.path(), &template).unwrap();
        prune(repo.path()).unwrap();
        let saved = list(repo.path());
        assert_eq!(saved.len(), 22);
        assert!(saved.iter().any(|e| e.id == "100-1"));
        assert!(saved.iter().any(|e| e.id == "101-1"));
        assert!(!saved.iter().any(|e| e.id == "102-1"));
        assert!(opened
            .find_reference("refs/recorde/recovery/102-1")
            .is_err());
        assert_eq!(repo.index_bytes("file"), b"base\n");
    }
    #[test]
    fn first_commit_undo_redo_preserves_untracked_and_partially_staged_files() {
        let repo = TestRepo::new();
        repo.write("first.txt", b"staged\n");
        repo.git_ok(&["add", "first.txt"]);
        repo.write("first.txt", b"unstaged\n");
        repo.write("notes.txt", b"untracked\n");
        protect(repo.path(), "First commit", || {
            repo.git_ok(&["commit", "-m", "first"]);
            Ok(())
        })
        .unwrap();
        let committed = repo.head();
        let saved = list(repo.path()).remove(0);
        restore(repo.path(), &saved).unwrap();
        assert!(repo.open().head().is_err());
        assert_eq!(repo.index_bytes("first.txt"), b"staged\n");
        assert!(repo
            .open()
            .index()
            .unwrap()
            .get_path(Path::new("notes.txt"), 0)
            .is_none());
        assert_eq!(
            std::fs::read(repo.path().join("first.txt")).unwrap(),
            b"unstaged\n"
        );
        assert_eq!(
            std::fs::read(repo.path().join("notes.txt")).unwrap(),
            b"untracked\n"
        );
        let inverse = list(repo.path()).remove(0);
        restore(repo.path(), &inverse).unwrap();
        assert_eq!(repo.head(), committed);
        assert_eq!(
            std::fs::read(repo.path().join("notes.txt")).unwrap(),
            b"untracked\n"
        );
    }

    #[test]
    fn snapshot_restores_renames_and_deleted_untracked_files_without_collecting_ignored_files() {
        let repo = TestRepo::new();
        repo.commit("old.txt", b"tracked\n");
        std::fs::write(repo.open().path().join("info/exclude"), "secret.txt\n").unwrap();
        repo.write("secret.txt", b"ignored\n");
        repo.write("scratch.txt", b"recover me\n");
        protect(repo.path(), "Rename and discard", || {
            repo.git_ok(&["mv", "old.txt", "new.txt"]);
            std::fs::remove_file(repo.path().join("scratch.txt")).unwrap();
            Ok(())
        })
        .unwrap();
        let saved = list(repo.path()).remove(0);
        let opened = repo.open();
        let tree = opened
            .find_tree(git2::Oid::from_str(&saved.before_state.as_ref().unwrap().worktree).unwrap())
            .unwrap();
        assert!(tree.get_path(Path::new("secret.txt")).is_err());
        repo.write("secret.txt", b"later ignored work\n");
        restore(repo.path(), &saved).unwrap();
        assert!(!repo.path().join("new.txt").exists());
        assert_eq!(repo.index_bytes("old.txt"), b"tracked\n");
        assert_eq!(
            std::fs::read(repo.path().join("scratch.txt")).unwrap(),
            b"recover me\n"
        );
        assert_eq!(
            std::fs::read(repo.path().join("secret.txt")).unwrap(),
            b"later ignored work\n"
        );
    }

    #[test]
    fn recovery_refuses_changes_to_preexisting_untracked_files() {
        let repo = TestRepo::new();
        repo.commit("file.txt", b"base\n");
        repo.write("notes", b"original\n");
        protect(repo.path(), "Commit", || {
            repo.commit("file.txt", b"next\n");
            Ok(())
        })
        .unwrap();
        let after = repo.head();
        let saved = list(repo.path()).remove(0);
        repo.write("notes", b"new user work\n");
        assert!(restore(repo.path(), &saved).is_err());
        assert_eq!(repo.head(), after);
        assert_eq!(
            std::fs::read(repo.path().join("notes")).unwrap(),
            b"new user work\n"
        );
    }
    #[test]
    fn recovery_refuses_ignored_file_collision_without_changing_head() {
        let repo = TestRepo::new();
        let before = repo.commit("file.txt", b"tracked\n");
        protect(repo.path(), "Remove tracked file", || {
            repo.git_ok(&["rm", "file.txt"]);
            repo.git_ok(&["commit", "-m", "remove"]);
            Ok(())
        })
        .unwrap();
        let after = repo.head();
        assert_ne!(before, after);
        std::fs::write(repo.open().path().join("info/exclude"), "file.txt\n").unwrap();
        repo.write("file.txt", b"ignored work\n");
        let saved = list(repo.path()).remove(0);
        assert!(restore(repo.path(), &saved)
            .unwrap_err()
            .contains("ignored path"));
        assert_eq!(repo.head(), after);
        assert_eq!(
            std::fs::read(repo.path().join("file.txt")).unwrap(),
            b"ignored work\n"
        );
    }
    #[test]
    fn soft_reset_undo_redo_preserves_index_and_rejects_later_edits() {
        let repo = TestRepo::new();
        let base = repo.commit("file.txt", b"base\n");
        let tip = repo.commit("file.txt", b"tip\n");
        protect(repo.path(), "Soft reset", || {
            super::super::merge::run_git_seq(repo.path(), &["reset", "--soft", "HEAD~1"])
                .map_err(|e| e.user_message())
        })
        .unwrap();
        let saved = list(repo.path()).remove(0);
        repo.write("file.txt", b"later edit\n");
        assert!(restore(repo.path(), &saved).is_err());
        assert_eq!(
            std::fs::read(repo.path().join("file.txt")).unwrap(),
            b"later edit\n"
        );
        repo.write("file.txt", b"tip\n");
        restore(repo.path(), &saved).unwrap();
        assert_eq!(repo.head(), tip);
        let inverse = list(repo.path()).remove(0);
        assert_eq!(inverse.label, "Undo Soft reset");
        restore(repo.path(), &inverse).unwrap();
        assert_eq!(repo.head(), base);
        assert_eq!(repo.index_bytes("file.txt"), b"tip\n");
        let redo = list(repo.path()).remove(0);
        assert_eq!(redo.label, "Redo Soft reset");
        remove(repo.path(), &redo).unwrap();
        assert!(!list(repo.path()).iter().any(|entry| entry.id == redo.id));
        assert!(repo
            .open()
            .find_reference(&format!("refs/recorde/recovery/{}", redo.id))
            .is_err());
        assert_eq!(repo.head(), base);
    }
    #[test]
    fn hard_reset_recovery_restores_head_index_and_worktree_across_reload() {
        let repo = TestRepo::new();
        repo.commit("file.txt", b"base\n");
        let before = repo.commit("file.txt", b"committed\n");
        repo.write("file.txt", b"staged\n");
        repo.git_ok(&["add", "file.txt"]);
        repo.write("file.txt", b"unstaged\n");
        protect(repo.path(), "Hard reset", || {
            super::super::merge::run_git_seq(repo.path(), &["reset", "--hard", "HEAD~1"])
                .map_err(|e| e.user_message())
        })
        .unwrap();
        let saved = list(repo.path());
        assert_eq!(saved.len(), 1);
        repo.write("new.txt", b"do not overwrite");
        assert!(restore(repo.path(), &saved[0]).is_err());
        std::fs::remove_file(repo.path().join("new.txt")).unwrap();
        restore(repo.path(), &saved[0]).unwrap();
        assert_eq!(repo.head(), before);
        assert_eq!(repo.index_bytes("file.txt"), b"staged\n");
        assert_eq!(
            std::fs::read(repo.path().join("file.txt")).unwrap(),
            b"unstaged\n"
        );
    }
}
