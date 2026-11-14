use std::io::{BufRead, Read, Seek};
use std::path::Path;

use crate::git::merge::run_git_seq;
use crate::state::{RebaseAction, RebaseEntry, SharedState};

// ── Plain rebase (non-interactive) ───────────────────────────────────────────

/// Rebase the current branch onto `branch_name`.
/// Aborts with an error if the working tree is dirty.
pub fn rebase_onto(branch_name: &str, state: SharedState) {
    let name = branch_name.to_string();
    let (path, dirty) = {
        let s = state.lock().unwrap();
        (s.repo_path.clone(), s.dirty_count)
    };
    let Some(path) = path else { return };

    if dirty > 0 {
        state.lock().unwrap().error = Some("Stash or commit changes before rebasing.".into());
        return;
    }

    {
        let mut s = state.lock().unwrap();
        s.remote_op = Some(format!("Rebasing onto {}…", name));
        s.console_log.push(format!("$ git rebase {}", name));
    }

    crate::git::mutation::spawn_operation(path, state, move |path| do_rebase_onto(path, &name));
}

pub(crate) fn do_rebase_onto(path: &Path, name: &str) -> Result<(), String> {
    let ignored = super::ignored::Guard::prepare(path, &[name])?;
    let target = ignored.operand(0);
    super::recovery::protect_sequence(path, "Rebase", || {
        ignored.validate()?;
        verify_merge_resolutions(path, &target)?;
        ignored.validate()?;
        run_git_seq(
            path,
            &[
                "-c",
                "rebase.updateRefs=false",
                "rebase",
                "--rebase-merges",
                "--no-autostash",
                "--",
                &target,
            ],
        )
        .map_err(|e| e.user_message())
    })
}

pub fn continue_rebase(state: SharedState) {
    crate::git::merge::seq_op_pub("rebase --continue", state, |path| {
        verify_split_complete(path).map_err(|e| git2::Error::from_str(&e))?;
        run_git_seq(
            path,
            &["-c", "rebase.updateRefs=false", "rebase", "--continue"],
        )?;
        cleanup_finished(path);
        Ok(())
    });
}

pub fn abort_rebase(state: SharedState) {
    crate::git::merge::seq_op_pub("rebase --abort", state, |path| {
        run_git_seq(
            path,
            &["-c", "rebase.updateRefs=false", "rebase", "--abort"],
        )?;
        cleanup_finished(path);
        Ok(())
    });
}

pub fn skip_rebase(state: SharedState) {
    crate::git::merge::seq_op_pub("rebase --skip", state, |path| {
        let repo = git2::Repository::open(path)?;
        if repo.path().join("rebase-merge/recorde-split").exists() {
            return Err(git2::Error::from_str(
                "Finish and continue the split, or abort the rebase.",
            )
            .into());
        }
        run_git_seq(path, &["-c", "rebase.updateRefs=false", "rebase", "--skip"])?;
        cleanup_finished(path);
        Ok(())
    });
}

pub fn is_edit_pause(repo: &git2::Repository) -> bool {
    let dir = repo.path().join("rebase-merge");
    dir.join("amend").is_file()
        && std::fs::read_to_string(dir.join("done")).is_ok_and(|done| {
            done.lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .is_some_and(|line| line.starts_with("edit "))
        })
}

