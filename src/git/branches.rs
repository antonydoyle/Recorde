use std::path::Path;

use crate::git::merge::run_git_seq;
use crate::state::SharedState;

#[cfg(test)]
#[path = "branches_tests.rs"]
mod tests;

// ── Re-exported reload helper ────────────────────────────────────────────────

/// Reload the graph + status for the currently open repo.
/// Spawns its own thread — safe to call from any thread.
pub fn reload_repo(state: SharedState) {
    crate::git::operations::refresh_repo(state);
}

// ── Checkout ─────────────────────────────────────────────────────────────────

pub fn checkout_branch(branch_name: &str, state: SharedState) {
    let name = branch_name.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    {
        let mut s = state.lock().unwrap();
        s.remote_op = Some(format!("Switching to {}...", name));
        s.console_log.push(format!("$ checkout {}", name));
    }

    crate::git::mutation::spawn_operation(path, state, move |path| {
        super::recovery::protect(path, "Switch branch", || {
            do_checkout(path, &name).map_err(|e| e.message().to_string())
        })
    });
}

fn do_checkout(path: &Path, name: &str) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(path)?;
    repo.find_branch(name, git2::BranchType::Local)?;
    run_git_seq(path, &["checkout", name, "--"])
        .map_err(|e| git2::Error::from_str(&e.user_message()))
}

// ── Create branch ─────────────────────────────────────────────────────────────

/// Create a new branch. `from_oid` = commit to branch from (None = HEAD).
pub fn create_branch(name: &str, from_oid: Option<&str>, checkout: bool, state: SharedState) {
    let name = name.to_string();
    let from_oid = from_oid.map(str::to_owned);
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };

    crate::git::mutation::spawn_operation(path, state, move |path| {
        do_create(path, &name, from_oid.as_deref(), checkout).map_err(|e| e.message().to_string())
    });
}

fn do_create(
    path: &Path,
    name: &str,
    from_oid: Option<&str>,
    checkout: bool,
) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(path)?;

    let commit = if let Some(oid_str) = from_oid {
        let oid = git2::Oid::from_str(oid_str)?;
        repo.find_commit(oid)?
    } else {
        repo.head()?.peel_to_commit()?
    };

    if !git2::Branch::name_is_valid(name)? {
        return Err(git2::Error::from_str("Invalid branch name."));
    }
    let oid = commit.id().to_string();
    let args = if checkout {
        vec!["checkout", "-b", name, &oid, "--"]
    } else {
        vec!["branch", "--", name, &oid]
    };
    run_git_seq(path, &args).map_err(|e| git2::Error::from_str(&e.user_message()))
}

// ── Rename branch ────────────────────────────────────────────────────────────

pub fn rename_branch(old_name: &str, new_name: &str, state: SharedState) {
    let old = old_name.to_string();
    let new = new_name.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    crate::git::mutation::spawn_operation(path, state, move |path| {
        do_rename(path, &old, &new).map_err(|e| e.message().to_string())
    });
}

fn do_rename(path: &Path, old: &str, new: &str) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(path)?;
    let mut branch = repo.find_branch(old, git2::BranchType::Local)?;
    branch.rename(new, false)?;
    Ok(())
}

// ── Force-delete branch ───────────────────────────────────────────────────────

pub fn force_delete_branch(name: &str, state: SharedState) {
    let n = name.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    let output_state = state.clone();
    crate::git::mutation::spawn_operation(path, state, move |path| {
        let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
        let oid = repo
            .find_branch(&n, git2::BranchType::Local)
            .and_then(|b| b.get().peel_to_commit())
            .map_err(|e| e.to_string())?
            .id()
            .to_string();
        save_deleted(&repo, &n, &oid).map_err(|e| e.to_string())?;
        run_git_seq(path, &["branch", "-D", "--", &n]).map_err(|e| e.user_message())?;
        push_deleted(&output_state, path, n, oid);
        Ok(())
    });
}

pub fn restore_deleted_branch(name: &str, oid: &str, state: SharedState) {
    let n = name.to_string();
    let o = oid.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    let output_state = state.clone();
    crate::git::mutation::spawn_operation(path, state, move |path| {
        let oid = git2::Oid::from_str(&o).map_err(|e| e.to_string())?;
        run_git_seq(path, &["branch", "--", &n, &oid.to_string()]).map_err(|e| e.user_message())?;
        let mut s = output_state.lock().unwrap();
        if s.repo_path.as_deref() == Some(path) {
            s.deleted_branches.retain(|d| d.name != n);
        }
        if let Some(view) = s.repo_views.get_mut(path) {
            view.deleted_branches.retain(|d| d.name != n);
        }
        Ok(())
    });
}

fn deleted_reference(name: &str, oid: &str) -> String {
    let encoded: String = name
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("refs/recorde/deleted-branches/{encoded}/{oid}")
}

