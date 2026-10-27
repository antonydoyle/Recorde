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
    /// The checkpoint this one undid or redid; absent for ordinary operations.
    #[serde(default)]
    pub reverses: Option<String>,
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
    if repo.state() == git2::RepositoryState::Clean && !super::merge::squash_in_progress(repo) {
        return Ok(None);
    }
    let mut digest = Sha256::new();
    for name in [
        "MERGE_HEAD",
        "SQUASH_MSG",
        "recorde-squash.json",
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
        state: if super::merge::squash_in_progress(repo) {
            "Squash".into()
        } else {
            format!("{:?}", repo.state())
        },
        control: format!("{:x}", digest.finalize()),
    }))
}

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TrackedState {
    index: String,
    worktree: String,
    #[serde(default)]
    sparse: Option<SparseState>,
}

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct SparseState {
    patterns: Vec<u8>,
    cone: bool,
    sparse_index: bool,
    #[serde(default)]
    skip_worktree: Vec<Vec<u8>>,
    #[serde(default)]
    materialized: Vec<Vec<u8>>,
}

fn sparse_state(repo: &git2::Repository) -> Result<Option<SparseState>, String> {
    use std::io::BufRead;
    let config = repo.config().map_err(|e| e.to_string())?;
    if !config.get_bool("core.sparseCheckout").unwrap_or(false) {
        return Ok(None);
    }
    let patterns = std::fs::read(repo.path().join("info/sparse-checkout"))
        .map_err(|e| format!("Cannot verify sparse checkout patterns: {e}"))?;
    let flags = git_listing(
        repo.workdir()
            .ok_or("Sparse recovery requires a worktree.")?,
        &[
            "ls-files",
            "-t",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    )?;
    let mut skip_worktree = Vec::new();
    let mut materialized = Vec::new();
    let root = repo
        .workdir()
        .ok_or("Sparse recovery requires a worktree.")?;
    for record in std::io::BufReader::new(flags.reopen().map_err(|e| e.to_string())?).split(0) {
        let record = record.map_err(|e| e.to_string())?;
        if record.len() <= 2 {
            continue;
        }
        if let Some(file) = record.strip_prefix(b"S ") {
            skip_worktree.push(file.to_vec());
        }
        let file = &record[2..];
        let name =
            std::str::from_utf8(file).map_err(|_| "Sparse recovery requires UTF-8 file paths.")?;
        if std::fs::symlink_metadata(root.join(name)).is_ok() {
            materialized.push(file.to_vec());
        }
    }
    materialized.sort();
    materialized.dedup();
    Ok(Some(SparseState {
        patterns,
        cone: config.get_bool("core.sparseCheckoutCone").unwrap_or(false),
        sparse_index: config.get_bool("index.sparse").unwrap_or(false),
        skip_worktree,
        materialized,
    }))
}

fn git_listing(path: &Path, args: &[&str]) -> Result<tempfile::NamedTempFile, String> {
    let output = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    let mut command = super::process::command(path);
    command.arg("--no-replace-objects").args(args);
    let result = super::process::run_to_file(
        &mut command,
        output.reopen().map_err(|e| e.to_string())?,
        std::time::Duration::from_secs(600),
    )
    .map_err(|e| e.user_message())?;
    if !result.status.success() {
        return Err(crate::auth::redact::text(&String::from_utf8_lossy(
            &result.stderr,
        )));
    }
    Ok(output)
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
            sparse: sparse_state(repo)?,
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
            sparse: sparse_state(repo)?,
        })
    }
}

