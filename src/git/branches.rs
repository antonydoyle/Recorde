use std::path::Path;

use crate::git::merge::run_git_seq;
use crate::state::SharedState;

// ── Re-exported reload helper ────────────────────────────────────────────────

/// Reload the graph + status for the currently open repo.
/// Spawns its own thread — safe to call from any thread.
pub fn reload_repo(state: SharedState) {
    let path = state.lock().unwrap().repo_path.clone();
    if let Some(path) = path {
        crate::git::operations::load_repo(path, state);
    }
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

    std::thread::spawn(move || match do_checkout(&path, &name) {
        Ok(()) => reload_repo(state),
        Err(e) => {
            let mut s = state.lock().unwrap();
            s.remote_op = None;
            s.error = Some(e.message().to_string());
            s.console_log.push(format!("error: {}", e.message()));
        }
    });
}

fn do_checkout(path: &Path, name: &str) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(path)?;
    let branch = repo.find_branch(name, git2::BranchType::Local)?;
    let obj = branch.get().peel(git2::ObjectType::Commit)?;
    let mut opts = git2::build::CheckoutBuilder::new();
    opts.safe();
    repo.checkout_tree(&obj, Some(&mut opts))?;
    repo.set_head(&format!("refs/heads/{}", name))?;
    Ok(())
}

// ── Create branch ─────────────────────────────────────────────────────────────

/// Create a new branch. `from_oid` = commit to branch from (None = HEAD).
pub fn create_branch(name: &str, from_oid: Option<&str>, checkout: bool, state: SharedState) {
    let name = name.to_string();
    let from_oid = from_oid.map(str::to_owned);
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };

    std::thread::spawn(move || {
        match do_create(&path, &name, from_oid.as_deref(), checkout) {
            Ok(()) => reload_repo(state),
            Err(e) => state.lock().unwrap().error = Some(e.message().to_string()),
        }
    });
}

fn do_create(path: &Path, name: &str, from_oid: Option<&str>, checkout: bool) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(path)?;

    let commit = if let Some(oid_str) = from_oid {
        let oid = git2::Oid::from_str(oid_str)?;
        repo.find_commit(oid)?
    } else {
        repo.head()?.peel_to_commit()?
    };

    repo.branch(name, &commit, false)?;

    if checkout {
        let obj = commit.as_object();
        let mut opts = git2::build::CheckoutBuilder::new();
        opts.safe();
        repo.checkout_tree(obj, Some(&mut opts))?;
        repo.set_head(&format!("refs/heads/{}", name))?;
    }
    Ok(())
}

// ── Rename branch ────────────────────────────────────────────────────────────

pub fn rename_branch(old_name: &str, new_name: &str, state: SharedState) {
    let old = old_name.to_string();
    let new = new_name.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    std::thread::spawn(move || {
        match do_rename(&path, &old, &new) {
            Ok(()) => reload_repo(state),
            Err(e) => state.lock().unwrap().error = Some(e.message().to_string()),
        }
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
    std::thread::spawn(move || {
        match run_git_seq(&path, &["branch", "-D", &n]) {
            Ok(()) => reload_repo(state),
            Err(e) => state.lock().unwrap().error = Some(e.user_message()),
        }
    });
}

// ── Publish branch ────────────────────────────────────────────────────────────

pub fn publish_branch(name: &str, state: SharedState) {
    let n = name.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    {
        let mut s = state.lock().unwrap();
        s.remote_op = Some(format!("Publishing {}…", n));
        s.console_log.push(format!("$ git push -u origin {}", n));
    }
    std::thread::spawn(move || {
        let result = run_git_seq(&path, &["push", "-u", "origin", &n]);
        state.lock().unwrap().remote_op = None;
        match result {
            Ok(()) => reload_repo(state),
            Err(e) => state.lock().unwrap().error = Some(e.user_message()),
        }
    });
}

// ── Set upstream ──────────────────────────────────────────────────────────────

pub fn set_upstream(branch: &str, remote: &str, state: SharedState) {
    let b = branch.to_string();
    let r = remote.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    std::thread::spawn(move || {
        let upstream = format!("{}/{}", r, b);
        match run_git_seq(&path, &["branch", &format!("--set-upstream-to={}", upstream), &b]) {
            Ok(()) => reload_repo(state),
            Err(e) => state.lock().unwrap().error = Some(e.user_message()),
        }
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
    std::thread::spawn(move || {
        match do_checkout_detached(&path, &oid_s) {
            Ok(()) => {
                state.lock().unwrap().remote_op = None;
                reload_repo(state);
            }
            Err(e) => {
                let mut s = state.lock().unwrap();
                s.remote_op = None;
                s.error = Some(e.message().to_string());
            }
        }
    });
}

fn do_checkout_detached(path: &Path, oid_str: &str) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(path)?;
    let oid = git2::Oid::from_str(oid_str)?;
    repo.set_head_detached(oid)?;
    let mut opts = git2::build::CheckoutBuilder::new();
    opts.force();
    repo.checkout_head(Some(&mut opts))?;
    Ok(())
}

// ── Delete branch ─────────────────────────────────────────────────────────────

pub fn delete_branch(branch_name: &str, state: SharedState) {
    let name = branch_name.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };

    std::thread::spawn(move || match do_delete(&path, &name) {
        Ok(()) => reload_repo(state),
        Err(e) => state.lock().unwrap().error = Some(e.message().to_string()),
    });
}

fn do_delete(path: &Path, name: &str) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(path)?;
    let mut branch = repo.find_branch(name, git2::BranchType::Local)?;
    branch.delete()?;
    Ok(())
}

// ── Stash ─────────────────────────────────────────────────────────────────────

pub fn stash_save(state: SharedState) {
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };

    std::thread::spawn(move || match do_stash_save(&path) {
        Ok(()) => reload_repo(state),
        Err(e) => state.lock().unwrap().error = Some(e.message().to_string()),
    });
}