pub(crate) fn save_deleted(
    repo: &git2::Repository,
    name: &str,
    oid: &str,
) -> Result<(), git2::Error> {
    let reference = deleted_reference(name, oid);
    let target = git2::Oid::from_str(oid)?;
    if repo
        .find_reference(&reference)
        .is_ok_and(|r| r.target() == Some(target))
    {
        return Ok(());
    }
    repo.reference(&reference, target, false, "Recorde deleted branch recovery")?;
    Ok(())
}

pub(crate) fn load_deleted(path: &Path) -> Vec<crate::state::DeletedBranch> {
    let Ok(repo) = git2::Repository::open(path) else {
        return Vec::new();
    };
    let Ok(references) = repo.references_glob("refs/recorde/deleted-branches/*") else {
        return Vec::new();
    };
    references
        .flatten()
        .filter_map(|reference| {
            let reference_name = reference.name().ok()?;
            let tail = reference_name.strip_prefix("refs/recorde/deleted-branches/")?;
            let (encoded, oid) = tail.split_once('/')?;
            if encoded.len() % 2 != 0 || !encoded.is_ascii() {
                return None;
            }
            let bytes: Option<Vec<u8>> = (0..encoded.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&encoded[i..i + 2], 16).ok())
                .collect();
            let name = String::from_utf8(bytes?).ok()?;
            if !git2::Branch::name_is_valid(&name).ok()?
                || repo.find_branch(&name, git2::BranchType::Local).is_ok()
            {
                return None;
            }
            if reference.target()? != git2::Oid::from_str(oid).ok()? {
                return None;
            }
            Some(crate::state::DeletedBranch {
                name,
                oid: oid.into(),
            })
        })
        .collect()
}

fn push_deleted(state: &SharedState, path: &Path, name: String, oid: String) {
    let mut s = state.lock().unwrap();
    let deleted = if s.repo_path.as_deref() == Some(path) {
        &mut s.deleted_branches
    } else {
        &mut s
            .repo_views
            .entry(path.into())
            .or_default()
            .deleted_branches
    };
    deleted.push(crate::state::DeletedBranch { name, oid });
    if deleted.len() > 5 {
        deleted.remove(0);
    }
}

// ── Publish branch ────────────────────────────────────────────────────────────

pub fn publish_branch(name: &str, state: SharedState) {
    let n = name.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    crate::git::mutation::spawn_operation(path, state, move |path| {
        let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
        let remote = super::remote::default_push_remote(&repo).map_err(|e| e.user_message())?;
        let source = format!("refs/heads/{n}");
        run_git_seq(
            path,
            &["push", "-u", "--", &remote, &format!("{source}:{source}")],
        )
        .map_err(|e| e.user_message())
    });
}

// ── Set upstream ──────────────────────────────────────────────────────────────

pub fn set_upstream(branch: &str, upstream: &str, state: SharedState) {
    let b = branch.to_string();
    let target = upstream.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    crate::git::mutation::spawn_operation(path, state, move |path| {
        run_git_seq(
            path,
            &["branch", &format!("--set-upstream-to={target}"), "--", &b],
        )
        .map_err(|e| e.user_message())
    });
}

// ── Remote-tracking branches ─────────────────────────────────────────────────

/// Split a remote-tracking shorthand such as `origin/feature/x` into its
/// configured remote and branch. Remote names may contain `/`, so the longest
/// configured remote that prefixes the shorthand wins.
pub(crate) fn split_remote_branch(
    repo: &git2::Repository,
    shorthand: &str,
) -> Result<(String, String), String> {
    let remotes = repo.remotes().map_err(|e| e.to_string())?;
    let remote = remotes
        .iter()
        .flatten()
        .flatten()
        .filter(|name| {
            shorthand
                .strip_prefix(name)
                .and_then(|rest| rest.strip_prefix('/'))
                .is_some_and(|branch| !branch.is_empty())
        })
        .max_by_key(|name| name.len())
        .ok_or_else(|| format!("No configured remote owns {shorthand}."))?;
    Ok((
        remote.to_string(),
        shorthand[remote.len() + 1..].to_string(),
    ))
}

/// Check out a remote-tracking branch as a local branch that tracks it.
pub fn checkout_remote_branch(shorthand: &str, state: SharedState) {
    let name = shorthand.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    {
        let mut s = state.lock().unwrap();
        s.remote_op = Some(format!("Checking out {name}..."));
        s.console_log.push(format!("$ checkout --track {name}"));
    }
    crate::git::mutation::spawn_operation(path, state, move |path| {
        super::recovery::protect(path, "Switch branch", || {
            do_checkout_remote(path, &name).map(|_| ())
        })
    });
}

