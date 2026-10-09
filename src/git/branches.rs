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

    crate::git::mutation::spawn_operation(path, state, move |path| super::recovery::protect(path, "Switch branch", || do_checkout(path, &name).map_err(|e| e.message().to_string())));
}

fn do_checkout(path: &Path, name: &str) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(path)?;
    repo.find_branch(name, git2::BranchType::Local)?;
    run_git_seq(path, &["checkout", name, "--"]).map_err(|e| git2::Error::from_str(&e.user_message()))
}

// ── Create branch ─────────────────────────────────────────────────────────────

/// Create a new branch. `from_oid` = commit to branch from (None = HEAD).
pub fn create_branch(name: &str, from_oid: Option<&str>, checkout: bool, state: SharedState) {
    let name = name.to_string();
    let from_oid = from_oid.map(str::to_owned);
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };

    crate::git::mutation::spawn_operation(path, state, move |path| do_create(path, &name, from_oid.as_deref(), checkout).map_err(|e| e.message().to_string()));
}

fn do_create(path: &Path, name: &str, from_oid: Option<&str>, checkout: bool) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(path)?;

    let commit = if let Some(oid_str) = from_oid {
        let oid = git2::Oid::from_str(oid_str)?;
        repo.find_commit(oid)?
    } else {
        repo.head()?.peel_to_commit()?
    };

    if !git2::Branch::name_is_valid(name)? { return Err(git2::Error::from_str("Invalid branch name.")); }
    let oid = commit.id().to_string();
    let args = if checkout { vec!["checkout", "-b", name, &oid, "--"] } else { vec!["branch", "--", name, &oid] };
    run_git_seq(path, &args).map_err(|e| git2::Error::from_str(&e.user_message()))
}

// ── Rename branch ────────────────────────────────────────────────────────────

pub fn rename_branch(old_name: &str, new_name: &str, state: SharedState) {
    let old = old_name.to_string();
    let new = new_name.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    crate::git::mutation::spawn_operation(path, state, move |path| do_rename(path, &old, &new).map_err(|e| e.message().to_string()));
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
        let oid = repo.find_branch(&n, git2::BranchType::Local).and_then(|b| b.get().peel_to_commit()).map_err(|e| e.to_string())?.id().to_string();
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
        if s.repo_path.as_deref() == Some(path) { s.deleted_branches.retain(|d| d.name != n); }
        if let Some(view) = s.repo_views.get_mut(path) { view.deleted_branches.retain(|d| d.name != n); }
        Ok(())
    });
}

fn deleted_reference(name: &str, oid: &str) -> String {
    let encoded: String = name.as_bytes().iter().map(|byte| format!("{byte:02x}")).collect();
    format!("refs/recorde/deleted-branches/{encoded}/{oid}")
}

fn save_deleted(repo: &git2::Repository, name: &str, oid: &str) -> Result<(), git2::Error> {
    let reference = deleted_reference(name, oid);
    let target = git2::Oid::from_str(oid)?;
    if repo.find_reference(&reference).is_ok_and(|r| r.target() == Some(target)) { return Ok(()); }
    repo.reference(&reference, target, false, "Recorde deleted branch recovery")?;
    Ok(())
}

pub(crate) fn load_deleted(path: &Path) -> Vec<crate::state::DeletedBranch> {
    let Ok(repo) = git2::Repository::open(path) else { return Vec::new() };
    let Ok(references) = repo.references_glob("refs/recorde/deleted-branches/*") else { return Vec::new() };
    references.flatten().filter_map(|reference| {
        let reference_name = reference.name().ok()?;
        let tail = reference_name.strip_prefix("refs/recorde/deleted-branches/")?;
        let (encoded, oid) = tail.split_once('/')?;
        if encoded.len() % 2 != 0 || !encoded.is_ascii() { return None; }
        let bytes: Option<Vec<u8>> = (0..encoded.len()).step_by(2).map(|i| u8::from_str_radix(&encoded[i..i+2], 16).ok()).collect();
        let name = String::from_utf8(bytes?).ok()?;
        if !git2::Branch::name_is_valid(&name).ok()? || repo.find_branch(&name, git2::BranchType::Local).is_ok() { return None; }
        if reference.target()? != git2::Oid::from_str(oid).ok()? { return None; }
        Some(crate::state::DeletedBranch { name, oid: oid.into() })
    }).collect()
}

fn push_deleted(state: &SharedState, path: &Path, name: String, oid: String) {
    let mut s = state.lock().unwrap();
    let deleted = if s.repo_path.as_deref() == Some(path) { &mut s.deleted_branches }
        else { &mut s.repo_views.entry(path.into()).or_default().deleted_branches };
    deleted.push(crate::state::DeletedBranch { name, oid });
    if deleted.len() > 5 { deleted.remove(0); }
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
        run_git_seq(path, &["push", "-u", "--", &remote, &format!("{source}:{source}")]).map_err(|e| e.user_message())
    });
}