fn do_stash_save(path: &Path) -> Result<(), git2::Error> {
    let mut repo = git2::Repository::open(path)?;
    let sig = repo.signature()?;
    repo.stash_save(&sig, "WIP", Some(git2::StashFlags::DEFAULT))?;
    Ok(())
}

pub fn stash_apply(index: usize, state: SharedState) {
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };

    std::thread::spawn(move || match do_stash_apply(&path, index, false) {
        Ok(()) => reload_repo(state),
        Err(e) => state.lock().unwrap().error = Some(e.message().to_string()),
    });
}

pub fn stash_pop(index: usize, state: SharedState) {
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };

    std::thread::spawn(move || match do_stash_apply(&path, index, true) {
        Ok(()) => reload_repo(state),
        Err(e) => state.lock().unwrap().error = Some(e.message().to_string()),
    });
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

    std::thread::spawn(move || match do_stash_drop(&path, index) {
        Ok(()) => reload_repo(state),
        Err(e) => state.lock().unwrap().error = Some(e.message().to_string()),
    });
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
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    std::thread::spawn(move || match do_cherry_pick(&path, &oid) {
        Ok(()) => reload_repo(state),
        Err(e) => state.lock().unwrap().error = Some(e.message().to_string()),
    });
}

fn do_cherry_pick(path: &Path, oid_str: &str) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(path)?;
    let oid = git2::Oid::from_str(oid_str)?;
    let commit = repo.find_commit(oid)?;
    repo.cherrypick(&commit, None)?;

    let mut index = repo.index()?;
    if index.has_conflicts() {
        repo.cleanup_state()?;
        return Err(git2::Error::from_str("Cherry-pick has conflicts — resolve manually"));
    }

    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let sig = repo.signature()?;
    let head = repo.head()?.peel_to_commit()?;
    let msg = commit.message().unwrap_or("cherry-pick");
    repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &[&head])?;
    repo.cleanup_state()?;
    Ok(())
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
    std::thread::spawn(move || match do_create_tag(&path, &name, from_oid.as_deref(), message.as_deref()) {
        Ok(()) => reload_repo(state),
        Err(e) => state.lock().unwrap().error = Some(e.message().to_string()),
    });
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
    let n = name.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    std::thread::spawn(move || match run_git_seq(&path, &["push", "origin", &n]) {
        Ok(()) => reload_repo(state),
        Err(e) => state.lock().unwrap().error = Some(e.user_message()),
    });
}

pub fn delete_remote_tag(name: &str, state: SharedState) {
    let n = name.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    std::thread::spawn(move || match run_git_seq(&path, &["push", "origin", "--delete", &n]) {
        Ok(()) => reload_repo(state),
        Err(e) => state.lock().unwrap().error = Some(e.user_message()),
    });
}

pub fn delete_tag(name: &str, state: SharedState) {
    let name = name.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    std::thread::spawn(move || match do_delete_tag(&path, &name) {
        Ok(()) => reload_repo(state),
        Err(e) => state.lock().unwrap().error = Some(e.message().to_string()),
    });
}

fn do_delete_tag(path: &Path, name: &str) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(path)?;
    repo.tag_delete(name)?;
    Ok(())
}

// ── Revert commit ─────────────────────────────────────────────────────────────

pub fn revert_commit(oid_str: &str, state: SharedState) {
    let oid = oid_str.to_string();
    let path = state.lock().unwrap().repo_path.clone();
    let Some(path) = path else { return };
    std::thread::spawn(move || match do_revert(&path, &oid) {
        Ok(()) => reload_repo(state),
        Err(e) => state.lock().unwrap().error = Some(e.message().to_string()),
    });
}

fn do_revert(path: &Path, oid_str: &str) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(path)?;
    let oid    = git2::Oid::from_str(oid_str)?;
    let commit = repo.find_commit(oid)?;
    let mut opts = git2::RevertOptions::new();
    repo.revert(&commit, Some(&mut opts))?;
    // Create the revert commit
    let mut index = repo.index()?;
    if index.has_conflicts() {
        repo.cleanup_state()?;
        return Err(git2::Error::from_str("Revert has conflicts — resolve manually"));
    }
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let sig  = repo.signature()?;
    let head = repo.head()?.peel_to_commit()?;
    let msg  = format!("Revert \"{}\"", commit.summary().unwrap_or(""));
    repo.commit(Some("HEAD"), &sig, &sig, &msg, &tree, &[&head])?;
    repo.cleanup_state()?;
    Ok(())
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