/// Returns the local branch that was checked out. An existing local branch of
/// the same name is reused only when it already tracks this remote branch; it
/// is never moved or reset.
pub(crate) fn do_checkout_remote(path: &Path, shorthand: &str) -> Result<String, String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let (_, branch) = split_remote_branch(&repo, shorthand)?;
    let remote_ref = format!("refs/remotes/{shorthand}");
    repo.refname_to_id(&remote_ref)
        .map_err(|_| format!("{shorthand} no longer exists. Fetch and try again."))?;
    if let Ok(local) = repo.find_branch(&branch, git2::BranchType::Local) {
        let upstream = local
            .upstream()
            .ok()
            .and_then(|u| u.get().name().ok().map(str::to_owned));
        if upstream.as_deref() != Some(remote_ref.as_str()) {
            return Err(format!(
                "A local branch named {branch} already exists and does not track {shorthand}. \
                 Check it out from Local branches, or create a branch from {shorthand} with another name."
            ));
        }
        run_git_seq(path, &["checkout", &branch, "--"]).map_err(|e| e.user_message())?;
        return Ok(branch);
    }
    if !git2::Branch::name_is_valid(&branch).map_err(|e| e.to_string())? {
        return Err(format!("{branch} is not a valid local branch name."));
    }
    run_git_seq(
        path,
        &["checkout", "--track", "-b", &branch, &remote_ref, "--"],
    )
    .map_err(|e| e.user_message())?;
    Ok(branch)
}

/// Delete a branch from its remote. The last known tip is kept as a deleted
/// branch so it can be restored locally and pushed again.
pub fn delete_remote_branch(shorthand: &str, state: SharedState) {
    let name = shorthand.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    {
        let mut s = state.lock().unwrap();
        s.remote_op = Some(format!("Deleting {name} from remote..."));
        s.console_log.push(format!("$ push --delete {name}"));
    }
    let output_state = state.clone();
    crate::git::mutation::spawn_operation(path, state, move |path| {
        let (branch, oid) = do_delete_remote_branch(path, &name)?;
        push_deleted(&output_state, path, branch, oid);
        Ok(())
    });
}

/// Returns the deleted branch name and its last known tip.
pub(crate) fn do_delete_remote_branch(
    path: &Path,
    shorthand: &str,
) -> Result<(String, String), String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let (remote, branch) = split_remote_branch(&repo, shorthand)?;
    let oid = repo
        .refname_to_id(&format!("refs/remotes/{shorthand}"))
        .map_err(|_| format!("{shorthand} no longer exists. Fetch and try again."))?
        .to_string();
    run_git_seq(
        path,
        &[
            "push",
            &format!("--force-with-lease=refs/heads/{branch}:{oid}"),
            "--",
            &remote,
            &format!(":refs/heads/{branch}"),
        ],
    )
    .map_err(|e| e.user_message())?;
    // Record the tip only once the remote change happened; a failed push must
    // not list a branch that still exists. The deletion itself already succeeded.
    if let Err(error) = save_deleted(&repo, &branch, &oid) {
        log::warn!("Could not record deleted remote branch {shorthand}: {error}");
    }
    Ok((branch, oid))
}

// ── Reset to commit ───────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ResetMode {
    Soft,
    #[default]
    Mixed,
    Hard,
}

impl ResetMode {
    pub const ALL: [Self; 3] = [Self::Soft, Self::Mixed, Self::Hard];

    pub fn label(self) -> &'static str {
        match self {
            Self::Soft => "Soft",
            Self::Mixed => "Mixed",
            Self::Hard => "Hard",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Soft => "Keep all changes staged.",
            Self::Mixed => "Keep all changes in the working tree, unstaged.",
            Self::Hard => "Discard all staged and unstaged changes to tracked files.",
        }
    }

    fn flag(self) -> &'static str {
        match self {
            Self::Soft => "--soft",
            Self::Mixed => "--mixed",
            Self::Hard => "--hard",
        }
    }
}

/// Move the current branch (or detached HEAD) to `oid`. A recovery snapshot
/// is saved first so the reset can be undone from repository settings.
pub fn reset_to_commit(oid: &str, mode: ResetMode, state: SharedState) {
    let (path, epoch) = {
        let s = state.lock().unwrap();
        (s.repo_path.clone(), s.repo_epoch)
    };
    let Some(path) = path else { return };
    match ResetPreview::capture(&path, epoch, oid) {
        Ok(preview) => preview.submit(mode, state),
        Err(error) => state.lock().unwrap().error = Some(error),
    }
}

#[cfg(test)]
pub(crate) fn do_reset_to_commit(path: &Path, oid: &str, mode: ResetMode) -> Result<(), String> {
    ResetPreview::capture(path, 0, oid)?.execute(path, mode)
}

