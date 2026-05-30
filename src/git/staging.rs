use std::path::Path;

use crate::state::SharedState;

fn reload(state: SharedState) {
    crate::git::branches::reload_repo(state);
}

// ── Stage / Unstage individual files ─────────────────────────────────────────

pub fn stage_file(file_path: &str, state: SharedState) {
    let fp = file_path.to_string();
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else { return };
    std::thread::spawn(move || match do_stage(&rp, &fp) {
        Ok(()) => reload(state),
        Err(e) => state.lock().unwrap().error = Some(e.message().to_string()),
    });
}

pub fn unstage_file(file_path: &str, state: SharedState) {
    let fp = file_path.to_string();
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else { return };
    std::thread::spawn(move || match do_unstage(&rp, &fp) {
        Ok(()) => reload(state),
        Err(e) => state.lock().unwrap().error = Some(e.message().to_string()),
    });
}

// ── Stage / Unstage all ───────────────────────────────────────────────────────

pub fn stage_all(state: SharedState) {
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else { return };
    std::thread::spawn(move || match do_stage_all(&rp) {
        Ok(()) => reload(state),
        Err(e) => state.lock().unwrap().error = Some(e.message().to_string()),
    });
}

pub fn unstage_all(state: SharedState) {
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else { return };
    std::thread::spawn(move || match do_unstage_all(&rp) {
        Ok(()) => reload(state),
        Err(e) => state.lock().unwrap().error = Some(e.message().to_string()),
    });
}

// ── Commit ────────────────────────────────────────────────────────────────────

pub fn create_commit(message: &str, state: SharedState) {
    let msg = message.to_string();
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else { return };
    std::thread::spawn(move || match do_commit(&rp, &msg) {
        Ok(()) => reload(state),
        Err(e) => state.lock().unwrap().error = Some(e.message().to_string()),
    });
}

pub fn amend_commit(message: &str, state: SharedState) {
    let msg = message.to_string();
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else { return };
    std::thread::spawn(move || match do_amend(&rp, &msg) {
        Ok(()) => reload(state),
        Err(e) => state.lock().unwrap().error = Some(e.message().to_string()),
    });
}

/// Load the HEAD commit message into `out` (used to pre-populate the amend form).
pub fn load_head_message(state: SharedState) -> Option<String> {
    let rp = state.lock().unwrap().repo_path.clone()?;
    let repo = git2::Repository::open(&rp).ok()?;
    let msg = repo.head().ok()?.peel_to_commit().ok()?.message()?.to_string();
    Some(msg)
}

// ── Internals ─────────────────────────────────────────────────────────────────

fn do_stage(repo_path: &Path, file_path: &str) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(repo_path)?;
    let mut index = repo.index()?;
    let on_disk = repo_path.join(file_path);
    if on_disk.exists() {
        index.add_path(Path::new(file_path))?;
    } else {
        // Stage the deletion
        index.remove_path(Path::new(file_path))?;
    }
    index.write()?;
    Ok(())
}

fn do_unstage(repo_path: &Path, file_path: &str) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(repo_path)?;
    // reset_default = git reset HEAD -- <path>
    let target = repo.head().ok().and_then(|h| h.peel(git2::ObjectType::Commit).ok());
    if let Some(ref obj) = target {
        repo.reset_default(Some(obj), &[file_path])?;
    } else {
        // Initial commit: just remove the path from the index
        let mut index = repo.index()?;
        index.remove_path(Path::new(file_path))?;
        index.write()?;
    }
    Ok(())
}

fn do_stage_all(repo_path: &Path) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(repo_path)?;
    let mut index = repo.index()?;
    index.add_all(["*"], git2::IndexAddOption::DEFAULT, None)?;
    index.write()?;
    Ok(())
}

fn do_unstage_all(repo_path: &Path) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(repo_path)?;
    let target = repo.head().ok().and_then(|h| h.peel(git2::ObjectType::Commit).ok());
    if let Some(ref obj) = target {
        repo.reset_default(Some(obj), &["*"])?;
    } else {
        // Initial commit: clear the index entirely
        let mut index = repo.index()?;
        index.clear()?;
        index.write()?;
    }
    Ok(())
}

fn do_commit(repo_path: &Path, message: &str) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(repo_path)?;
    let mut index = repo.index()?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let sig = repo.signature()?;
    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)?;
    Ok(())
}

fn do_amend(repo_path: &Path, message: &str) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(repo_path)?;
    let mut index = repo.index()?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let sig = repo.signature()?;
    let head = repo.head()?.peel_to_commit()?;
    head.amend(Some("HEAD"), Some(&sig), Some(&sig), None, Some(message), Some(&tree))?;
    Ok(())
}