// A separate index lets native Git apply attributes, modes and symlink rules
// without changing the user's index or HEAD. Ignored files are never added.
fn complete_state(path: &Path, repo: &git2::Repository) -> Result<TrackedState, String> {
    let sparse = sparse_state(repo)?;
    let temporary = tempfile::tempdir().map_err(|e| e.to_string())?;
    let index_path = temporary.path().join("index");
    let run = |args: &[&str]| -> Result<String, String> {
        let mut command = super::process::command(path);
        command
            .env("GIT_INDEX_FILE", &index_path)
            .arg("--no-replace-objects")
            .args(args);
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
    // Git primes an index's cached trees with the IDs it read. While
    // replacement refs are active those are the replaced objects' IDs, not
    // the entries' contents, and write-tree trusts them until a refresh
    // invalidates them. Rebuild those indexes from their entries instead.
    // Sparse indexes keep the native copy, which preserves skip-worktree bits.
    let replaced =
        sparse.is_none() && super::replacement::refs(repo).is_ok_and(|refs| !refs.is_empty());
    let index = if replaced {
        // Listings can exceed the diagnostic output limit; stream via a file.
        let listing = git_listing(path, &["ls-files", "--stage", "-z"])?;
        let mut command = super::process::command(path);
        command.env("GIT_INDEX_FILE", &index_path).args([
            "--no-replace-objects",
            "update-index",
            "-z",
            "--index-info",
        ]);
        let output = super::process::run_with_stdin(
            &mut command,
            &AtomicBool::new(false),
            std::time::Duration::from_secs(600),
            listing.reopen().map_err(|e| e.to_string())?.into(),
        )
        .map_err(|e| e.user_message())?;
        if !output.status.success() {
            return Err(crate::auth::redact::text(&String::from_utf8_lossy(
                &output.stderr,
            )));
        }
        run(&["write-tree"])?
    } else {
        let output = super::process::git(path, &["write-tree"], &AtomicBool::new(false))
            .map_err(|e| e.user_message())?;
        let index = String::from_utf8_lossy(&output.stdout).trim().to_string();
        git2::Oid::from_str(&index).map_err(|e| e.to_string())?;
        if repo.path().join("index").exists() {
            std::fs::copy(repo.path().join("index"), &index_path).map_err(|e| e.to_string())?;
        } else {
            run(&["read-tree", &index])?;
        }
        index
    };
    git2::Oid::from_str(&index).map_err(|e| e.to_string())?;
    run(&["add", "--all", "--sparse", "--", "."])?;
    // Explicit pathspecs include materialized files outside the sparse cone on
    // Git versions where a directory pathspec still applies sparse filtering.
    if let Some(sparse) = &sparse {
        if !sparse.materialized.is_empty() {
            use std::io::Write;
            let mut files = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
            for name in &sparse.materialized {
                files
                    .write_all(name)
                    .and_then(|_| files.write_all(&[0]))
                    .map_err(|e| e.to_string())?;
            }
            files.flush().map_err(|e| e.to_string())?;
            run(&[
                "--literal-pathspecs",
                "add",
                "--sparse",
                "--pathspec-file-nul",
                &format!("--pathspec-from-file={}", files.path().to_string_lossy()),
            ])?;
        }
    }
    let worktree = run(&["write-tree"])?;
    git2::Oid::from_str(&worktree).map_err(|e| e.to_string())?;
    Ok(TrackedState {
        index,
        worktree,
        sparse,
    })
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
    capture_reversing(path, label, None)
}

fn capture_reversing(
    path: &Path,
    label: &str,
    reverses: Option<&str>,
) -> Result<Option<Checkpoint>, String> {
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
        reverses: reverses.map(str::to_owned),
    };
    save(&repo, &checkpoint)?;
    Ok(Some(checkpoint))
}

pub fn protect<T>(
    path: &Path,
    label: &str,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    if super::merge::squash_in_progress(&repo) {
        return Err("Finish or abort the squash merge before starting another operation.".into());
    }
    protect_inner(path, label, false, None, operation)
}

pub fn protect_sequence<T>(
    path: &Path,
    label: &str,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    if repo.state() != git2::RepositoryState::Clean || super::merge::squash_in_progress(&repo) {
        return Err("Finish the current operation first.".into());
    }
    protect_inner(path, label, true, None, operation)
}

fn protect_inner<T>(
    path: &Path,
    label: &str,
    sequence: bool,
    reverses: Option<&str>,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let checkpoint = capture_reversing(path, label, reverses)?;
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
            if checkpoint.sequence.is_some() {
                checkpoint.after_state = None;
            }
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
    if repo.state() != git2::RepositoryState::Clean || super::merge::squash_in_progress(&repo) {
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
    let sparse = sparse_state(&repo)?;
    let saved_sparse = checkpoint
        .before_state
        .as_ref()
        .and_then(|s| s.sparse.as_ref());
    let same_selection = match (saved_sparse, sparse.as_ref()) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.patterns == b.patterns && a.cone == b.cone && a.sparse_index == b.sparse_index
        }
        _ => false,
    };
    if !same_selection {
        return Err("Sparse checkout patterns or settings changed after the snapshot. Restore the original sparse selection before recovery.".into());
    }
    let mut options = git2::StatusOptions::new();
    options.include_untracked(true).recurse_untracked_dirs(true);
    let statuses = if checkpoint.before_state.is_none() {
        Some(
            repo.statuses(Some(&mut options))
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    if checkpoint.before_state.is_none()
        && statuses.as_ref().is_some_and(|statuses| {
            statuses
                .iter()
                .any(|entry| entry.status().contains(git2::Status::WT_NEW))
        })
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
    } else if checkpoint.before_state.is_some() || statuses.as_ref().is_some_and(|s| !s.is_empty())
    {
        return Err(
            "This snapshot has no completed state check and cannot be restored automatically."
                .into(),
        );
    }
    ensure_no_untracked_collisions(&repo, checkpoint)?;
    protect_inner(
        path,
        &restore_label(checkpoint),
        false,
        Some(&checkpoint.id),
        || restore_contents(path, checkpoint),
    )
}