/// The exact repository, symbolic branch (or detached HEAD), and tip shown in
/// the reset dialog. A checkout to another branch at the same tip is stale too.
#[derive(Clone, Debug)]
pub struct ResetPreview {
    pub path: std::path::PathBuf,
    pub epoch: u64,
    pub head_ref: String,
    pub head_oid: String,
    pub oid: String,
}

impl ResetPreview {
    pub fn capture(path: &Path, epoch: u64, oid: &str) -> Result<Self, String> {
        let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
        let head = repo.head().map_err(|e| e.to_string())?;
        let preview = Self {
            path: path.canonicalize().map_err(|e| e.to_string())?,
            epoch,
            head_ref: head.name().map_err(|e| e.to_string())?.to_string(),
            head_oid: head
                .peel_to_commit()
                .map_err(|e| e.to_string())?
                .id()
                .to_string(),
            oid: oid.to_string(),
        };
        preview.validate(path)?;
        Ok(preview)
    }

    pub fn validate(&self, path: &Path) -> Result<(), String> {
        if path.canonicalize().map_err(|e| e.to_string())? != self.path {
            return Err("Repository changed. Review the reset again.".into());
        }
        let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
        let head = repo
            .head()
            .map_err(|_| "HEAD changed. Review the reset again.")?;
        if head.name().ok() != Some(self.head_ref.as_str())
            || head
                .peel_to_commit()
                .map_err(|e| e.to_string())?
                .id()
                .to_string()
                != self.head_oid
        {
            return Err(
                "The current branch or HEAD changed since confirmation. Review the reset again."
                    .into(),
            );
        }
        if self.oid.len() != 40 || !self.oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Reset requires a full commit ID.".into());
        }
        repo.find_commit(git2::Oid::from_str(&self.oid).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn reset_after_snapshot(&self, path: &Path, mode: ResetMode) -> Result<(), String> {
        self.validate(path)?;
        run_git_seq(path, &["reset", mode.flag(), &self.oid]).map_err(|e| e.user_message())
    }

    #[cfg(test)]
    fn execute(&self, path: &Path, mode: ResetMode) -> Result<(), String> {
        self.execute_guarded(path, mode, || Ok(()))
    }

    fn execute_guarded(
        &self,
        path: &Path,
        mode: ResetMode,
        check_session: impl Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        check_session()?;
        self.validate(path)?;
        super::recovery::protect(path, &format!("{} reset", mode.label()), || {
            check_session()?;
            self.reset_after_snapshot(path, mode)
        })
    }

    pub fn submit(self, mode: ResetMode, state: SharedState) {
        {
            let mut s = state.lock().unwrap();
            if !confirmed_repository(&s, &self.path, self.epoch) {
                s.error =
                    Some("Repository changed since confirmation. Review the reset again.".into());
                return;
            }
            s.pending_discard = None;
            s.remote_op = Some(format!("{} reset...", mode.label()));
            s.console_log
                .push(format!("$ reset {} {}", mode.flag(), self.oid));
        }
        let path = self.path.clone();
        let guard_state = state.clone();
        crate::git::mutation::spawn_operation(path, state, move |path| {
            self.execute_guarded(path, mode, || {
                if !confirmed_repository(&guard_state.lock().unwrap(), &self.path, self.epoch) {
                    return Err(
                        "Repository changed while the reset was queued. Review the reset again."
                            .into(),
                    );
                }
                Ok(())
            })
        });
    }
}

fn confirmed_repository(state: &crate::state::AppState, path: &Path, epoch: u64) -> bool {
    state.repo_epoch == epoch
        && state
            .repo_path
            .as_ref()
            .and_then(|path| path.canonicalize().ok())
            .as_deref()
            == Some(path)
}

// ── Checkout detached HEAD ────────────────────────────────────────────────────

pub fn checkout_detached(oid_str: &str, state: SharedState) {
    let oid_s = oid_str.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    {
        let mut s = state.lock().unwrap();
        s.remote_op = Some("Checking out…".into());
    }
    crate::git::mutation::spawn_operation(path, state, move |path| {
        super::recovery::protect(path, "Detached checkout", || {
            do_checkout_detached(path, &oid_s).map_err(|e| e.message().to_string())
        })
    });
}

fn do_checkout_detached(path: &Path, oid_str: &str) -> Result<(), git2::Error> {
    // Validate the revision before passing it to Git. Let Git perform the safe
    // checkout and HEAD update together, including its dirty-worktree checks.
    let oid = git2::Oid::from_str(oid_str)?;
    run_git_seq(path, &["checkout", "--detach", &oid.to_string()])
        .map_err(|e| git2::Error::from_str(&e.user_message()))
}

// ── Delete branch ─────────────────────────────────────────────────────────────

pub fn delete_branch(branch_name: &str, state: SharedState) {
    let name = branch_name.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };

    let output_state = state.clone();
    crate::git::mutation::spawn_operation(path, state, move |path| {
        let oid = do_delete(path, &name).map_err(|e| e.to_string())?;
        push_deleted(&output_state, path, name, oid);
        Ok(())
    });
}

fn do_delete(path: &Path, name: &str) -> Result<String, git2::Error> {
    let repo = git2::Repository::open(path)?;
    let branch = repo.find_branch(name, git2::BranchType::Local)?;
    let oid = branch.get().peel_to_commit()?.id().to_string();
    save_deleted(&repo, name, &oid)?;
    run_git_seq(path, &["branch", "-d", "--", name])
        .map_err(|e| git2::Error::from_str(&e.user_message()))?;
    Ok(oid)
}

// ── Stash ─────────────────────────────────────────────────────────────────────

pub fn stash_save(state: SharedState) {
    stash_save_with_message("WIP", false, state);
}

pub fn stash_save_with_message(message: &str, include_untracked: bool, state: SharedState) {
    let msg = message.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    crate::git::mutation::spawn_operation(path, state, move |path| {
        do_stash_save(path, &msg, include_untracked)
    });
}

pub(crate) fn do_stash_save(
    path: &Path,
    message: &str,
    include_untracked: bool,
) -> Result<(), String> {
    let mut args = vec!["stash", "push"];
    if include_untracked {
        args.push("--include-untracked");
    }
    args.extend(["-m", message]);
    run_git_seq(path, &args).map_err(|e| e.user_message())
}

pub fn stash_file(file_path: &str, state: SharedState) {
    let fp = file_path.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    crate::git::mutation::spawn_operation(path, state, move |path| do_stash_file(path, &fp));
}

/// Stash one path. Untracked files are included so a new file can be stashed
/// on its own; the literal pathspec keeps other untracked files in place.
pub(crate) fn do_stash_file(path: &Path, file: &str) -> Result<(), String> {
    run_git_seq(
        path,
        &[
            "--literal-pathspecs",
            "stash",
            "push",
            "--include-untracked",
            "-m",
            &format!("Stash {file}"),
            "--",
            file,
        ],
    )
    .map_err(|e| e.user_message())
}

/// A sidebar stash is identified by both its displayed position and object ID.
/// Do not silently retarget it when another queued stash changes the reflog.
#[derive(Clone, Debug)]
pub struct StashTarget {
    pub path: std::path::PathBuf,
    pub epoch: u64,
    pub index: usize,
    pub oid: String,
}

impl StashTarget {
    pub fn capture(path: &Path, epoch: u64, index: usize, oid: &str) -> Result<Self, String> {
        let target = Self {
            path: path.canonicalize().map_err(|e| e.to_string())?,
            epoch,
            index,
            oid: oid.to_string(),
        };
        target.validate(path)?;
        Ok(target)
    }

    pub fn validate(&self, path: &Path) -> Result<(), String> {
        if path.canonicalize().map_err(|e| e.to_string())? != self.path {
            return Err("Repository changed. Review the stash again.".into());
        }
        if self.oid.len() != 40 || !self.oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Stash operations require a full commit ID.".into());
        }
        let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
        let actual = repo
            .reflog("refs/stash")
            .ok()
            .and_then(|log| log.get(self.index).map(|entry| entry.id_new()));
        if actual.map(|oid| oid.to_string()).as_deref() != Some(self.oid.as_str()) {
            return Err("The stash list changed. Refresh and select the stash again.".into());
        }
        Ok(())
    }

    /// Durable recovery ref, intentionally independent of the shifting stash list.
    pub fn recovery_ref(&self) -> String {
        format!("refs/recorde/dropped-stashes/{}", self.oid)
    }
}

fn submit_stash(
    target: StashTarget,
    state: SharedState,
    work: impl FnOnce(&Path, &StashTarget) -> Result<(), String> + Send + 'static,
) {
    {
        let mut s = state.lock().unwrap();
        if !confirmed_repository(&s, &target.path, target.epoch) {
            s.error = Some("Repository changed. Review the stash again.".into());
            return;
        }
    }
    let path = target.path.clone();
    let guard_state = state.clone();
    crate::git::mutation::spawn_operation(path, state, move |path| {
        if !confirmed_repository(&guard_state.lock().unwrap(), &target.path, target.epoch) {
            return Err(
                "Repository changed while the stash operation was queued. Review the stash again."
                    .into(),
            );
        }
        target.validate(path)?;
        work(path, &target)
    });
}

pub fn stash_branch(branch_name: &str, target: StashTarget, state: SharedState) {
    let name = branch_name.to_string();
    submit_stash(target, state, move |path, target| {
        do_stash_branch(path, &name, target)
    });
}