pub fn split_paused_commit(state: SharedState) {
    let Some(path) = state.lock().unwrap().repo_path.clone() else {
        return;
    };
    crate::git::mutation::spawn_operation(path, state, |path| {
        super::recovery::continue_sequence(path, || prepare_split(path))
    });
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct SplitState {
    pub base: String,
    pub original: String,
    pub tree: String,
}

pub(super) fn split_state(repo: &git2::Repository) -> Result<SplitState, String> {
    let text =
        std::fs::read(repo.path().join("rebase-merge/recorde-split")).map_err(|e| e.to_string())?;
    serde_json::from_slice(&text).map_err(|e| e.to_string())
}

fn prepare_split(path: &Path) -> Result<(), String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let dir = repo.path().join("rebase-merge");
    if !is_edit_pause(&repo) || dir.join("recorde-split").exists() {
        return Err("Split is available only at an unchanged edit stop.".into());
    }
    let head_oid = repo
        .head()
        .map_err(|e| e.to_string())?
        .target()
        .ok_or("HEAD has no commit.")?;
    let head = super::replacement::commit(&repo, head_oid).map_err(|e| e.to_string())?;
    let expected = std::fs::read_to_string(dir.join("amend")).map_err(|e| e.to_string())?;
    if expected.trim() != head_oid.to_string() || head.parent_count() != 1 {
        return Err(
            "The edited commit changed or is a merge. Reopen the edit step before splitting."
                .into(),
        );
    }
    ensure_clean_split(path)?;
    let base = head.parent_id(0).map_err(|e| e.to_string())?.to_string();
    // Persist before resetting so a cancelled reset still leaves explicit guidance.
    let split = SplitState {
        base: base.clone(),
        original: head_oid.to_string(),
        tree: super::replacement::resolve(
            head.tree_id(),
            &super::replacement::refs(&repo).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?
        .to_string(),
    };
    std::fs::write(
        dir.join("recorde-split"),
        serde_json::to_vec(&split).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    run_git_seq(path, &["reset", "--mixed", &base]).map_err(|e| e.user_message())
}

fn ensure_clean_split(path: &Path) -> Result<(), String> {
    let status = super::process::git(
        path,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
        &std::sync::atomic::AtomicBool::new(false),
    )
    .map_err(|e| e.user_message())?;
    if !status.stdout.is_empty() {
        return Err("Stage selected files or lines and commit each part. Commit all remaining changes before continuing.".into());
    }
    Ok(())
}

fn verify_split_complete(path: &Path) -> Result<(), String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let marker = repo.path().join("rebase-merge/recorde-split");
    if !marker.exists() {
        return Ok(());
    }
    let split = split_state(&repo)?;
    let base = git2::Oid::from_str(&split.base).map_err(|e| e.to_string())?;
    let head = repo
        .head()
        .and_then(|h| h.peel_to_commit())
        .map_err(|e| e.to_string())?
        .id();
    if head == base
        || !repo
            .graph_descendant_of(head, base)
            .map_err(|e| e.to_string())?
    {
        return Err("Create at least one replacement commit before continuing the split.".into());
    }
    ensure_clean_split(path)?;
    let commit = super::replacement::commit(&repo, head).map_err(|e| e.to_string())?;
    let tree = super::replacement::resolve(
        commit.tree_id(),
        &super::replacement::refs(&repo).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if tree.to_string() != split.tree {
        return Err("The split commits do not reproduce the original commit's files. Restore and commit every original change before continuing, or abort the rebase.".into());
    }
    std::fs::remove_file(marker).map_err(|e| e.to_string())?;
    Ok(())
}

// ── Interactive rebase ────────────────────────────────────────────────────────

/// Build the list of commits that would be rebased when right-clicking `upstream_oid`.
/// Returns entries ordered oldest → newest (the natural todo order).
pub fn load_rebase_entries(repo_path: &Path, upstream_oid: &str) -> Option<Vec<RebaseEntry>> {
    load_rebase_entries_checked(repo_path, upstream_oid).ok()
}

pub fn load_rebase_entries_checked(
    repo_path: &Path,
    upstream_oid: &str,
) -> Result<Vec<RebaseEntry>, String> {
    // Mutation workers already have a thread-local cancellation token. Keep it
    // when there is no explicit background-plan token to install.
    load_rebase_entries_inner(repo_path, upstream_oid, None)
}

const MAX_PLAN_COMMITS: usize = 100_000;

pub(crate) fn check_plan_cancel(cancel: &std::sync::atomic::AtomicBool) -> Result<(), String> {
    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        Err("History preparation cancelled.".into())
    } else {
        Ok(())
    }
}

pub(crate) fn load_rebase_entries_cancellable(
    repo_path: &Path,
    upstream_oid: &str,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<Vec<RebaseEntry>, String> {
    load_rebase_entries_inner(repo_path, upstream_oid, Some(cancel))
}

fn load_rebase_entries_inner(
    repo_path: &Path,
    upstream_oid: &str,
    token: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<Vec<RebaseEntry>, String> {
    let not_cancelled = std::sync::atomic::AtomicBool::new(false);
    let cancel = token.map(|token| token.as_ref()).unwrap_or(&not_cancelled);
    check_plan_cancel(cancel)?;
    let repo = git2::Repository::open(repo_path).map_err(|e| e.to_string())?;
    let upstream = git2::Oid::from_str(upstream_oid).map_err(|e| e.to_string())?;
    let replacements = super::replacement::refs(&repo).map_err(|e| e.to_string())?;

    // Machine-readable history does not belong in the capped diagnostic pipe.
    // Request one extra commit so oversized plans are refused, never truncated.
    let mut listing = tempfile::tempfile()
        .map_err(|e| format!("Cannot create a private history listing: {e}"))?;
    let mut command = super::process::command(repo_path);
    command.args([
        "rev-list",
        "--topo-order",
        "--reverse",
        &format!("--max-count={}", MAX_PLAN_COMMITS + 1),
        "HEAD",
        &format!("^{upstream}"),
    ]);
    let output_file = listing
        .try_clone()
        .map_err(|e| format!("Cannot open the private history listing: {e}"))?;
    let run = || {
        super::process::run_to_file(
            &mut command,
            output_file,
            std::time::Duration::from_secs(600),
        )
    };
    let output = match token {
        Some(token) => super::process::with_cancellation(token.clone(), run),
        None => run(),
    }
    .map_err(|e| format!("Cannot load the rebase plan: {}", e.user_message()))?;
    check_plan_cancel(cancel)?;
    if !output.status.success() {
        return Err(format!(
            "Cannot load the rebase plan: {}",
            crate::auth::redact::text(String::from_utf8_lossy(&output.stderr).trim())
        ));
    }
    listing
        .rewind()
        .map_err(|e| format!("Cannot rewind the history listing: {e}"))?;
    entries_from_reader(
        &repo,
        std::io::BufReader::new(listing),
        &replacements,
        MAX_PLAN_COMMITS,
        || check_plan_cancel(cancel),
    )
}

fn entries_from_reader(
    repo: &git2::Repository,
    mut reader: impl BufRead,
    replacements: &std::collections::BTreeMap<git2::Oid, git2::Oid>,
    limit: usize,
    check_cancel: impl Fn() -> Result<(), String>,
) -> Result<Vec<RebaseEntry>, String> {
    let mut entries = Vec::new();
    let mut line = Vec::with_capacity(42);
    loop {
        check_cancel()?;
        line.clear();
        // Full SHA-1 plus CRLF is at most 42 bytes. Malformed output must not
        // cause an unlimited allocation before the count bound can be checked.
        let count = reader
            .by_ref()
            .take(42)
            .read_until(b'\n', &mut line)
            .map_err(|e| format!("Cannot read the history listing: {e}"))?;
        if count == 0 {
            break;
        }
        if entries.len() >= limit {
            return Err(format!("This rebase plan exceeds {limit} commits. Select a more recent starting commit or a narrower range."));
        }
        let bytes = line.strip_suffix(b"\n").unwrap_or(&line);
        let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
        if bytes.len() != 40 || !bytes.iter().all(u8::is_ascii_hexdigit) {
            return Err("Git returned an invalid rebase commit list.".into());
        }
        let id = std::str::from_utf8(bytes)
            .map_err(|_| "Git returned an invalid rebase commit list.")?;
        let oid = git2::Oid::from_str(id).map_err(|e| e.to_string())?;
        let resolved = super::replacement::resolve(oid, replacements).map_err(|e| e.to_string())?;
        let c = repo.find_commit(resolved).map_err(|e| {
            format!(
                "A commit in the rebase plan could not be read: {e}. Refresh history and retry."
            )
        })?;
        entries.push(RebaseEntry {
            oid: oid.to_string(),
            message: c.message().unwrap_or("").to_string(),
            summary: c.summary().ok().flatten().unwrap_or("").to_string(),
            action: RebaseAction::Pick,
        });
    }
    check_cancel()?;

    if entries.is_empty() {
        Err("No commits to rebase between this commit and HEAD.".into())
    } else {
        Ok(entries)
    }
}

/// Prepare an interactive rebase that squashes `selected` (oldest first)
/// into its oldest commit. Returns the upstream to rebase onto and the plan.
/// The commits must be adjacent on the current branch.
pub fn squash_plan(
    repo_path: &Path,
    selected: &[String],
) -> Result<(String, Vec<RebaseEntry>), String> {
    squash_plan_cancellable(
        repo_path,
        selected,
        &std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
}

pub(crate) fn squash_plan_cancellable(
    repo_path: &Path,
    selected: &[String],
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<(String, Vec<RebaseEntry>), String> {
    check_plan_cancel(cancel)?;
    let oldest = selected.first().ok_or("Select commits to squash.")?;
    let repo = git2::Repository::open(repo_path).map_err(|e| e.to_string())?;
    let commit = git2::Oid::from_str(oldest)
        .and_then(|oid| super::replacement::commit(&repo, oid))
        .map_err(|e| e.to_string())?;
    let upstream = commit
        .parent_id(0)
        .map_err(|_| "The first commit of the repository cannot be squashed this way.")?
        .to_string();
    let mut entries = load_rebase_entries_cancellable(repo_path, &upstream, cancel)?;
    mark_squash(&mut entries, selected)?;
    check_plan_cancel(cancel)?;
    Ok((upstream, entries))
}

/// Prepare a reviewed interactive rebase which drops the selected commits.
/// Keep non-selected descendants and reject root, merge and off-branch commits.
/// No history changes until the user starts the existing rebase workflow.
pub fn drop_plan(
    repo_path: &Path,
    selected: &[String],
) -> Result<(String, Vec<RebaseEntry>), String> {
    drop_plan_cancellable(
        repo_path,
        selected,
        &std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
}

pub(crate) fn drop_plan_cancellable(
    repo_path: &Path,
    selected: &[String],
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<(String, Vec<RebaseEntry>), String> {
    check_plan_cancel(cancel)?;
    let oldest = selected.first().ok_or("Select commits to drop.")?;
    let repo = git2::Repository::open(repo_path).map_err(|e| e.to_string())?;
    let commit = git2::Oid::from_str(oldest)
        .and_then(|oid| super::replacement::commit(&repo, oid))
        .map_err(|e| e.to_string())?;
    let upstream = commit
        .parent_id(0)
        .map_err(|_| "The first commit of the repository cannot be dropped this way.")?
        .to_string();
    let mut entries = load_rebase_entries_cancellable(repo_path, &upstream, cancel)?;
    let wanted: std::collections::HashSet<_> = selected.iter().collect();
    let available: std::collections::HashSet<_> = entries.iter().map(|entry| &entry.oid).collect();
    if wanted.len() != selected.len() || !wanted.iter().all(|oid| available.contains(oid)) {
        return Err("Select distinct commits on the current branch, oldest first.".into());
    }
    // Merge topology needs the explicit merge-preserving editor. A simple graph
    // selection cannot express which branch segment should survive a dropped tip.
    let replacements = super::replacement::refs(&repo).map_err(|e| e.to_string())?;
    for entry in &mut entries {
        check_plan_cancel(cancel)?;
        let commit = repo
            .find_commit(
                super::replacement::resolve(
                    git2::Oid::from_str(&entry.oid).map_err(|e| e.to_string())?,
                    &replacements,
                )
                .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        if commit.parent_count() > 1 {
            return Err("This range contains merges. Use Interactive Rebase to review the merge topology before dropping commits.".into());
        }
        if wanted.contains(&entry.oid) {
            entry.action = RebaseAction::Drop;
        }
    }
    Ok((upstream, entries))
}

fn mark_squash(entries: &mut [RebaseEntry], selected: &[String]) -> Result<(), String> {
    let start = entries
        .iter()
        .position(|e| Some(&e.oid) == selected.first())
        .ok_or("The selected commits are not on the current branch.")?;
    let adjacent = entries
        .get(start..start + selected.len())
        .is_some_and(|run| run.iter().zip(selected).all(|(e, oid)| &e.oid == oid));
    if !adjacent {
        return Err(
            "Only adjacent commits on the current branch can be squashed. Use Interactive Rebase to reorder them first."
                .into(),
        );
    }
    for entry in &mut entries[start + 1..start + selected.len()] {
        entry.action = RebaseAction::Squash;
    }
    Ok(())
}

/// Branch-rebase callers share the same protection against losing manual merge
/// resolutions. Run before launching native Git with --rebase-merges.
pub fn verify_merge_resolutions(path: &Path, upstream: &str) -> Result<(), String> {
    let output = super::process::git(
        path,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{upstream}^{{commit}}"),
        ],
        &std::sync::atomic::AtomicBool::new(false),
    )
    .map_err(|e| e.user_message())?;
    let upstream = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let Some(entries) = load_rebase_entries(path, &upstream) else {
        let count = super::process::git(
            path,
            &["rev-list", "--count", "HEAD", &format!("^{upstream}")],
            &std::sync::atomic::AtomicBool::new(false),
        )
        .map_err(|e| e.user_message())?;
        return if String::from_utf8_lossy(&count.stdout).trim() == "0" {
            Ok(())
        } else {
            Err("Unable to verify the replay range.".into())
        };
    };
    verify_merge_entries(path, &repo, &entries)
}

fn verify_merge_entries(
    path: &Path,
    repo: &git2::Repository,
    entries: &[RebaseEntry],
) -> Result<(), String> {
    for entry in entries {
        let commit = super::replacement::commit(
            repo,
            git2::Oid::from_str(&entry.oid).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if commit.parent_count() > 2 {
            return Err("This range contains an octopus merge whose manual resolutions Git cannot verify. Exclude it from the replay range to preserve its recorded tree.".into());
        }
        if commit.parent_count() == 2 {
            let diff = super::process::git(
                path,
                &[
                    "show",
                    "--remerge-diff",
                    "--ignore-submodules=none",
                    "--format=",
                    "--no-ext-diff",
                    "--no-textconv",
                    &entry.oid,
                    "--",
                ],
                &std::sync::atomic::AtomicBool::new(false),
            )
            .map_err(|e| e.user_message())?;
            if !diff.stdout.is_empty() {
                return Err(format!("Merge {} contains manual resolution changes. Recreating it could lose those changes; replay a range that excludes this merge or retain and reapply its resolution explicitly.", &entry.oid[..7]));
            }
        }
    }
    Ok(())
}

/// Identity of the branch shown while a background plan is prepared/reviewed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanTarget {
    pub path: std::path::PathBuf,
    pub epoch: u64,
    pub reference: String,
    pub oid: String,
    replacements: std::collections::BTreeMap<git2::Oid, git2::Oid>,
}

impl PlanTarget {
    pub fn capture(path: &Path, epoch: u64) -> Result<Self, String> {
        let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
        let head = repo.head().map_err(|e| e.to_string())?;
        if !head.is_branch() {
            return Err("Check out a branch before preparing a rebase plan.".into());
        }
        let oid = head
            .peel_to_commit()
            .map_err(|e| e.to_string())?
            .id()
            .to_string();
        let replacements = super::replacement::refs(&repo).map_err(|e| e.to_string())?;
        Ok(Self {
            path: path.to_path_buf(),
            epoch,
            reference: head.name().map_err(|e| e.to_string())?.into(),
            oid,
            replacements,
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        let actual = Self::capture(&self.path, self.epoch)?;
        if actual.replacements != self.replacements {
            return Err("Replacement history changed. Reopen the rebase plan to review the current history.".into());
        }
        if actual != *self {
            return Err(
                "The branch or HEAD changed. Reopen the rebase plan to review the current history."
                    .into(),
            );
        }
        Ok(())
    }

    pub(crate) fn review_metadata(
        &self,
        upstream: &str,
        entries: &[RebaseEntry],
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<(String, bool), String> {
        self.metadata_checked(upstream, entries, || check_plan_cancel(cancel))
    }

    fn metadata_checked(
        &self,
        upstream: &str,
        entries: &[RebaseEntry],
        check_cancel: impl Fn() -> Result<(), String>,
    ) -> Result<(String, bool), String> {
        check_cancel()?;
        let repo = git2::Repository::open(&self.path).map_err(|e| e.to_string())?;
        let commit = |oid: &str| {
            let oid = git2::Oid::from_str(oid)?;
            repo.find_commit(super::replacement::resolve(oid, &self.replacements)?)
        };
        let summary = commit(upstream)
            .map_err(|e| e.to_string())?
            .summary()
            .ok()
            .flatten()
            .unwrap_or_default()
            .to_owned();
        let mut preserve = false;
        for entry in entries {
            check_cancel()?;
            preserve |= commit(&entry.oid)
                .map_err(|e| e.to_string())?
                .parent_count()
                > 1;
        }
        check_cancel()?;
        Ok((summary, preserve))
    }

    fn validate_session(&self, state: &crate::state::AppState) -> Result<(), String> {
        if state.repo_path.as_ref() != Some(&self.path) || state.repo_epoch != self.epoch {
            return Err("The repository session changed. Reopen the rebase plan.".into());
        }
        Ok(())
    }

    pub(crate) fn execute_guarded(
        &self,
        upstream: &str,
        entries: &[RebaseEntry],
        check_session: impl Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        check_session()?;
        self.validate()?;
        let ignored = super::ignored::Guard::prepare(&self.path, &[upstream])?;
        super::recovery::protect_sequence(&self.path, "Interactive rebase", || {
            check_session()?;
            self.validate()?;
            ignored.validate()?;
            execute(&self.path, upstream, entries)
        })
    }
}

/// Execute the interactive rebase in a background thread.
pub fn start_rebase(upstream_oid: &str, entries: Vec<RebaseEntry>, state: SharedState) {
    let (path, epoch) = {
        let s = state.lock().unwrap();
        (s.repo_path.clone(), s.repo_epoch)
    };
    let Some(path) = path else { return };
    match PlanTarget::capture(&path, epoch) {
        Ok(target) => start_rebase_checked(target, upstream_oid, entries, state),
        Err(error) => state.lock().unwrap().error = Some(error),
    }
}

pub fn start_rebase_checked(
    target: PlanTarget,
    upstream_oid: &str,
    entries: Vec<RebaseEntry>,
    state: SharedState,
) {
    {
        let mut s = state.lock().unwrap();
        if let Err(error) = target.validate_session(&s) {
            s.error = Some(error);
            return;
        }
    }
    let upstream = upstream_oid.to_string();

    let guard_state = state.clone();
    crate::git::mutation::spawn_operation(target.path.clone(), state, move |_| {
        target.execute_guarded(&upstream, &entries, || {
            target.validate_session(&guard_state.lock().unwrap())
        })
    });
}

pub fn validate_entries(entries: &[RebaseEntry]) -> Result<(), String> {
    if entries.is_empty() {
        return Err("There are no commits in this rebase plan.".into());
    }
    let first = entries.iter().find(|e| e.action != RebaseAction::Drop);
    if first.is_some_and(|first| matches!(first.action, RebaseAction::Squash | RebaseAction::Fixup))
    {
        return Err("The first retained commit must be pick or reword.".into());
    }
    let mut seen = std::collections::HashSet::new();
    for entry in entries {
        let oid = git2::Oid::from_str(&entry.oid).map_err(|e| e.to_string())?;
        if !seen.insert(oid) {
            return Err("A commit occurs more than once in the replay list.".into());
        }
        if matches!(entry.action, RebaseAction::Reword | RebaseAction::Squash)
            && entry.message.trim().is_empty()
        {
            return Err("Reword and squash require a commit message.".into());
        }
    }
    Ok(())
}

fn quote_shell(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn git_path(value: &str) -> String {
    #[cfg(windows)]
    {
        value.replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        value.to_string()
    }
}

fn execute(repo_path: &Path, upstream_oid: &str, entries: &[RebaseEntry]) -> Result<(), String> {
    validate_entries(entries)?;
    let upstream = git2::Oid::from_str(upstream_oid).map_err(|e| e.to_string())?;
    let repo = git2::Repository::open(repo_path).map_err(|e| e.to_string())?;
    if repo.state() != git2::RepositoryState::Clean {
        return Err("Finish the current operation before rebasing.".into());
    }
    if repo.head_detached().map_err(|e| e.to_string())? {
        return Err("Select a branch before rebasing.".into());
    }
    let expected = load_rebase_entries(repo_path, upstream_oid).ok_or("No commits to replay.")?;
    let expected_set: std::collections::HashSet<_> = expected.iter().map(|e| &e.oid).collect();
    let actual: std::collections::HashSet<_> = entries.iter().map(|e| &e.oid).collect();
    if expected_set != actual {
        return Err("History changed; reopen the rebase preview.".into());
    }
    let preserve_merges = entries.iter().any(|entry| {
        super::replacement::commit(&repo, git2::Oid::from_str(&entry.oid).unwrap())
            .is_ok_and(|c| c.parent_count() > 1)
    });
    verify_merge_entries(repo_path, &repo, entries)?;
    let ignored = super::ignored::Guard::prepare(repo_path, &[upstream_oid])?;
    let directory = repo.path().join("recorde/rebases");
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let tmp = tempfile::Builder::new()
        .prefix("replay-")
        .tempdir_in(directory)
        .map_err(|e| e.to_string())?;
    std::fs::write(tmp.path().join("recorde-owned"), b"1").map_err(|e| e.to_string())?;
    let mut edits = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        if matches!(entry.action, RebaseAction::Reword | RebaseAction::Squash) {
            let file = tmp.path().join(format!("message-{index}"));
            let witness = tmp.path().join(format!("head-{index}"));
            std::fs::write(&file, &entry.message).map_err(|e| e.to_string())?;
            let witness = quote_shell(&git_path(&witness.to_string_lossy()));
            // Skipping a conflicted pick/fixup/merge leaves HEAD at this witness.
            // Its subsequent message edit must then be a no-op, including when
            // the user runs native `git rebase --skip` outside Recorde.
            let before = format!("exec git rev-parse --verify HEAD > {witness}");
            let after = format!(
                "exec before=$(cat {witness}) && current=$(git rev-parse --verify HEAD) && if test \"$before\" != \"$current\"; then git -c core.editor=true commit --amend --file {}; fi",
                quote_shell(&git_path(&file.to_string_lossy()))
            );
            edits.push((entry.oid.clone(), before, after));
        }
    }
    let editor = tmp.path().join("editor.sh");
    std::fs::write(
        &editor,
        sequence_editor(entries, &expected, &edits, preserve_merges),
    )
    .map_err(|e| e.to_string())?;
    let mut command = super::process::command(repo_path);
    command.args([
        "-c",
        "rebase.updateRefs=false",
        "-c",
        "core.abbrev=40",
        "-c",
        "rebase.abbreviateCommands=false",
        "rebase",
        "-i",
        "--no-autostash",
        "--reapply-cherry-picks",
        "--empty=keep",
        "--reschedule-failed-exec",
        // The config override above disables updateRefs on every supported Git;
        // --no-update-refs itself was only added in Git 2.38.
        "--no-autosquash",
    ]);
    if preserve_merges {
        command.arg("--rebase-merges");
    }
    command.arg(upstream.to_string()).env(
        "GIT_SEQUENCE_EDITOR",
        format!("sh {}", quote_shell(&git_path(&editor.to_string_lossy()))),
    );
    ignored.validate()?;
    let result = super::process::run(
        &mut command,
        &std::sync::atomic::AtomicBool::new(false),
        std::time::Duration::from_secs(600),
    )
    .map_err(|e| e.user_message());
    // The persisted todo references these messages after a conflict or failed hook.
    // Keep them in the worktree's Git directory until the operation is finished.
    if repo.path().join("rebase-merge").exists() || repo.path().join("rebase-apply").exists() {
        let _ = tmp.keep();
    }
    let output = result?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Rebase stopped. Resolve conflicts or the reported error, then Continue or Abort.\n{}",
            crate::auth::redact::text(String::from_utf8_lossy(&output.stderr).trim())
        ))
    }
}

// Only delete positively identified Recorde files after the native sequencer ends.
fn cleanup_finished(path: &Path) {
    let Ok(repo) = git2::Repository::open(path) else {
        return;
    };
    if repo.state() != git2::RepositoryState::Clean {
        return;
    }
    let Ok(git_dir) = repo.path().canonicalize() else {
        return;
    };
    let Ok(root) = repo.path().join("recorde/rebases").canonicalize() else {
        return;
    };
    if !root.starts_with(&git_dir) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with("replay-")
            || !entry.file_type().is_ok_and(|t| t.is_dir())
        {
            continue;
        }
        let Ok(directory) = entry.path().canonicalize() else {
            continue;
        };
        if directory.parent() != Some(root.as_path())
            || std::fs::read(directory.join("recorde-owned"))
                .ok()
                .as_deref()
                != Some(b"1")
        {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&directory) else {
            continue;
        };
        let files: Vec<_> = files.collect();
        let safe = files.iter().all(|file| {
            file.as_ref().is_ok_and(|file| {
                let name = file.file_name();
                let name = name.to_string_lossy();
                file.file_type().is_ok_and(|t| t.is_file())
                    && (matches!(name.as_ref(), "todo" | "editor.sh" | "recorde-owned")
                        || name
                            .strip_prefix("message-")
                            .or_else(|| name.strip_prefix("head-"))
                            .is_some_and(|s| {
                                !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
                            }))
            })
        });
        if !safe {
            continue;
        }
        for file in files.into_iter().flatten() {
            let _ = std::fs::remove_file(file.path());
        }
        let _ = std::fs::remove_dir(directory);
    }
}

// Git owns the topology program. Only consecutive pick commands can be moved
// or combined; labels/resets/merges are hard boundaries. Validate the complete
// transformation before replacing Git's todo so rejection leaves HEAD untouched.
fn sequence_editor(
    entries: &[RebaseEntry],
    expected: &[RebaseEntry],
    edits: &[(String, String, String)],
    preserve_segments: bool,
) -> String {
    fn awk(value: &str) -> String {
        format!(
            "\"{}\"",
            value
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
        )
    }
    let mut program = format!(
        "BEGIN {{\npreserve_segments={};\n",
        u8::from(preserve_segments)
    );
    for (rank, entry) in entries.iter().enumerate() {
        let verb = match entry.action {
            RebaseAction::Pick | RebaseAction::Reword => "pick",
            RebaseAction::Edit => "edit",
            RebaseAction::Squash | RebaseAction::Fixup => "fixup",
            RebaseAction::Drop => "drop",
        };
        program.push_str(&format!(
            "rank[{}]={}; verb[{}]={};\n",
            awk(&entry.oid),
            rank + 1,
            awk(&entry.oid),
            awk(verb)
        ));
    }
    for (rank, entry) in expected.iter().enumerate() {
        program.push_str(&format!("original[{}]={};\n", awk(&entry.oid), rank + 1));
    }
    for (oid, before, after) in edits {
        program.push_str(&format!(
            "before[{}]={}; edit[{}]={};\n",
            awk(oid),
            awk(before),
            awk(oid),
            awk(after)
        ));
    }
    program.push_str(r#"}
function fail(message) { print message > "/dev/stderr"; failed=1 }
{
  text[NR]=$0
  if ($1 == "pick") {
    if (!($2 in rank)) fail("History changed; reopen the rebase preview.")
    if (seen[$2]++) fail("Git repeated a planned commit; reopen the rebase preview.")
    group[$2]=block; picks[block,++size[block]]=$2; oid[NR]=$2
  } else {
    block++
    if ($1 == "merge") {
      if (!($3 in rank)) fail("History changed; reopen the rebase preview.")
      if (seen[$3]++) fail("Git repeated a planned commit; reopen the rebase preview.")
      group[$3]=block; oid[NR]=$3
      if (verb[$3] != "pick") fail("Merge commits retain their topology: use Pick or Reword for a merge itself.")
    }
    block++
  }
}
END {
  for (a in rank) if (!(a in seen)) fail("Git omitted a planned commit; reopen the rebase preview.")
  for (a in rank) for (b in rank)
    if (preserve_segments && original[a] < original[b] && rank[a] > rank[b] && group[a] != group[b])
      fail("Reorder within one branch segment. Moving across a branch point or merge would change topology.")
  for (g in size) {
    for (i=1;i<=size[g];i++) for (j=i+1;j<=size[g];j++)
      if (rank[picks[g,i]] > rank[picks[g,j]]) { t=picks[g,i]; picks[g,i]=picks[g,j]; picks[g,j]=t }
    retained=0
    for (i=1;i<=size[g];i++) {
      a=picks[g,i]
      if (verb[a] == "fixup" && !retained) fail("Squash or fixup needs an earlier retained commit in the same branch segment.")
      if (verb[a] != "drop") retained=1
    }
    if (preserve_segments && !retained) fail("Retain a commit in each branch segment so dropping its tip cannot collapse merge topology.")
  }
  if (failed) exit 1
  for (line=1;line<=NR;line++) {
    a=oid[line]
    if (a != "" && text[line] ~ /^pick /) {
      g=group[a]
      if (emitted[g]++) continue
      for (i=1;i<=size[g];i++) {
        a=picks[g,i]; if (a in before) print before[a]
        print verb[a] " " a
        if (a in edit) print edit[a]
      }
    } else {
      if (a != "" && a in before) print before[a]
      print text[line]
      if (a != "" && a in edit) print edit[a]
    }
  }
}
"#);
    format!("#!/bin/sh\nset -eu\nresult=\"$1.recorde\"\ntrap 'rm -f -- \"$result\"' EXIT\nawk {} \"$1\" > \"$result\"\nmv -- \"$result\" \"$1\"\n", quote_shell(&program))
}

#[cfg(test)]
mod squash_tests {
    use super::*;
    use crate::git::test_support::TestRepo;

    #[test]
    fn dropping_selected_tip_range_returns_to_upstream() {
        let repo = TestRepo::new();
        let base = repo.commit("base", b"base");
        let first = repo.commit("first", b"first").to_string();
        let last = repo.commit("last", b"last").to_string();
        let (upstream, entries) = drop_plan(repo.path(), &[first, last]).unwrap();
        assert_eq!(upstream, base.to_string());
        assert!(entries
            .iter()
            .all(|entry| entry.action == RebaseAction::Drop));
        execute(repo.path(), &upstream, &entries).unwrap();
        assert_eq!(repo.head(), base);
        assert_eq!(repo.open().state(), git2::RepositoryState::Clean);
        assert!(!repo.path().join("first").exists());
        assert!(!repo.path().join("last").exists());
    }

    #[test]
    fn drop_plan_keeps_gaps_and_descendants_without_mutating_history() {
        let repo = TestRepo::new();
        let root = repo.commit("base", b"0\n").to_string();
        let a = repo.commit("a", b"a\n").to_string();
        let b = repo.commit("b", b"b\n").to_string();
        let c = repo.commit("c", b"c\n").to_string();
        let head = repo.head();
        let (upstream, entries) = drop_plan(repo.path(), &[a.clone(), c.clone()]).unwrap();
        assert_eq!(upstream, root);
        assert_eq!(
            entries.iter().map(|e| e.action).collect::<Vec<_>>(),
            [RebaseAction::Drop, RebaseAction::Pick, RebaseAction::Drop]
        );
        assert_eq!(entries[1].oid, b);
        assert_eq!(repo.head(), head);
        assert!(drop_plan(repo.path(), &[root]).is_err());
        assert!(drop_plan(repo.path(), &[c.clone(), a.clone()]).is_err());
        assert!(drop_plan(repo.path(), &[a.clone(), a]).is_err());
    }

    #[test]
    fn drop_plan_rejects_off_branch_commits_and_merge_ranges() {
        let repo = TestRepo::new();
        repo.commit("base", b"base");
        repo.git_ok(&["checkout", "-b", "topic"]);
        let topic = repo.commit("topic", b"topic").to_string();
        repo.git_ok(&["checkout", "main"]);
        let main = repo.commit("main", b"main").to_string();
        assert!(drop_plan(repo.path(), &[topic.clone()]).is_err());
        assert!(drop_plan(repo.path(), &[main.clone(), topic]).is_err());
        repo.git_ok(&["merge", "--no-ff", "topic", "-m", "merge topic"]);
        let head = repo.head();
        let error = drop_plan(repo.path(), &[main]).unwrap_err();
        assert!(error.contains("contains merges"));
        assert_eq!(repo.head(), head);
    }

    #[test]
    fn squash_plan_marks_adjacent_commits_and_rejects_gaps() {
        let repo = TestRepo::new();
        repo.commit("file", b"0\n");
        let a = repo.commit("file", b"1\n").to_string();
        let b = repo.commit("file", b"2\n").to_string();
        let c = repo.commit("file", b"3\n").to_string();
        let (_, entries) = squash_plan(repo.path(), &[a.clone(), b.clone()]).unwrap();
        let actions: Vec<_> = entries.iter().map(|e| e.action).collect();
        assert_eq!(
            actions,
            [RebaseAction::Pick, RebaseAction::Squash, RebaseAction::Pick]
        );
        assert_eq!(entries[2].oid, c);
        assert!(squash_plan(repo.path(), &[a.clone(), c.clone()]).is_err());
        let git = repo.open();
        let first = git.find_commit(git2::Oid::from_str(&a).unwrap()).unwrap();
        let root = first.parent_id(0).unwrap().to_string();
        assert!(squash_plan(repo.path(), &[root, a]).is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::TestRepo;

    #[test]
    fn command_config_keeps_other_branches_when_user_enables_update_refs() {
        let repo = TestRepo::new();
        let base = repo.commit("base", b"base\n");
        let first = repo.commit("one", b"one\n");
        repo.git_ok(&["branch", "keep-tip", &first.to_string()]);
        repo.commit("two", b"two\n");
        repo.git_ok(&["config", "rebase.updateRefs", "true"]);
        let mut entries = load_rebase_entries(repo.path(), &base.to_string()).unwrap();
        entries[0].action = RebaseAction::Reword;
        entries[0].message = "rewritten first commit".into();
        execute(repo.path(), &base.to_string(), &entries).unwrap();
        assert_eq!(
            repo.open().refname_to_id("refs/heads/keep-tip").unwrap(),
            first
        );
        assert_ne!(repo.head(), first);
    }

    #[test]
    fn streamed_history_listing_accepts_more_than_the_diagnostic_pipe_limit() {
        use std::io::Write;
        let repo = TestRepo::new();
        let oid = repo.commit("file", b"base");
        let line = format!("{oid}\n");
        // Reuse one real object: exercise the complete file-reading/metadata
        // pipeline without creating tens of thousands of fixture commits.
        let count = 26_000;
        let mut listing = tempfile::tempfile().unwrap();
        {
            let mut writer = std::io::BufWriter::new(&mut listing);
            for _ in 0..count {
                writer.write_all(line.as_bytes()).unwrap();
            }
            writer.flush().unwrap();
        }
        assert!(listing.metadata().unwrap().len() > 1024 * 1024);
        listing.rewind().unwrap();
        let entries = entries_from_reader(
            &repo.open(),
            std::io::BufReader::new(listing),
            &Default::default(),
            MAX_PLAN_COMMITS,
            || Ok(()),
        )
        .unwrap();
        assert_eq!(entries.len(), count);
        assert!(entries
            .iter()
            .all(|entry| entry.oid == oid.to_string() && entry.summary == "fixture commit"));
    }

    #[test]
    fn streamed_history_listing_refuses_oversized_ranges_and_bounds_each_line() {
        let repo = TestRepo::new();
        let oid = repo.commit("file", b"base");
        let listing = format!("{oid}\n{oid}\n{oid}\n");
        let error = entries_from_reader(
            &repo.open(),
            listing.as_bytes(),
            &Default::default(),
            2,
            || Ok(()),
        )
        .unwrap_err();
        assert_eq!(error, "This rebase plan exceeds 2 commits. Select a more recent starting commit or a narrower range.");
        let mut malformed = std::io::Cursor::new(vec![b'a'; 10_000]);
        let error = entries_from_reader(
            &repo.open(),
            &mut malformed,
            &Default::default(),
            MAX_PLAN_COMMITS,
            || Ok(()),
        )
        .unwrap_err();
        assert_eq!(error, "Git returned an invalid rebase commit list.");
        assert_eq!(
            malformed.position(),
            42,
            "malformed lines must not be read without a bound"
        );
        let boundary = format!("{oid}\r\n{oid}");
        assert_eq!(
            entries_from_reader(
                &repo.open(),
                boundary.as_bytes(),
                &Default::default(),
                2,
                || Ok(())
            )
            .unwrap()
            .len(),
            2
        );
    }

    #[test]
    fn streamed_history_preserves_mutation_worker_cancellation() {
        let repo = TestRepo::new();
        let base = repo.commit("file", b"base");
        let tip = repo.commit("file", b"tip");
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let error = super::super::process::with_cancellation(cancel.clone(), || {
            load_rebase_entries_checked(repo.path(), &base.to_string())
        })
        .unwrap_err();
        assert!(error.to_lowercase().contains("cancel"), "{error}");
        assert_eq!(
            load_rebase_entries_cancellable(repo.path(), &base.to_string(), &cancel).unwrap_err(),
            "History preparation cancelled."
        );
        assert_eq!(repo.head(), tip);
    }

    #[test]
    fn history_metadata_loops_check_cancellation_between_commits() {
        let repo = TestRepo::new();
        let base = repo.commit("file", b"base");
        let tip = repo.commit("file", b"tip");
        let opened = repo.open();
        let checks = std::cell::Cell::new(0);
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let error = entries_from_reader(
            &opened,
            format!("{base}\nnot-an-object-id\n").as_bytes(),
            &Default::default(),
            MAX_PLAN_COMMITS,
            || {
                checks.set(checks.get() + 1);
                if checks.get() == 2 {
                    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                check_plan_cancel(&cancel)
            },
        )
        .unwrap_err();
        assert_eq!(error, "History preparation cancelled.");
        assert_eq!(checks.get(), 2, "stop before parsing the second commit");

        let target = PlanTarget::capture(repo.path(), 0).unwrap();
        let mut entries = load_rebase_entries_checked(repo.path(), &base.to_string()).unwrap();
        entries.push(RebaseEntry {
            oid: "not-an-object-id".into(),
            summary: String::new(),
            message: String::new(),
            action: RebaseAction::Pick,
        });
        checks.set(0);
        cancel.store(false, std::sync::atomic::Ordering::Relaxed);
        let error = target
            .metadata_checked(&base.to_string(), &entries, || {
                checks.set(checks.get() + 1);
                if checks.get() == 3 {
                    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                check_plan_cancel(&cancel)
            })
            .unwrap_err();
        assert_eq!(error, "History preparation cancelled.");
        assert_eq!(
            checks.get(),
            3,
            "stop before reading the second entry's metadata"
        );
        assert_eq!(repo.head(), tip);
    }

    #[test]
    fn plan_freezes_replacements_and_metadata_matches_replaced_history() {
        let repo = TestRepo::new();
        let base = repo.commit("file", b"base");
        let tip = repo.commit("file", b"tip");
        let opened = repo.open();
        let base_commit = opened.find_commit(base).unwrap();
        let tip_commit = opened.find_commit(tip).unwrap();
        let signature = opened.signature().unwrap();
        let replacement_base = opened
            .commit(
                None,
                &signature,
                &signature,
                "replacement upstream",
                &base_commit.tree().unwrap(),
                &[],
            )
            .unwrap();
        let side = opened
            .commit(
                None,
                &signature,
                &signature,
                "replacement side",
                &base_commit.tree().unwrap(),
                &[&base_commit],
            )
            .unwrap();
        let side_commit = opened.find_commit(side).unwrap();
        let replacement_tip = opened
            .commit(
                None,
                &signature,
                &signature,
                "replacement merge",
                &tip_commit.tree().unwrap(),
                &[&base_commit, &side_commit],
            )
            .unwrap();
        opened
            .reference(
                &format!("refs/replace/{base}"),
                replacement_base,
                false,
                "fixture",
            )
            .unwrap();
        opened
            .reference(
                &format!("refs/replace/{tip}"),
                replacement_tip,
                false,
                "fixture",
            )
            .unwrap();
        let target = PlanTarget::capture(repo.path(), 0).unwrap();
        let entries = load_rebase_entries_checked(repo.path(), &base.to_string()).unwrap();
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.oid == tip.to_string())
                .unwrap()
                .summary,
            "replacement merge"
        );
        let (summary, preserve) = target
            .review_metadata(
                &base.to_string(),
                &entries,
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(summary, "replacement upstream");
        assert!(
            preserve,
            "merge classification must use the same replacement objects as the entries"
        );
        assert!(drop_plan(repo.path(), &[tip.to_string()])
            .unwrap_err()
            .contains("contains merges"));
        target.validate().unwrap();
        let new_replacement = opened
            .commit(
                None,
                &signature,
                &signature,
                "changed replacement merge",
                &tip_commit.tree().unwrap(),
                &[&base_commit, &side_commit],
            )
            .unwrap();
        opened
            .reference(
                &format!("refs/replace/{tip}"),
                new_replacement,
                true,
                "changed fixture",
            )
            .unwrap();
        assert_eq!(repo.head(), tip);
        assert!(target
            .validate()
            .unwrap_err()
            .contains("Replacement history changed"));
        assert!(target
            .execute_guarded(&base.to_string(), &entries, || Ok(()))
            .unwrap_err()
            .contains("Replacement history changed"));
        assert!(crate::git::recovery::list(repo.path()).is_empty());
    }

    #[test]
    fn queued_rebase_refuses_changed_session_and_keeps_history() {
        for changed_path in [false, true] {
            let repo = TestRepo::new();
            let base = repo.commit("base", b"base");
            let tip = repo.commit("tip", b"tip");
            let target = PlanTarget::capture(repo.path(), 0).unwrap();
            let mut entries = load_rebase_entries_checked(repo.path(), &base.to_string()).unwrap();
            entries[0].action = RebaseAction::Drop;
            let state = crate::state::new_shared_state();
            state.lock().unwrap().repo_path = Some(repo.path().to_path_buf());
            let hold = crate::git::mutation::reserve(repo.path());
            start_rebase_checked(target, &base.to_string(), entries, state.clone());
            let other = TestRepo::new();
            other.commit("other", b"other");
            if changed_path {
                state.lock().unwrap().repo_path = Some(other.path().to_path_buf());
            } else {
                state.lock().unwrap().repo_epoch += 1;
            }
            drop(hold);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !state.lock().unwrap().local_jobs.is_empty()
                && std::time::Instant::now() < deadline
            {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(state.lock().unwrap().local_jobs.is_empty());
            assert_eq!(repo.head(), tip);
            assert!(repo.path().join("tip").exists());
            assert!(crate::git::recovery::list(repo.path()).is_empty());
        }
    }

    #[test]
    fn rebase_rechecks_session_and_replacements_after_snapshot() {
        for replacements_changed in [false, true] {
            let repo = TestRepo::new();
            let base = repo.commit("base", b"base");
            let tip = repo.commit("tip", b"tip");
            let target = PlanTarget::capture(repo.path(), 0).unwrap();
            let mut entries = load_rebase_entries_checked(repo.path(), &base.to_string()).unwrap();
            entries[0].action = RebaseAction::Drop;
            let checks = std::cell::Cell::new(0);
            let result = target.execute_guarded(&base.to_string(), &entries, || {
                checks.set(checks.get() + 1);
                if checks.get() == 2 {
                    if replacements_changed {
                        repo.open()
                            .reference(
                                &format!("refs/replace/{tip}"),
                                base,
                                false,
                                "concurrent fixture",
                            )
                            .unwrap();
                    } else {
                        return Err("repository session changed after snapshot".into());
                    }
                }
                Ok(())
            });
            assert_eq!(checks.get(), 2);
            let error = result.unwrap_err();
            assert!(
                error.contains(if replacements_changed {
                    "Replacement history changed"
                } else {
                    "session changed after snapshot"
                }),
                "{error}"
            );
            assert_eq!(repo.head(), tip);
            assert!(repo.path().join("tip").exists());
            assert!(!repo.open().path().join("rebase-merge").exists());
        }
    }

    #[test]
    fn native_skip_does_not_amend_upstream_or_previous_commit() {
        for squash in [false, true] {
            let repo = TestRepo::new();
            let base = repo.commit("file", b"a\n");
            repo.commit("file", b"b\n");
            if squash {
                repo.commit("retained", b"retain\n");
            }
            repo.commit("file", b"c\n");
            let mut entries = load_rebase_entries(repo.path(), &base.to_string()).unwrap();
            entries[0].action = RebaseAction::Drop;
            let last = entries.last_mut().unwrap();
            last.action = if squash {
                RebaseAction::Squash
            } else {
                RebaseAction::Reword
            };
            last.message = "MUST NOT REWRITE PREVIOUS COMMIT".into();
            assert!(execute(repo.path(), &base.to_string(), &entries).is_err());
            assert!(repo.open().index().unwrap().has_conflicts());
            let prior = repo.head();
            let prior_tree = repo.open().find_commit(prior).unwrap().tree_id();
            // Native skip is deliberately tested: safety must survive app restart
            // and command-line continuation, not depend on intercepting a button.
            run_git_seq(repo.path(), &["rebase", "--skip"]).unwrap();
            assert_eq!(repo.head(), prior);
            assert_eq!(
                repo.open()
                    .head()
                    .unwrap()
                    .peel_to_commit()
                    .unwrap()
                    .tree_id(),
                prior_tree
            );
            assert_eq!(repo.index_bytes("file"), b"a\n");
            if !squash {
                assert_eq!(repo.head(), base);
            }
            cleanup_finished(repo.path());
        }
    }

    #[test]
    fn native_skip_reword_in_merge_segment_preserves_untouched_messages() {
        let repo = TestRepo::new();
        let base = repo.commit("file", b"a\n");
        repo.git_ok(&["checkout", "-b", "side"]);
        let dropped = repo.commit("file", b"b\n");
        let reword = repo.commit("file", b"c\n");
        repo.git_ok(&["checkout", "main"]);
        repo.commit("main", b"main\n");
        repo.git_ok(&["merge", "--no-ff", "side", "-m", "original merge"]);
        let mut entries = load_rebase_entries(repo.path(), &base.to_string()).unwrap();
        entries
            .iter_mut()
            .find(|e| e.oid == dropped.to_string())
            .unwrap()
            .action = RebaseAction::Drop;
        let edit = entries
            .iter_mut()
            .find(|e| e.oid == reword.to_string())
            .unwrap();
        edit.action = RebaseAction::Reword;
        edit.message = "UNRELATED MESSAGE".into();
        assert!(execute(repo.path(), &base.to_string(), &entries).is_err());
        assert!(repo.open().index().unwrap().has_conflicts());
        run_git_seq(repo.path(), &["rebase", "--skip"]).unwrap();
        let log = repo.git(&["log", "--format=%B", "HEAD"]);
        assert!(!String::from_utf8_lossy(&log.stdout).contains("UNRELATED MESSAGE"));
        assert!(repo.open().graph_descendant_of(repo.head(), base).unwrap());
        assert_eq!(
            repo.open()
                .find_reference("refs/heads/side")
                .unwrap()
                .target(),
            Some(reword)
        );
        assert_eq!(repo.index_bytes("file"), b"a\n");
    }

    #[test]
    fn native_skip_conflicted_reworded_merge_does_not_amend_first_parent() {
        let repo = TestRepo::new();
        let base = repo.commit("file", b"a\n");
        repo.git_ok(&["checkout", "-b", "side"]);
        repo.commit("file", b"b\n");
        let reverted = repo.commit("file", b"a\n");
        repo.git_ok(&["checkout", "main"]);
        let main = repo.commit("file", b"c\n");
        repo.git_ok(&["merge", "--no-ff", "side", "-m", "original merge"]);
        let merged = repo.head();
        let mut entries = load_rebase_entries(repo.path(), &base.to_string()).unwrap();
        entries
            .iter_mut()
            .find(|e| e.oid == reverted.to_string())
            .unwrap()
            .action = RebaseAction::Drop;
        let edit = entries
            .iter_mut()
            .find(|e| e.oid == merged.to_string())
            .unwrap();
        edit.action = RebaseAction::Reword;
        edit.message = "MUST NOT AMEND FIRST PARENT".into();
        assert!(execute(repo.path(), &base.to_string(), &entries).is_err());
        assert!(repo.open().index().unwrap().has_conflicts());
        assert_eq!(repo.head(), main);
        run_git_seq(repo.path(), &["rebase", "--skip"]).unwrap();
        assert_eq!(repo.head(), main);
        assert_eq!(repo.index_bytes("file"), b"c\n");
        assert_eq!(
            repo.open()
                .find_reference("refs/heads/side")
                .unwrap()
                .target(),
            Some(reverted)
        );
    }

    #[test]
    fn split_refuses_discarded_or_stashed_original_changes() {
        for stash in [false, true] {
            let repo = TestRepo::new();
            let base = repo.commit("base", b"base\n");
            repo.write("one", b"one\n");
            repo.write("two", b"two\n");
            repo.git_ok(&["add", "one", "two"]);
            repo.git_ok(&["commit", "-m", "combined"]);
            let original = repo.head();
            let mut entries = load_rebase_entries(repo.path(), &base.to_string()).unwrap();
            entries[0].action = RebaseAction::Edit;
            execute(repo.path(), &base.to_string(), &entries).unwrap();
            prepare_split(repo.path()).unwrap();
            repo.git_ok(&["add", "one"]);
            super::super::staging::git_commit(repo.path(), "first part", false, false).unwrap();
            if stash {
                repo.git_ok(&["stash", "push", "-u"]);
            } else {
                std::fs::remove_file(repo.path().join("two")).unwrap();
            }
            ensure_clean_split(repo.path()).unwrap();
            let head = repo.head();
            let index = std::fs::read(repo.open().path().join("index")).unwrap();
            assert!(verify_split_complete(repo.path())
                .unwrap_err()
                .contains("do not reproduce"));
            assert_eq!(repo.head(), head);
            assert_eq!(
                std::fs::read(repo.open().path().join("index")).unwrap(),
                index
            );
            assert!(repo
                .open()
                .path()
                .join("rebase-merge/recorde-split")
                .exists());
            repo.git_ok(&[
                "restore",
                "--source",
                &original.to_string(),
                "--worktree",
                "--",
                "two",
            ]);
            repo.git_ok(&["add", "two"]);
            super::super::staging::git_commit(repo.path(), "restored part", false, false).unwrap();
            verify_split_complete(repo.path()).unwrap();
            run_git_seq(repo.path(), &["rebase", "--continue"]).unwrap();
            assert_eq!(
                repo.open()
                    .head()
                    .unwrap()
                    .peel_to_commit()
                    .unwrap()
                    .tree_id(),
                repo.open().find_commit(original).unwrap().tree_id()
            );
        }
    }

    #[test]
    fn linear_editor_rejects_changed_native_source_set_without_replacing_todo() {
        let repo = TestRepo::new();
        let base = repo.commit("base", b"base\n");
        let planned = repo.commit("planned", b"planned\n");
        let entries = load_rebase_entries(repo.path(), &base.to_string()).unwrap();
        let extra = repo.commit("extra", b"extra\n");
        let script = repo.open().path().join("witness-editor.sh");
        std::fs::write(&script, sequence_editor(&entries, &entries, &[], false)).unwrap();
        let todo = repo.open().path().join("witness-todo");
        for native in [
            format!("pick {planned}\npick {extra}\n"),
            format!("pick {extra}\n"),
            format!("pick {planned}\npick {planned}\n"),
        ] {
            std::fs::write(&todo, &native).unwrap();
            let index = std::fs::read(repo.open().path().join("index")).unwrap();
            let result = repo.git(&[
                "-c",
                &format!(
                    "alias.witness=!sh {}",
                    quote_shell(&git_path(&script.to_string_lossy()))
                ),
                "witness",
                &git_path(&todo.to_string_lossy()),
            ]);
            assert!(!result.status.success());
            assert_eq!(std::fs::read_to_string(&todo).unwrap(), native);
            assert_eq!(repo.head(), extra);
            assert_eq!(
                std::fs::read(repo.open().path().join("index")).unwrap(),
                index
            );
        }
    }

    #[test]
    fn manual_submodule_resolution_is_detected_despite_ignore_config() {
        let repo = TestRepo::new();
        let initial = repo.commit("base", b"base\n");
        repo.git_ok(&[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{initial},module"),
        ]);
        repo.git_ok(&["commit", "-m", "submodule base"]);
        let base = repo.head();
        repo.git_ok(&["checkout", "-b", "side"]);
        let side = repo.commit("side", b"side\n");
        repo.git_ok(&["checkout", "main"]);
        repo.commit("main", b"main\n");
        repo.git_ok(&["merge", "--no-ff", "--no-commit", "side"]);
        repo.git_ok(&[
            "update-index",
            "--cacheinfo",
            &format!("160000,{side},module"),
        ]);
        repo.git_ok(&["commit", "-m", "manual gitlink resolution"]);
        repo.git_ok(&["config", "diff.ignoreSubmodules", "all"]);
        let head = repo.head();
        let index = std::fs::read(repo.open().path().join("index")).unwrap();
        assert!(verify_merge_resolutions(repo.path(), &base.to_string())
            .unwrap_err()
            .contains("manual resolution"));
        assert_eq!(repo.head(), head);
        assert_eq!(
            std::fs::read(repo.open().path().join("index")).unwrap(),
            index
        );
    }

    #[test]
    fn edit_split_survives_reload_and_refuses_uncommitted_parts() {
        let repo = TestRepo::new();
        let base = repo.commit("base", b"base\n");
        repo.write("one", b"one\n");
        repo.write("two", b"two\n");
        repo.git_ok(&["add", "one", "two"]);
        repo.git_ok(&[
            "commit",
            "--author",
            "Split Author <split@example.com>",
            "-m",
            "combined",
        ]);
        let original = repo.head();
        let mut entries = load_rebase_entries(repo.path(), &base.to_string()).unwrap();
        entries[0].action = RebaseAction::Edit;
        super::super::recovery::protect_sequence(repo.path(), "Split", || {
            execute(repo.path(), &base.to_string(), &entries)
        })
        .unwrap();
        assert!(is_edit_pause(&repo.open()));
        super::super::recovery::continue_sequence(repo.path(), || prepare_split(repo.path()))
            .unwrap();
        assert_eq!(repo.head(), base);
        // Git versions may abbreviate this file; authorship uses our pinned OID.
        std::fs::write(
            repo.open().path().join("rebase-merge/stopped-sha"),
            &original.to_string()[..7],
        )
        .unwrap();
        assert_eq!(std::fs::read(repo.path().join("two")).unwrap(), b"two\n");
        assert!(prepare_split(repo.path()).is_err());
        assert!(verify_split_complete(repo.path()).is_err());
        repo.git_ok(&["add", "one"]);
        super::super::recovery::continue_sequence(repo.path(), || {
            super::super::staging::git_commit(repo.path(), "first part", false, false)
        })
        .unwrap();
        assert!(verify_split_complete(repo.path()).is_err());
        repo.git_ok(&["add", "two"]);
        super::super::recovery::continue_sequence(repo.path(), || {
            super::super::staging::git_commit(repo.path(), "second part", false, false)
        })
        .unwrap();
        super::super::recovery::continue_sequence(repo.path(), || {
            verify_split_complete(repo.path())?;
            run_git_seq(repo.path(), &["rebase", "--continue"]).map_err(|e| e.user_message())
        })
        .unwrap();
        let opened = repo.open();
        let head = opened.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(
            head.tree_id(),
            opened.find_commit(original).unwrap().tree_id()
        );
        assert_eq!(head.parent(0).unwrap().parent_id(0).unwrap(), base);
        assert_eq!(head.author().email().unwrap(), "split@example.com");
        assert_eq!(
            head.parent(0).unwrap().author().email().unwrap(),
            "split@example.com"
        );
        let checkpoint = super::super::recovery::list(repo.path()).remove(0);
        super::super::recovery::restore(repo.path(), &checkpoint).unwrap();
        assert_eq!(repo.head(), original);
    }

    #[test]
    fn merge_range_reorders_squashes_and_drops_within_branch_segments() {
        let repo = TestRepo::new();
        let base = repo.commit("base", b"base\n");
        repo.git_ok(&["checkout", "-b", "side"]);
        let one = repo.commit("one", b"one\n");
        let two = repo.commit("two", b"two\n");
        let dropped = repo.commit("dropped", b"drop\n");
        let side = repo.head();
        repo.git_ok(&["checkout", "main"]);
        let main = repo.commit("main", b"main\n");
        repo.git_ok(&["merge", "--no-ff", "side", "-m", "merge side"]);
        let mut entries = load_rebase_entries(repo.path(), &base.to_string()).unwrap();
        let first = entries
            .iter()
            .position(|e| e.oid == one.to_string())
            .unwrap();
        let second = entries
            .iter()
            .position(|e| e.oid == two.to_string())
            .unwrap();
        entries.swap(first, second);
        entries[second].action = RebaseAction::Squash;
        entries[second].message = "combined side".into();
        entries
            .iter_mut()
            .find(|e| e.oid == dropped.to_string())
            .unwrap()
            .action = RebaseAction::Drop;
        execute(repo.path(), &base.to_string(), &entries).unwrap();
        let opened = repo.open();
        let head = opened.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.parent_count(), 2);
        assert_eq!(head.parent_id(0).unwrap(), main);
        let rewritten_side = head.parent(1).unwrap();
        assert_eq!(rewritten_side.message().unwrap(), "combined side\n");
        assert_eq!(rewritten_side.parent_id(0).unwrap(), base);
        assert!(head.tree().unwrap().get_path(Path::new("dropped")).is_err());
        assert_eq!(
            opened.find_reference("refs/heads/side").unwrap().target(),
            Some(side)
        );
        assert_eq!(repo.index_bytes("one"), b"one\n");
        assert_eq!(repo.index_bytes("two"), b"two\n");
    }

    #[test]
    fn manual_merge_resolution_is_never_silently_discarded() {
        let repo = TestRepo::new();
        let base = repo.commit("base", b"base\n");
        repo.git_ok(&["checkout", "-b", "side"]);
        repo.commit("side", b"side\n");
        repo.git_ok(&["checkout", "main"]);
        repo.commit("main", b"main\n");
        repo.git_ok(&["merge", "--no-ff", "--no-commit", "side"]);
        repo.write("resolution", b"manual merge change\n");
        repo.git_ok(&["add", "resolution"]);
        repo.git_ok(&["commit", "-m", "manual merge"]);
        let before = repo.head();
        let entries = load_rebase_entries(repo.path(), &base.to_string()).unwrap();
        assert!(execute(repo.path(), &base.to_string(), &entries)
            .unwrap_err()
            .contains("manual resolution"));
        assert_eq!(repo.head(), before);
        assert_eq!(repo.open().state(), git2::RepositoryState::Clean);
        assert_eq!(repo.index_bytes("resolution"), b"manual merge change\n");
    }

    #[test]
    fn moving_across_merge_branch_points_is_rejected_without_mutation() {
        let repo = TestRepo::new();
        let base = repo.commit("base", b"base\n");
        repo.git_ok(&["checkout", "-b", "side"]);
        let side = repo.commit("side", b"side\n");
        repo.git_ok(&["checkout", "main"]);
        let main = repo.commit("main", b"main\n");
        repo.git_ok(&["merge", "--no-ff", "side", "-m", "merge"]);
        let before = repo.head();
        let mut entries = load_rebase_entries(repo.path(), &base.to_string()).unwrap();
        let a = entries
            .iter()
            .position(|e| e.oid == side.to_string())
            .unwrap();
        let b = entries
            .iter()
            .position(|e| e.oid == main.to_string())
            .unwrap();
        entries.swap(a, b);
        assert!(execute(repo.path(), &base.to_string(), &entries).is_err());
        assert_eq!(repo.head(), before);
        assert_eq!(repo.open().state(), git2::RepositoryState::Clean);
        assert_eq!(repo.index_bytes("main"), b"main\n");
        assert_eq!(std::fs::read(repo.path().join("side")).unwrap(), b"side\n");
    }

    #[test]
    fn split_refuses_dirty_reset_and_cancelled_continue_then_aborts_natively() {
        let repo = TestRepo::new();
        let base = repo.commit("base", b"base\n");
        let original = repo.commit("part", b"committed\n");
        let mut entries = load_rebase_entries(repo.path(), &base.to_string()).unwrap();
        entries[0].action = RebaseAction::Edit;
        execute(repo.path(), &base.to_string(), &entries).unwrap();
        repo.write("part", b"later work\n");
        assert!(prepare_split(repo.path()).is_err());
        assert_eq!(repo.head(), original);
        assert_eq!(
            std::fs::read(repo.path().join("part")).unwrap(),
            b"later work\n"
        );
        repo.write("part", b"committed\n");
        prepare_split(repo.path()).unwrap();
        repo.git_ok(&["add", "part"]);
        super::super::staging::git_commit(repo.path(), "split part", false, false).unwrap();
        let split_head = repo.head();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        assert!(
            super::super::process::with_cancellation(cancel, || verify_split_complete(repo.path()))
                .is_err()
        );
        assert_eq!(repo.head(), split_head);
        assert!(repo
            .open()
            .path()
            .join("rebase-merge/recorde-split")
            .is_file());
        run_git_seq(repo.path(), &["rebase", "--abort"]).unwrap();
        cleanup_finished(repo.path());
        assert_eq!(repo.head(), original);
        assert_eq!(repo.index_bytes("part"), b"committed\n");
        assert_eq!(repo.open().state(), git2::RepositoryState::Clean);
    }

    #[test]
    fn merge_preserving_reword_retains_parents_and_tree() {
        let repo = TestRepo::new();
        let base = repo.commit("base", b"base\n");
        repo.git_ok(&["checkout", "-b", "side"]);
        let side = repo.commit("side", b"side\n");
        repo.git_ok(&["checkout", "main"]);
        repo.commit("main", b"main\n");
        repo.git_ok(&["merge", "--no-ff", "side", "-m", "merge side"]);
        let merged = repo.head();
        let tree = repo
            .open()
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .tree_id();
        let mut entries = load_rebase_entries(repo.path(), &base.to_string()).unwrap();
        for entry in &mut entries {
            if entry.oid == side.to_string() {
                entry.action = RebaseAction::Reword;
                entry.message = "reword side\n".into();
            }
            if entry.oid == merged.to_string() {
                entry.action = RebaseAction::Reword;
                entry.message = "reword merge\n\nbody with $() and quotes ' \"\n".into();
            }
        }
        execute(repo.path(), &base.to_string(), &entries).unwrap();
        let opened = repo.open();
        let head = opened.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.parent_count(), 2);
        assert_eq!(head.tree_id(), tree);
        assert_eq!(
            head.message().unwrap(),
            "reword merge\n\nbody with $() and quotes ' \"\n"
        );
        assert_eq!(head.parent(1).unwrap().message().unwrap(), "reword side\n");
        assert_eq!(
            opened
                .find_branch("side", git2::BranchType::Local)
                .unwrap()
                .get()
                .target(),
            Some(side)
        );
        assert_eq!(repo.index_bytes("side"), b"side\n");
    }

    #[test]
    fn simultaneous_rebases_use_independent_todos() {
        let mut workers = Vec::new();
        for i in 0..2 {
            workers.push(std::thread::spawn(move || {
                let repo = TestRepo::new();
                let base = repo.commit("base.txt", b"base\n");
                let first = repo.commit("first.txt", b"first\n");
                let second = repo.commit("second.txt", b"second\n");
                let entries = vec![
                    RebaseEntry {
                        message: String::new(),
                        oid: first.to_string(),
                        summary: "ignored\nexec false".into(),
                        action: if i == 0 {
                            RebaseAction::Pick
                        } else {
                            RebaseAction::Drop
                        },
                    },
                    RebaseEntry {
                        message: String::new(),
                        oid: second.to_string(),
                        summary: String::new(),
                        action: if i == 0 {
                            RebaseAction::Drop
                        } else {
                            RebaseAction::Pick
                        },
                    },
                ];
                execute(repo.path(), &base.to_string(), &entries).unwrap();
                assert_eq!(repo.path().join("first.txt").exists(), i == 0);
                assert_eq!(repo.path().join("second.txt").exists(), i == 1);
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
    }
    #[test]
    fn reword_and_squash_preserve_tree_and_author_with_edited_messages() {
        let repo = TestRepo::new();
        let base = repo.commit("base", b"base");
        repo.commit("first", b"first");
        repo.commit("second", b"second");
        let before = repo
            .open()
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .tree_id();
        let mut entries = load_rebase_entries(repo.path(), &base.to_string()).unwrap();
        entries[0].action = RebaseAction::Reword;
        entries[0].message = "new title\n\nfull body\n".into();
        entries[1].action = RebaseAction::Squash;
        entries[1].message = "combined title\n\ncombined body\n".into();
        execute(repo.path(), &base.to_string(), &entries).unwrap();
        let opened = repo.open();
        let head = opened.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(
            head.message().ok(),
            Some("combined title\n\ncombined body\n")
        );
        assert_eq!(head.tree_id(), before);
        assert_eq!(head.parent_id(0).unwrap(), base);
        assert_eq!(head.author().email().ok(), Some("author@example.com"));
    }

    #[test]
    fn invalid_first_action_is_rejected_before_changing_head() {
        let repo = TestRepo::new();
        let base = repo.commit("base", b"base");
        let head = repo.commit("first", b"first");
        let mut entries = load_rebase_entries(repo.path(), &base.to_string()).unwrap();
        entries[0].action = RebaseAction::Squash;
        assert!(execute(repo.path(), &base.to_string(), &entries).is_err());
        assert_eq!(repo.head(), head);
        assert_eq!(repo.open().state(), git2::RepositoryState::Clean);
    }

    #[test]
    fn edited_message_survives_conflict_and_native_continue() {
        let repo = TestRepo::new();
        let base = repo.commit("file", b"a\n");
        repo.commit("file", b"b\n");
        repo.commit("file", b"c\n");
        let mut entries = load_rebase_entries(repo.path(), &base.to_string()).unwrap();
        entries[0].action = RebaseAction::Drop;
        entries[1].action = RebaseAction::Reword;
        entries[1].message = "retained through conflict\n".into();
        assert!(execute(repo.path(), &base.to_string(), &entries).is_err());
        assert!(repo.open().index().unwrap().has_conflicts());
        let loaded = crate::git::conflict_edit::load(repo.path(), "file").unwrap();
        crate::git::conflict_edit::save(&loaded, "c\n").unwrap();
        run_git_seq(repo.path(), &["rebase", "--continue"]).unwrap();
        cleanup_finished(repo.path());
        assert_eq!(
            std::fs::read_dir(repo.open().path().join("recorde/rebases"))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(
            repo.open()
                .head()
                .unwrap()
                .peel_to_commit()
                .unwrap()
                .message()
                .ok(),
            Some("retained through conflict\n")
        );
        assert_eq!(repo.index_bytes("file"), b"c\n");
        assert_eq!(repo.open().state(), git2::RepositoryState::Clean);
    }
}