/// What restoring `checkpoint` does: undo its operation, or redo an undone one.
pub fn restore_label(checkpoint: &Checkpoint) -> String {
    match checkpoint.label.strip_prefix("Undo ") {
        Some(label) => format!("Redo {label}"),
        None => format!(
            "Undo {}",
            checkpoint
                .label
                .strip_prefix("Redo ")
                .unwrap_or(&checkpoint.label)
        ),
    }
}

/// Operations that can be undone and undone operations that can be redone,
/// oldest first; the next step is the last entry of each.
#[derive(Default)]
pub struct UndoHistory {
    pub undo: Vec<Checkpoint>,
    pub redo: Vec<Checkpoint>,
}

impl UndoHistory {
    /// Checkpoint ID and button label for the next undo or redo.
    pub fn next(&self, redo: bool) -> Option<(String, String)> {
        if self
            .undo
            .last()
            .into_iter()
            .chain(self.redo.last())
            .any(|entry| entry.after_state.is_none() || entry.sequence.is_some())
        {
            return None;
        }
        let stack = if redo { &self.redo } else { &self.undo };
        stack
            .last()
            .map(|checkpoint| (checkpoint.id.clone(), restore_label(checkpoint)))
    }
}

/// A completed operation that changed nothing (for example one Git refused)
/// is not worth an undo step.
fn unchanged(checkpoint: &Checkpoint) -> bool {
    checkpoint.after_state.is_some()
        && checkpoint.after_state == checkpoint.before_state
        && checkpoint.after.as_deref().unwrap_or_default() == checkpoint.before
        && checkpoint.after_branch == checkpoint.branch
}

/// Replay the journal like an editor's undo stack: a new operation clears
/// redo, undoing moves a step to redo and redoing moves it back.
pub fn undo_history(path: &Path) -> UndoHistory {
    let mut history = UndoHistory::default();
    for checkpoint in list(path).into_iter().rev() {
        if unchanged(&checkpoint) {
            continue;
        }
        let target = checkpoint.reverses.clone();
        let undone = target.as_ref().and_then(|id| {
            history
                .undo
                .iter()
                .rposition(|entry| &entry.id == id)
                .map(|index| (true, index))
                .or_else(|| {
                    history
                        .redo
                        .iter()
                        .rposition(|entry| &entry.id == id)
                        .map(|index| (false, index))
                })
        });
        match undone {
            Some((true, index)) => {
                history.undo.remove(index);
                history.redo.push(checkpoint);
            }
            Some((false, index)) => {
                history.redo.remove(index);
                history.undo.push(checkpoint);
            }
            None => {
                history.redo.clear();
                history.undo.push(checkpoint);
            }
        }
    }
    history
}

/// Restore the next undo (or redo) step, but only if it is still `id`.
/// Another operation may have saved a newer checkpoint after the user confirmed.
pub fn restore_step(path: &Path, id: &str, redo: bool) -> Result<(), String> {
    let history = undo_history(path);
    if history.next(redo).is_none() {
        return Err("Finish or recover the incomplete operation before using Undo or Redo.".into());
    }
    let stack = if redo { history.redo } else { history.undo };
    let next = stack.last().ok_or(if redo {
        "There is nothing to redo."
    } else {
        "There is nothing to undo."
    })?;
    if next.id != id {
        return Err("Another operation finished first. Choose Undo again to review it.".into());
    }
    restore(path, next)
}

fn verify_restore_state(path: &Path, checkpoint: &Checkpoint) -> Result<(), String> {
    // Taking the inverse checkpoint can run filters and take time. Recheck
    // user edits and ignored collisions after that capture, before writing.
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let (head, branch) = head_state(&repo)?;
    let expected_branch = if checkpoint.head_target_changed {
        &checkpoint.after_branch
    } else {
        &checkpoint.branch
    };
    if repo.state() != git2::RepositoryState::Clean
        || super::merge::squash_in_progress(&repo)
        || head != checkpoint.after
        || &branch != expected_branch
    {
        return Err("HEAD, branch or active operation changed while preparing recovery.".into());
    }
    if checkpoint.head_target_changed {
        verify_switch_target(path, &repo, checkpoint)?;
    }
    if let Some(expected) = &checkpoint.after_state {
        let actual = if checkpoint.before_state.is_some() {
            complete_state(path, &repo)?
        } else {
            tracked_state(path, &repo)?
        };
        if &actual != expected {
            return Err("Files or the index changed while preparing recovery. Preserve those edits before retrying.".into());
        }
    }
    ensure_no_untracked_collisions(&repo, checkpoint)
}