fn do_stash_branch(path: &Path, name: &str, target: &StashTarget) -> Result<(), String> {
    target.validate(path)?;
    // A raw object ID makes Git apply the reviewed stash even if an external
    // client changes refs/stash; unlike stash@{n}, it does not auto-drop it.
    run_git_seq(path, &["stash", "branch", "--", name, &target.oid])
        .map_err(|e| e.user_message())?;
    do_stash_drop(path, target)
}

pub fn stash_apply(target: StashTarget, state: SharedState) {
    submit_stash(target, state, |path, target| {
        do_stash_apply(path, target, false)
    });
}

pub fn stash_pop(target: StashTarget, state: SharedState) {
    submit_stash(target, state, |path, target| {
        do_stash_apply(path, target, true)
    });
}

fn do_stash_apply(path: &Path, target: &StashTarget, pop: bool) -> Result<(), String> {
    target.validate(path)?;
    run_git_seq(path, &["stash", "apply", &target.oid]).map_err(|e| e.user_message())?;
    if pop {
        do_stash_drop(path, target)?;
    }
    Ok(())
}

pub fn stash_drop(target: StashTarget, state: SharedState) {
    submit_stash(target, state, do_stash_drop);
}

fn do_stash_drop(path: &Path, target: &StashTarget) -> Result<(), String> {
    target.validate(path)?;
    let mut repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let oid = git2::Oid::from_str(&target.oid).map_err(|e| e.to_string())?;
    let reference = target.recovery_ref();
    match repo.find_reference(&reference) {
        Ok(existing) if existing.target() == Some(oid) => {}
        Ok(_) => {
            return Err("The dropped-stash recovery reference changed. The stash was kept.".into())
        }
        Err(error) if error.code() == git2::ErrorCode::NotFound => {
            repo.reference(&reference, oid, false, "Recorde: retain removed stash")
                .map_err(|e| format!("Cannot retain the stash for recovery: {e}"))?;
        }
        Err(error) => return Err(error.to_string()),
    }
    // Recheck after saving the recovery ref and before touching the reflog.
    target.validate(path)?;
    repo.stash_drop(target.index).map_err(|e| e.to_string())?;
    Ok(())
}

// ── Cherry-pick / revert sequencing ──────────────────────────────────────────

pub fn continue_cherry_pick(state: SharedState) {
    crate::git::merge::seq_op_pub("cherry-pick --continue", state, |path| {
        crate::git::merge::run_git_seq(path, &["cherry-pick", "--continue", "--no-edit"])
    });
}

pub fn abort_cherry_pick(state: SharedState) {
    crate::git::merge::seq_op_pub("cherry-pick --abort", state, |path| {
        crate::git::merge::run_git_seq(path, &["cherry-pick", "--abort"])
    });
}

pub fn continue_revert(state: SharedState) {
    crate::git::merge::seq_op_pub("revert --continue", state, |path| {
        crate::git::merge::run_git_seq(path, &["revert", "--continue", "--no-edit"])
    });
}

pub fn abort_revert(state: SharedState) {
    crate::git::merge::seq_op_pub("revert --abort", state, |path| {
        crate::git::merge::run_git_seq(path, &["revert", "--abort"])
    });
}

// ── Cherry-pick ───────────────────────────────────────────────────────────────

pub fn cherry_pick(oid_str: &str, state: SharedState) {
    let oid = oid_str.to_string();
    crate::git::merge::seq_op_pub("cherry-pick", state, move |path| do_cherry_pick(path, &oid));
}

fn do_cherry_pick(path: &Path, oid_str: &str) -> Result<(), crate::git::error::GitError> {
    let oid = git2::Oid::from_str(oid_str)?;
    // Native Git preserves authorship, hooks and the sequencer's recovery state.
    run_git_seq(path, &["cherry-pick", &oid.to_string()])
}

/// An immutable multi-commit operation, reviewed before entering the mutation queue.
#[derive(Debug, Clone)]
pub struct CommitSequencePreview {
    pub path: std::path::PathBuf,
    pub epoch: u64,
    pub head_ref: String,
    pub head_oid: String,
    /// Selection order is oldest first; reverting reverses this exact list.
    pub oids: Vec<String>,
    pub revert: bool,
}

impl CommitSequencePreview {
    pub fn capture(
        path: &Path,
        epoch: u64,
        oids: Vec<String>,
        revert: bool,
    ) -> Result<Self, String> {
        let repo = git2::Repository::open(path).map_err(|error| error.to_string())?;
        let head = repo
            .head()
            .map_err(|_| "Check out an existing branch before applying commits.")?;
        if !head.is_branch() {
            return Err("Check out a branch before applying commits.".into());
        }
        let preview = Self {
            path: path.canonicalize().map_err(|error| error.to_string())?,
            epoch,
            head_ref: head.name().map_err(|error| error.to_string())?.to_string(),
            head_oid: head
                .peel_to_commit()
                .map_err(|error| error.to_string())?
                .id()
                .to_string(),
            oids,
            revert,
        };
        preview.validate(&preview.path)?;
        Ok(preview)
    }