// ── Set upstream ──────────────────────────────────────────────────────────────

pub fn set_upstream(branch: &str, upstream: &str, state: SharedState) {
    let b = branch.to_string();
    let target = upstream.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    crate::git::mutation::spawn_operation(path, state, move |path| {
        run_git_seq(path, &["branch", &format!("--set-upstream-to={target}"), "--", &b]).map_err(|e| e.user_message())
    });
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
    crate::git::mutation::spawn_operation(path, state, move |path| super::recovery::protect(path, "Detached checkout", || do_checkout_detached(path, &oid_s).map_err(|e| e.message().to_string())));
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
    run_git_seq(path, &["branch", "-d", "--", name]).map_err(|e| git2::Error::from_str(&e.user_message()))?;
    Ok(oid)
}

// ── Stash ─────────────────────────────────────────────────────────────────────

pub fn stash_save(state: SharedState) {
    stash_save_with_message("WIP", state);
}

pub fn stash_save_with_message(message: &str, state: SharedState) {
    let msg = message.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    crate::git::mutation::spawn_operation(path, state, move |path| run_git_seq(path, &["stash", "push", "-m", &msg]).map_err(|e| e.user_message()));
}

pub fn stash_file(file_path: &str, state: SharedState) {
    let fp = file_path.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    crate::git::mutation::spawn_operation(path, state, move |path| run_git_seq(path, &["--literal-pathspecs", "stash", "push", "--", &fp]).map_err(|e| e.user_message()));
}

pub fn stash_branch(branch_name: &str, stash_index: usize, state: SharedState) {
    let name = branch_name.to_string();
    let stash_ref = format!("stash@{{{}}}", stash_index);
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    crate::git::mutation::spawn_operation(path, state, move |path| run_git_seq(path, &["stash", "branch", "--", &name, &stash_ref]).map_err(|e| e.user_message()));
}

fn do_stash_save(_path: &Path) -> Result<(), git2::Error> { Ok(()) } // kept for ABI compat, unused

pub fn stash_apply(index: usize, state: SharedState) {
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };

    crate::git::mutation::spawn_operation(path, state, move |path| do_stash_apply(path, index, false).map_err(|e| e.message().to_string()));
}

pub fn stash_pop(index: usize, state: SharedState) {
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };

    crate::git::mutation::spawn_operation(path, state, move |path| do_stash_apply(path, index, true).map_err(|e| e.message().to_string()));
}

fn do_stash_apply(path: &Path, index: usize, pop: bool) -> Result<(), git2::Error> {
    let mut repo = git2::Repository::open(path)?;
    let mut opts = git2::StashApplyOptions::new();
    if pop {
        repo.stash_pop(index, Some(&mut opts))?;
    } else {
        repo.stash_apply(index, Some(&mut opts))?;
    }
    Ok(())
}

pub fn stash_drop(index: usize, state: SharedState) {
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };

    crate::git::mutation::spawn_operation(path, state, move |path| do_stash_drop(path, index).map_err(|e| e.message().to_string()));
}

fn do_stash_drop(path: &Path, index: usize) -> Result<(), git2::Error> {
    let mut repo = git2::Repository::open(path)?;
    repo.stash_drop(index)?;
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
    crate::git::merge::seq_op_pub("cherry-pick", state, move |path| {
        do_cherry_pick(path, &oid)
    });
}

fn do_cherry_pick(path: &Path, oid_str: &str) -> Result<(), crate::git::error::GitError> {
    let oid = git2::Oid::from_str(oid_str)?;
    // Native Git preserves authorship, hooks and the sequencer's recovery state.
    run_git_seq(path, &["cherry-pick", &oid.to_string()])
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
    crate::git::mutation::spawn_operation(path, state, move |path| do_create_tag(path, &name, from_oid.as_deref(), message.as_deref()).map_err(|e| e.message().to_string()));
}

fn do_create_tag(path: &Path, name: &str, from_oid: Option<&str>, message: Option<&str>) -> Result<(), git2::Error> {
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
        _ => { repo.tag_lightweight(name, &obj, false)?; }
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
        git2::Repository::open(path).and_then(|repo| repo.tag_delete(&name)).map_err(|e| e.to_string())
    });
}

fn remote_tag(name: &str, delete: bool, state: SharedState) {
    let n = name.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    crate::git::mutation::spawn_operation(path, state, move |path| {
        let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
        let remote = super::remote::default_push_remote(&repo).map_err(|e| e.user_message())?;
        let reference = format!("refs/tags/{n}");
        let spec = if delete { format!(":{reference}") } else { format!("{reference}:{reference}") };
        run_git_seq(path, &["push", "--", &remote, &spec]).map_err(|e| e.user_message())
    });
}

// ── Revert commit ─────────────────────────────────────────────────────────────

pub fn revert_commit(oid_str: &str, state: SharedState) {
    let oid = oid_str.to_string();
    crate::git::merge::seq_op_pub("revert", state, move |path| {
        do_revert(path, &oid)
    });
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