fn ensure_no_untracked_collisions(
    repo: &git2::Repository,
    checkpoint: &Checkpoint,
) -> Result<(), String> {
    let root = repo.workdir().ok_or("Recovery requires a worktree.")?;
    let mut index = if checkpoint.before_state.is_some() {
        git2::Index::new()
    } else {
        repo.index()
    }
    .map_err(|e| e.to_string())?;
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
        let mut invalid_name = false;
        tree.walk(git2::TreeWalkMode::PreOrder, |prefix, entry| {
            if entry.name().is_err() {
                invalid_name = true;
                return git2::TreeWalkResult::Skip;
            }
            if entry.kind() != Some(git2::ObjectType::Tree) {
                if let Ok(name) = entry.name() {
                    paths.push(format!("{prefix}{name}"));
                }
            }
            git2::TreeWalkResult::Ok
        })
        .map_err(|e| e.to_string())?;
        if invalid_name {
            return Err("Recovery cannot safely check a non-UTF-8 tree path. Restore it using native Git after preserving untracked files.".into());
        }
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
            // Checkpoint OIDs name exact physical snapshots, independent of
            // replacement refs used by the visible history.
            let mut raw_args = vec!["--no-replace-objects"];
            raw_args.extend_from_slice(args);
            super::process::git(path, &raw_args, &AtomicBool::new(false))
                .map(|_| ())
                .map_err(|e| e.user_message())
        };
        // Prepare the complete flag listing before the first index/worktree
        // mutation. Large sparse indexes must not hit the diagnostic output cap.
        let flag_files = if saved.sparse.is_some() {
            let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
            let tree = repo
                .find_tree(git2::Oid::from_str(&saved.worktree).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            for file in &saved.sparse.as_ref().unwrap().materialized {
                let name = std::str::from_utf8(file)
                    .map_err(|_| "Sparse recovery requires UTF-8 file paths.")?;
                tree.get_path(Path::new(name)).map_err(|_| "The saved sparse snapshot is missing materialized files. Recovery was refused before changing files.")?;
            }
            Some(git_listing(
                path,
                &["ls-tree", "-r", "--name-only", "-z", &saved.index],
            )?)
        } else {
            None
        };
        verify_restore_state(path, checkpoint)?;
        // Materialize only files included in the verified snapshot; ignored files
        // are excluded and collisions were checked before taking the inverse snapshot.
        run(&["read-tree", &current.worktree])?;
        run(&["read-tree", "--reset", "-u", &saved.worktree])?;
        if let Some(sparse) = &saved.sparse {
            restore_materialized(path, &saved.worktree, &sparse.materialized)?;
        }
        // Replace staged content without inspecting or writing working files:
        // materialized paths outside the cone may intentionally be unstaged.
        // Restore the witnessed skip-worktree bitmap separately and atomically.
        run(&["read-tree", "--reset", &saved.index])?;
        if let Some(sparse) = &saved.sparse {
            restore_sparse_flags(path, sparse, flag_files.as_ref().unwrap())?;
        }
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
    verify_restore_state(path, checkpoint)?;
    super::process::git(
        path,
        &[
            "--no-replace-objects",
            "reset",
            "--hard",
            &checkpoint.before,
        ],
        &AtomicBool::new(false),
    )
    .map_err(|e| e.user_message())?;
    if let Some(snapshot) = &checkpoint.snapshot {
        super::process::git(
            path,
            &[
                "--no-replace-objects",
                "stash",
                "apply",
                "--index",
                snapshot,
            ],
            &AtomicBool::new(false),
        )
        .map_err(|e| e.user_message())?;
    }
    Ok(())
}

fn restore_materialized(path: &Path, tree: &str, files: &[Vec<u8>]) -> Result<(), String> {
    use std::io::Write;
    if files.is_empty() {
        return Ok(());
    }
    let mut input = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    for file in files {
        input
            .write_all(file)
            .and_then(|_| input.write_all(&[0]))
            .map_err(|e| e.to_string())?;
    }
    input.flush().map_err(|e| e.to_string())?;
    let mut command = super::process::command(path);
    command
        .args([
            "--no-replace-objects",
            "--literal-pathspecs",
            // Git 2.55 can skip a matching sparse-directory subtree before
            // matching its children, even with --ignore-skip-worktree-bits.
            // Expand this command's index in memory; --worktree with --source
            // does not save it, and the repository's sparse config stays intact.
            "-c",
            "index.sparse=false",
            "restore",
            "--worktree",
            "--ignore-skip-worktree-bits",
            "--pathspec-file-nul",
        ])
        .arg(format!("--source={tree}"))
        .arg(format!(
            "--pathspec-from-file={}",
            input.path().to_string_lossy()
        ));
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
    Ok(())
}