    pub fn label(&self) -> &'static str {
        if self.revert {
            "Revert"
        } else {
            "Cherry-pick"
        }
    }

    pub fn validate(&self, path: &Path) -> Result<(), String> {
        if path.canonicalize().map_err(|error| error.to_string())? != self.path {
            return Err("Repository changed. Review the selected commits again.".into());
        }
        let repo = git2::Repository::open(path).map_err(|error| error.to_string())?;
        if !super::replacement::refs(&repo)
            .map_err(|error| error.to_string())?
            .is_empty()
        {
            return Err("Multi-commit operations are unavailable while replacement objects are active. Remove or disable replacements before reviewing this operation.".into());
        }
        let head = repo
            .head()
            .map_err(|_| "HEAD changed. Review the selected commits again.")?;
        if !head.is_branch()
            || head.name().ok() != Some(self.head_ref.as_str())
            || head
                .peel_to_commit()
                .map_err(|error| error.to_string())?
                .id()
                .to_string()
                != self.head_oid
        {
            return Err("The current branch or HEAD changed since confirmation. Review the selected commits again.".into());
        }
        if repo.state() != git2::RepositoryState::Clean {
            return Err("Finish or abort the current Git operation first.".into());
        }
        let mut options = git2::StatusOptions::new();
        options.include_untracked(false);
        if !repo
            .statuses(Some(&mut options))
            .map_err(|error| error.to_string())?
            .is_empty()
        {
            return Err("Commit or stash staged and tracked working-tree changes before applying multiple commits.".into());
        }
        validate_sequence_oids(&repo, &self.oids)?;
        Ok(())
    }

    pub(crate) fn execute(&self, path: &Path) -> Result<(), String> {
        self.validate(path)?;
        let mut args: Vec<&str> = if self.revert {
            vec!["revert", "--no-edit", "--"]
        } else {
            vec!["cherry-pick", "--"]
        };
        if self.revert {
            args.extend(self.oids.iter().rev().map(String::as_str));
        } else {
            args.extend(self.oids.iter().map(String::as_str));
        }
        let label = format!("{} {} commits", self.label(), self.oids.len());
        super::recovery::protect_sequence(path, &label, || {
            // Recovery can take time. Check the exact destination again immediately before Git.
            self.validate(path)?;
            run_git_seq(path, &args).map_err(|error| error.user_message())
        })
    }

    pub fn submit(self, state: SharedState) {
        {
            let mut s = state.lock().unwrap();
            if s.repo_epoch != self.epoch
                || s.repo_path
                    .as_ref()
                    .and_then(|path| path.canonicalize().ok())
                    .as_ref()
                    != Some(&self.path)
            {
                s.error = Some(
                    "Repository changed since confirmation. Review the selected commits again."
                        .into(),
                );
                return;
            }
            s.remote_op = Some(format!("{} {} commits", self.label(), self.oids.len()));
            let ordered = if self.revert {
                self.oids.iter().rev().cloned().collect::<Vec<_>>()
            } else {
                self.oids.clone()
            };
            s.console_log.push(format!(
                "$ git {} -- {}",
                if self.revert {
                    "revert --no-edit"
                } else {
                    "cherry-pick"
                },
                ordered.join(" ")
            ));
        }
        let path = self.path.clone();
        let guard_state = state.clone();
        crate::git::mutation::spawn_operation(path, state, move |path| {
            let s = guard_state.lock().unwrap();
            if s.repo_epoch != self.epoch
                || s.repo_path
                    .as_ref()
                    .and_then(|path| path.canonicalize().ok())
                    .as_ref()
                    != Some(&self.path)
            {
                return Err("Repository changed while the confirmed operation was queued. Review the selected commits again.".into());
            }
            drop(s);
            self.execute(path)
        });
    }
}

fn validate_sequence_oids(repo: &git2::Repository, oids: &[String]) -> Result<(), String> {
    if oids.is_empty() {
        return Err("Select at least one commit.".into());
    }
    let mut seen = std::collections::HashSet::new();
    for text in oids {
        if text.len() != 40 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Multi-commit operations require full commit IDs.".into());
        }
        let oid = git2::Oid::from_str(text).map_err(|error| error.to_string())?;
        if !seen.insert(oid) {
            return Err("A commit occurs more than once in the selection.".into());
        }
        let commit = repo.find_commit(oid).map_err(|error| error.to_string())?;
        if commit.parent_count() > 1 {
            return Err(format!("{} is a merge commit. Leave it out of the selection; merges need a mainline parent.", &oid.to_string()[..7]));
        }
    }
    Ok(())
}

/// Request confirmation; no Git mutation occurs until the preview is submitted.
pub fn cherry_pick_many(oids: Vec<String>, state: SharedState) {
    preview_sequence_many(oids, state, false);
}

/// Request confirmation for an exact selection, applied newest first.
pub fn revert_many(oids: Vec<String>, state: SharedState) {
    preview_sequence_many(oids, state, true);
}

fn preview_sequence_many(oids: Vec<String>, state: SharedState, revert: bool) {
    let (path, epoch) = {
        let s = state.lock().unwrap();
        (s.repo_path.clone(), s.repo_epoch)
    };
    let Some(path) = path else { return };
    let result = CommitSequencePreview::capture(&path, epoch, oids, revert);
    let mut s = state.lock().unwrap();
    if s.repo_epoch != epoch || s.repo_path.as_ref() != Some(&path) {
        return;
    }
    match result {
        Ok(preview) => s.modal = crate::state::ModalKind::ConfirmCommitSequence { preview },
        Err(error) => s.error = Some(error),
    }
}

/// Backend fixture entry point. User actions must capture and confirm a preview.
#[cfg(test)]
pub(crate) fn do_sequence_many(path: &Path, oids: &[String], revert: bool) -> Result<(), String> {
    CommitSequencePreview::capture(path, 0, oids.to_vec(), revert)?.execute(path)
}

// ── Tags ──────────────────────────────────────────────────────────────────────

/// Create a tag. If `message` is Some and non-empty, creates an annotated tag;
/// otherwise creates a lightweight tag.
pub fn create_tag(name: &str, from_oid: Option<&str>, message: Option<&str>, state: SharedState) {
    let name = name.to_string();
    let from_oid = from_oid.map(str::to_owned);
    let message = message.map(str::to_owned);
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    crate::git::mutation::spawn_operation(path, state, move |path| {
        do_create_tag(path, &name, from_oid.as_deref(), message.as_deref())
            .map_err(|e| e.message().to_string())
    });
}

fn do_create_tag(
    path: &Path,
    name: &str,
    from_oid: Option<&str>,
    message: Option<&str>,
) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(path)?;
    let obj = if let Some(oid_str) = from_oid {
        let oid = git2::Oid::from_str(oid_str)?;
        repo.find_object(oid, None)?
    } else {
        repo.head()?.peel(git2::ObjectType::Any)?
    };
    match message {
        Some(msg) if !msg.trim().is_empty() => {
            let sig = repo.signature()?;
            repo.tag(name, &obj, &sig, msg.trim(), false)?;
        }
        _ => {
            repo.tag_lightweight(name, &obj, false)?;
        }
    }
    Ok(())
}

pub fn push_tag(name: &str, state: SharedState) {
    remote_tag(name, false, state);
}

pub fn delete_remote_tag(name: &str, state: SharedState) {
    remote_tag(name, true, state);
}

pub fn delete_tag(name: &str, state: SharedState) {
    let name = name.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    crate::git::mutation::spawn_operation(path, state, move |path| {
        git2::Repository::open(path)
            .and_then(|repo| repo.tag_delete(&name))
            .map_err(|e| e.to_string())
    });
}

fn remote_tag(name: &str, delete: bool, state: SharedState) {
    let n = name.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    crate::git::mutation::spawn_operation(path, state, move |path| do_remote_tag(path, &n, delete));
}

fn do_remote_tag(path: &Path, name: &str, delete: bool) -> Result<(), String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let remote = super::remote::default_push_remote(&repo).map_err(|e| e.user_message())?;
    let reference = format!("refs/tags/{name}");
    let spec = if delete {
        format!(":{reference}")
    } else {
        format!("{reference}:{reference}")
    };
    run_git_seq(path, &["push", "--", &remote, &spec]).map_err(|e| e.user_message())
}

// ── Revert commit ─────────────────────────────────────────────────────────────

pub fn revert_commit(oid_str: &str, state: SharedState) {
    let oid = oid_str.to_string();
    crate::git::merge::seq_op_pub("revert", state, move |path| do_revert(path, &oid));
}

fn do_revert(path: &Path, oid_str: &str) -> Result<(), crate::git::error::GitError> {
    let oid = git2::Oid::from_str(oid_str)?;
    run_git_seq(path, &["revert", "--no-edit", &oid.to_string()])
}

// ── Collect stashes (used by load_repo_inner) ────────────────────────────────

pub fn collect_stashes(repo: &mut git2::Repository) -> Vec<crate::state::StashEntry> {
    let mut stashes = Vec::new();
    let _ = repo.stash_foreach(|idx, msg, oid| {
        stashes.push(crate::state::StashEntry {
            index: idx,
            message: msg.to_string(),
            oid: oid.to_string(),
        });
        true
    });
    stashes
}