fn restore_sparse_flags(
    path: &Path,
    sparse: &SparseState,
    all: &tempfile::NamedTempFile,
) -> Result<(), String> {
    use std::io::Write;
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let index = repo.path().join("index");
    let lock = repo.path().join("index.lock");
    let mut lock_file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .map_err(|e| format!("Cannot lock the index to restore sparse flags: {e}"))?;
    struct LockGuard {
        path: Option<std::path::PathBuf>,
        witness: std::fs::Metadata,
    }
    impl Drop for LockGuard {
        fn drop(&mut self) {
            let Some(path) = &self.path else { return };
            let Ok(actual) = std::fs::symlink_metadata(path) else {
                return;
            };
            let same = actual.is_file()
                && !actual.file_type().is_symlink()
                && actual.len() == self.witness.len()
                && actual.created().ok() == self.witness.created().ok()
                && actual.modified().ok() == self.witness.modified().ok();
            #[cfg(unix)]
            let same = {
                use std::os::unix::fs::MetadataExt;
                same && actual.ino() == self.witness.ino() && actual.dev() == self.witness.dev()
            };
            if same {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    let mut guard = LockGuard {
        path: Some(lock.clone()),
        witness: lock_file.metadata().map_err(|e| e.to_string())?,
    };
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let temp_index = temp.path().join("index");
    std::fs::copy(&index, &temp_index).map_err(|e| e.to_string())?;
    let mut skipped = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    for file in &sparse.skip_worktree {
        skipped
            .write_all(file)
            .and_then(|_| skipped.write_all(&[0]))
            .map_err(|e| e.to_string())?;
    }
    skipped.flush().map_err(|e| e.to_string())?;
    for (flag, input) in [("--no-skip-worktree", all), ("--skip-worktree", &skipped)] {
        let mut command = super::process::command(path);
        command
            .env("GIT_INDEX_FILE", &temp_index)
            .args(["update-index", flag, "-z", "--stdin"]);
        let result = super::process::run_with_stdin(
            &mut command,
            &AtomicBool::new(false),
            std::time::Duration::from_secs(60),
            std::fs::File::open(input.path())
                .map_err(|e| e.to_string())?
                .into(),
        )
        .map_err(|e| e.user_message())?;
        if !result.status.success() {
            return Err(crate::auth::redact::text(&String::from_utf8_lossy(
                &result.stderr,
            )));
        }
    }
    lock_file
        .write_all(&std::fs::read(temp_index).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    lock_file.sync_all().map_err(|e| e.to_string())?;
    guard.witness = lock_file.metadata().map_err(|e| e.to_string())?;
    drop(lock_file);
    std::fs::rename(&lock, &index).map_err(|e| e.to_string())?;
    guard.path = None;
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
    #[test]
    fn incomplete_checkpoint_blocks_older_undo_and_redo_without_disappearing() {
        let fixture = TestRepo::new();
        fixture.commit("file", b"before\n");
        protect(fixture.path(), "Edit", || {
            fixture.write("file", b"after\n");
            Ok(())
        })
        .unwrap();
        let (older, _) = undo_history(fixture.path()).next(false).unwrap();
        let incomplete = capture(fixture.path(), "Interrupted operation")
            .unwrap()
            .unwrap();
        let history = undo_history(fixture.path());
        assert!(history.next(false).is_none());
        assert!(history.next(true).is_none());
        assert_eq!(history.undo.last().unwrap().id, incomplete.id);
        assert!(restore_step(fixture.path(), &older, false).is_err());
        assert_eq!(
            std::fs::read(fixture.path().join("file")).unwrap(),
            b"after\n"
        );
    }
    #[test]
    fn large_tree_listing_is_complete_beyond_diagnostic_limit() {
        use std::io::Read;
        let fixture = TestRepo::new();
        let repo = fixture.open();
        let blob = repo.blob(b"same content").unwrap();
        let mut builder = repo.treebuilder(None).unwrap();
        for n in 0..6000 {
            builder
                .insert(format!("{n:06}-{}", "x".repeat(190)), blob, 0o100644)
                .unwrap();
        }
        let tree = builder.write().unwrap();
        let file = git_listing(
            fixture.path(),
            &["ls-tree", "-r", "--name-only", "-z", &tree.to_string()],
        )
        .unwrap();
        let mut bytes = Vec::new();
        file.reopen().unwrap().read_to_end(&mut bytes).unwrap();
        assert!(bytes.len() > 1024 * 1024);
        assert_eq!(bytes.iter().filter(|b| **b == 0).count(), 6000);
    }

    #[test]
    fn restore_recheck_preserves_edits_and_ignored_collisions_after_capture() {
        let fixture = TestRepo::new();
        fixture.commit("file", b"before\n");
        protect(fixture.path(), "Remove", || {
            fixture.git_ok(&["rm", "file"]);
            fixture.git_ok(&["commit", "-m", "remove"]);
            Ok(())
        })
        .unwrap();
        let checkpoint = list(fixture.path()).remove(0);
        verify_restore_state(fixture.path(), &checkpoint).unwrap();
        std::fs::write(fixture.open().path().join("info/exclude"), "file\n").unwrap();
        fixture.write("file", b"late ignored work\n");
        assert!(verify_restore_state(fixture.path(), &checkpoint)
            .unwrap_err()
            .contains("ignored path"));
        assert_eq!(
            std::fs::read(fixture.path().join("file")).unwrap(),
            b"late ignored work\n"
        );
        fixture.write("new-file", b"late untracked work\n");
        assert!(verify_restore_state(fixture.path(), &checkpoint)
            .unwrap_err()
            .contains("changed"));
    }

    #[test]
    fn recovery_refuses_non_utf8_tree_names_before_writing() {
        let fixture = TestRepo::new();
        fixture.commit("file", b"original\n");
        let mut checkpoint = capture(fixture.path(), "Non UTF8 fixture")
            .unwrap()
            .unwrap();
        checkpoint.after_state = checkpoint.before_state.clone();
        let repo = fixture.open();
        let blob = repo.blob(b"overwritten").unwrap();
        let mut builder = repo.treebuilder(None).unwrap();
        builder.insert(&b"invalid\xff"[..], blob, 0o100644).unwrap();
        checkpoint.before_state.as_mut().unwrap().worktree = builder.write().unwrap().to_string();
        assert!(ensure_no_untracked_collisions(&repo, &checkpoint)
            .unwrap_err()
            .contains("non-UTF-8"));
        assert_eq!(
            std::fs::read(fixture.path().join("file")).unwrap(),
            b"original\n"
        );
    }
    use crate::git::test_support::TestRepo;

    #[test]
    fn recovery_restores_raw_snapshot_even_when_its_tree_is_replaced() {
        let repo = TestRepo::new();
        let before = repo.commit("file", b"original\n");
        let raw_repo = repo.open();
        let tree = raw_repo.find_commit(before).unwrap().tree_id();
        let blob = raw_repo.blob(b"replacement\n").unwrap();
        let mut builder = raw_repo.treebuilder(None).unwrap();
        builder.insert("file", blob, 0o100644).unwrap();
        let replacement = builder.write().unwrap();
        repo.git_ok(&["replace", &tree.to_string(), &replacement.to_string()]);
        protect(repo.path(), "Reset using replacement", || {
            repo.git_ok(&["reset", "--hard", "HEAD"]);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            std::fs::read(repo.path().join("file")).unwrap(),
            b"replacement\n"
        );
        let checkpoint = list(repo.path()).remove(0);
        restore(repo.path(), &checkpoint).unwrap();
        assert_eq!(repo.head(), before);
        assert_eq!(
            std::fs::read(repo.path().join("file")).unwrap(),
            b"original\n"
        );
        assert_eq!(
            repo.git(&["--no-replace-objects", "show", ":file"]).stdout,
            b"original\n"
        );
        assert_eq!(
            raw_repo
                .find_reference(&format!("refs/replace/{tree}"))
                .unwrap()
                .target(),
            Some(replacement)
        );
    }

    #[test]
    fn complete_state_hashes_entries_when_a_replaced_tree_primes_the_cache() {
        let repo = TestRepo::new();
        repo.commit("file", b"original\n");
        let raw = repo.open();
        let tree = raw.head().unwrap().peel_to_tree().unwrap().id();
        let blob = raw.blob(b"replacement\n").unwrap();
        let mut builder = raw.treebuilder(None).unwrap();
        builder.insert("file", blob, 0o100644).unwrap();
        let replacement = builder.write().unwrap();
        repo.git_ok(&["replace", &tree.to_string(), &replacement.to_string()]);
        // Reset primes the index's cached tree with the replaced tree's ID.
        repo.git_ok(&["reset", "--hard", "HEAD"]);
        let state = complete_state(repo.path(), &raw).unwrap();
        assert_eq!(state.index, replacement.to_string());
        assert_eq!(state.worktree, replacement.to_string());
        repo.write("file", b"edited\n");
        let edited = complete_state(repo.path(), &raw).unwrap();
        assert_eq!(edited.index, replacement.to_string());
        assert_ne!(edited.worktree, replacement.to_string());
    }

    #[test]
    fn sparse_recovery_keeps_patterns_hidden_paths_and_staging() {
        for cone in [true, false] {
            let repo = TestRepo::new();
            std::fs::create_dir(repo.path().join("included")).unwrap();
            std::fs::create_dir(repo.path().join("excluded")).unwrap();
            repo.commit("included/file", b"base\n");
            let before = repo.commit("excluded/file", b"hidden\n");
            if cone {
                repo.git_ok(&[
                    "sparse-checkout",
                    "set",
                    "--cone",
                    "--sparse-index",
                    "included",
                ]);
            } else {
                repo.git_ok(&["sparse-checkout", "set", "--no-cone", "/included/"]);
            }
            let patterns = std::fs::read(repo.open().path().join("info/sparse-checkout")).unwrap();
            repo.write("included/file", b"staged\n");
            repo.git_ok(&["add", "included/file"]);
            repo.write("included/file", b"unstaged\n");
            protect(repo.path(), "Sparse reset", || {
                super::super::merge::run_git_seq(repo.path(), &["reset", "--hard", "HEAD"])
                    .map_err(|e| e.user_message())
            })
            .unwrap();
            let checkpoint = list(repo.path()).remove(0);
            restore(repo.path(), &checkpoint).unwrap();
            assert_eq!(repo.head(), before);
            assert_eq!(repo.git(&["show", ":included/file"]).stdout, b"staged\n");
            assert_eq!(
                std::fs::read(repo.path().join("included/file")).unwrap(),
                b"unstaged\n"
            );
            assert!(!repo.path().join("excluded/file").exists());
            assert_eq!(repo.git(&["show", ":excluded/file"]).stdout, b"hidden\n");
            assert_eq!(
                std::fs::read(repo.open().path().join("info/sparse-checkout")).unwrap(),
                patterns
            );
            assert!(repo
                .git(&["ls-files", "-t", "excluded/file"])
                .stdout
                .starts_with(b"S "));
            let undo = list(repo.path()).remove(0);
            restore(repo.path(), &undo).unwrap();
            assert!(!repo.path().join("excluded/file").exists());
            assert_eq!(
                std::fs::read(repo.path().join("included/file")).unwrap(),
                b"base\n"
            );
        }
    }

    #[test]
    fn sparse_pattern_change_rejects_recovery_before_any_mutation() {
        let repo = TestRepo::new();
        std::fs::create_dir(repo.path().join("included")).unwrap();
        repo.commit("included/file", b"base\n");
        repo.git_ok(&["sparse-checkout", "set", "--cone", "included"]);
        protect(repo.path(), "Commit", || {
            repo.git_ok(&["commit", "--allow-empty", "-m", "next"]);
            Ok(())
        })
        .unwrap();
        let checkpoint = list(repo.path()).remove(0);
        let before = repo.head();
        repo.git_ok(&["sparse-checkout", "set", "--cone", "elsewhere"]);
        let index = std::fs::read(repo.open().path().join("index")).unwrap();
        assert!(restore(repo.path(), &checkpoint).is_err());
        assert_eq!(repo.head(), before);
        assert_eq!(
            std::fs::read(repo.open().path().join("index")).unwrap(),
            index
        );
        assert!(!repo.path().join("included/file").exists());
    }

    #[test]
    fn sparse_recovery_restores_only_previously_materialized_outside_cone_files() {
        let repo = TestRepo::new();
        std::fs::create_dir(repo.path().join("included")).unwrap();
        std::fs::create_dir(repo.path().join("excluded")).unwrap();
        repo.commit("included/file", b"included\n");
        repo.commit("excluded/absent", b"remain absent\n");
        repo.commit("excluded/materialized", b"original\n");
        repo.git_ok(&[
            "sparse-checkout",
            "set",
            "--cone",
            "--sparse-index",
            "included",
        ]);
        std::fs::create_dir_all(repo.path().join("excluded")).unwrap();
        repo.write("excluded/materialized", b"outside edit\n");
        repo.write("excluded/scratch", b"outside untracked\n");
        let git_dir = repo.open().path().to_owned();
        let patterns = std::fs::read(git_dir.join("info/sparse-checkout")).unwrap();
        let config = std::fs::read(git_dir.join("config")).unwrap();
        let worktree_config = std::fs::read(git_dir.join("config.worktree")).unwrap();
        protect(repo.path(), "Sparse discard", || {
            super::super::merge::run_git_seq(repo.path(), &["reset", "--hard", "HEAD"])
                .map_err(|e| e.user_message())?;
            std::fs::remove_file(repo.path().join("excluded/scratch"))
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .unwrap();
        let saved = list(repo.path()).remove(0);
        restore(repo.path(), &saved).unwrap();
        assert_eq!(
            std::fs::read(repo.path().join("excluded/materialized")).unwrap(),
            b"outside edit\n"
        );
        assert_eq!(
            std::fs::read(repo.path().join("excluded/scratch")).unwrap(),
            b"outside untracked\n"
        );
        assert!(!repo.path().join("excluded/absent").exists());
        assert_eq!(
            std::fs::read(git_dir.join("info/sparse-checkout")).unwrap(),
            patterns
        );
        assert_eq!(std::fs::read(git_dir.join("config")).unwrap(), config);
        assert_eq!(
            std::fs::read(git_dir.join("config.worktree")).unwrap(),
            worktree_config
        );
        assert_eq!(
            repo.git(&["show", ":excluded/materialized"]).stdout,
            b"original\n"
        );
    }

    #[test]
    fn sparse_linked_recovery_does_not_change_main_worktree_or_ignored_files() {
        let main = TestRepo::new();
        std::fs::create_dir(main.path().join("included")).unwrap();
        std::fs::create_dir(main.path().join("excluded")).unwrap();
        main.commit("included/file", b"base\n");
        let original = main.commit("excluded/file", b"hidden\n");
        let root = tempfile::tempdir().unwrap();
        let linked = root.path().join("linked");
        main.git_ok(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
        let git = |args: &[&str]| {
            let mut all = vec!["-C", linked.to_str().unwrap()];
            all.extend_from_slice(args);
            main.git_ok(&all);
        };
        git(&[
            "sparse-checkout",
            "set",
            "--cone",
            "--sparse-index",
            "included",
        ]);
        std::fs::write(main.open().path().join("info/exclude"), "secret\n").unwrap();
        std::fs::write(linked.join("secret"), b"ignored\n").unwrap();
        std::fs::write(linked.join("included/file"), b"staged\n").unwrap();
        git(&["add", "included/file"]);
        std::fs::write(linked.join("included/file"), b"unstaged\n").unwrap();
        protect(&linked, "Sparse linked reset", || {
            super::super::merge::run_git_seq(&linked, &["reset", "--hard", "HEAD"])
                .map_err(|e| e.user_message())
        })
        .unwrap();
        std::fs::write(linked.join("secret"), b"new ignored work\n").unwrap();
        let saved = list(&linked).remove(0);
        restore(&linked, &saved).unwrap();
        assert_eq!(
            std::fs::read(linked.join("included/file")).unwrap(),
            b"unstaged\n"
        );
        assert_eq!(
            std::fs::read(linked.join("secret")).unwrap(),
            b"new ignored work\n"
        );
        assert!(!linked.join("excluded/file").exists());
        assert_eq!(main.head(), original);
        assert_eq!(main.index_bytes("included/file"), b"base\n");
        assert_eq!(
            std::fs::read(main.path().join("excluded/file")).unwrap(),
            b"hidden\n"
        );
        assert!(sparse_state(&main.open()).unwrap().is_none());
    }

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
    fn soft_reset(repo: &TestRepo, label: &str) {
        protect(repo.path(), label, || {
            super::super::merge::run_git_seq(repo.path(), &["reset", "--soft", "HEAD~1"])
                .map_err(|e| e.user_message())
        })
        .unwrap();
    }

    fn step(repo: &TestRepo, redo: bool) -> String {
        let (id, label) = undo_history(repo.path()).next(redo).unwrap();
        restore_step(repo.path(), &id, redo).unwrap();
        label
    }

    #[test]
    fn undo_and_redo_walk_several_steps_and_a_new_operation_clears_redo() {
        let repo = TestRepo::new();
        let first = repo.commit(
            "file.txt", b"first
",
        );
        let second = repo.commit(
            "file.txt", b"second
",
        );
        let third = repo.commit(
            "file.txt", b"third
",
        );
        assert!(restore_step(repo.path(), "missing", false).is_err());
        soft_reset(&repo, "Reset A");
        soft_reset(&repo, "Reset B");
        assert_eq!(repo.head(), first);
        // An operation Git refused leaves nothing to undo.
        protect(repo.path(), "Refused", || Err::<(), _>("refused".into())).unwrap_err();
        let history = undo_history(repo.path());
        assert_eq!(history.undo.len(), 2);
        assert!(history.redo.is_empty());

        assert_eq!(step(&repo, false), "Undo Reset B");
        assert_eq!(repo.head(), second);
        assert_eq!(step(&repo, false), "Undo Reset A");
        assert_eq!(repo.head(), third);
        assert!(undo_history(repo.path()).next(false).is_none());
        assert_eq!(step(&repo, true), "Redo Reset A");
        assert_eq!(repo.head(), second);
        assert_eq!(step(&repo, true), "Redo Reset B");
        assert_eq!(repo.head(), first);
        assert!(undo_history(repo.path()).next(true).is_none());
        assert_eq!(step(&repo, false), "Undo Reset B");
        assert_eq!(repo.head(), second);

        // A new operation after an undo discards the remaining redo step.
        let (redo_id, _) = undo_history(repo.path()).next(true).unwrap();
        soft_reset(&repo, "Reset C");
        assert_eq!(repo.head(), first);
        let history = undo_history(repo.path());
        assert!(history.redo.is_empty());
        assert!(restore_step(repo.path(), &redo_id, true).is_err());
        assert_eq!(step(&repo, false), "Undo Reset C");
        assert_eq!(repo.head(), second);
        assert_eq!(step(&repo, false), "Undo Reset A");
        assert_eq!(repo.head(), third);
    }

    #[test]
    fn undo_refuses_a_stale_confirmation() {
        let repo = TestRepo::new();
        repo.commit(
            "file.txt", b"base
",
        );
        repo.commit(
            "file.txt", b"tip
",
        );
        repo.commit(
            "file.txt", b"newest
",
        );
        soft_reset(&repo, "Soft reset");
        let (id, label) = undo_history(repo.path()).next(false).unwrap();
        assert_eq!(label, "Undo Soft reset");
        // A newer snapshot after confirmation must not be restored silently.
        soft_reset(&repo, "Mixed reset");
        let head = repo.head();
        assert!(restore_step(repo.path(), &id, false).is_err());
        assert_eq!(repo.head(), head);
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
