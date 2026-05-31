use std::path::Path;

use crate::git::{merge::run_git_seq, patch};
use crate::state::{PendingDiscard, SharedState};

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

pub fn create_commit(message: &str, signoff: bool, state: SharedState) {
    let msg = message.to_string();
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else { return };
    std::thread::spawn(move || {
        match git_commit(&rp, &msg, false, signoff) {
            Ok(out) => {
                state.lock().unwrap().console_log.push(out.trim().to_string());
                reload(state);
            }
            Err(e) => {
                let mut s = state.lock().unwrap();
                s.console_log.push(format!("error: {}", e.trim()));
                s.error = Some(e.trim().to_string());
            }
        }
    });
}

pub fn amend_commit(message: &str, signoff: bool, state: SharedState) {
    let msg = message.to_string();
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else { return };
    std::thread::spawn(move || {
        match git_commit(&rp, &msg, true, signoff) {
            Ok(out) => {
                state.lock().unwrap().console_log.push(out.trim().to_string());
                reload(state);
            }
            Err(e) => {
                let mut s = state.lock().unwrap();
                s.console_log.push(format!("error: {}", e.trim()));
                s.error = Some(e.trim().to_string());
            }
        }
    });
}

// ── Reset ─────────────────────────────────────────────────────────────────────

pub fn reset_soft(state: SharedState) {
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else { return };
    std::thread::spawn(move || match run_git_seq(&rp, &["reset", "--soft", "HEAD~1"]) {
        Ok(()) => reload(state),
        Err(e) => state.lock().unwrap().error = Some(e.user_message()),
    });
}

pub fn reset_mixed(state: SharedState) {
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else { return };
    std::thread::spawn(move || match run_git_seq(&rp, &["reset", "--mixed", "HEAD~1"]) {
        Ok(()) => reload(state),
        Err(e) => state.lock().unwrap().error = Some(e.user_message()),
    });
}

pub fn reset_hard(state: SharedState) {
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else {
        state.lock().unwrap().pending_discard = None;
        return;
    };
    std::thread::spawn(move || {
        let result = run_git_seq(&rp, &["reset", "--hard", "HEAD~1"]);
        state.lock().unwrap().pending_discard = None;
        match result {
            Ok(()) => reload(state),
            Err(e) => state.lock().unwrap().error = Some(e.user_message()),
        }
    });
}

/// Load the HEAD commit message into `out` (used to pre-populate the amend form).
pub fn load_head_message(state: SharedState) -> Option<String> {
    let rp = state.lock().unwrap().repo_path.clone()?;
    let repo = git2::Repository::open(&rp).ok()?;
    let msg = repo.head().ok()?.peel_to_commit().ok()?.message()?.to_string();
    Some(msg)
}

// ── Hunk / line staging ───────────────────────────────────────────────────────

pub fn stage_hunk(hunk_index: usize, state: SharedState) {
    let Some((rp, file_path, hunk)) = pluck_hunk(hunk_index, &state) else { return };
    std::thread::spawn(move || {
        let p = patch::hunk_patch(&file_path, &hunk);
        match patch::apply_to_index(&rp, &p) {
            Ok(()) => reload(state),
            Err(e) => state.lock().unwrap().error = Some(e.user_message()),
        }
    });
}

pub fn unstage_hunk(hunk_index: usize, state: SharedState) {
    let Some((rp, file_path, hunk)) = pluck_hunk(hunk_index, &state) else { return };
    std::thread::spawn(move || {
        let p = patch::hunk_patch(&file_path, &hunk);
        match patch::remove_from_index(&rp, &p) {
            Ok(()) => reload(state),
            Err(e) => state.lock().unwrap().error = Some(e.user_message()),
        }
    });
}

pub fn discard_hunk(hunk_index: usize, state: SharedState) {
    let Some((rp, file_path, hunk)) = pluck_hunk(hunk_index, &state) else {
        state.lock().unwrap().pending_discard = None;
        return;
    };
    std::thread::spawn(move || {
        let p = patch::hunk_patch(&file_path, &hunk);
        let mut s = state.lock().unwrap();
        s.pending_discard = None;
        match patch::discard_from_workdir(&rp, &p) {
            Ok(()) => {
                drop(s);
                reload(state);
            }
            Err(e) => s.error = Some(e.user_message()),
        }
    });
}

pub fn stage_lines(hunk_index: usize, line_indices: Vec<usize>, state: SharedState) {
    let Some((rp, file_path, hunk)) = pluck_hunk(hunk_index, &state) else { return };
    std::thread::spawn(move || {
        let p = patch::lines_patch(&file_path, &hunk, &line_indices);
        match patch::apply_to_index(&rp, &p) {
            Ok(()) => reload(state),
            Err(e) => state.lock().unwrap().error = Some(e.user_message()),
        }
    });
}

pub fn unstage_lines(hunk_index: usize, line_indices: Vec<usize>, state: SharedState) {
    let Some((rp, file_path, hunk)) = pluck_hunk(hunk_index, &state) else { return };
    std::thread::spawn(move || {
        let p = patch::lines_patch(&file_path, &hunk, &line_indices);
        match patch::remove_from_index(&rp, &p) {
            Ok(()) => reload(state),
            Err(e) => state.lock().unwrap().error = Some(e.user_message()),
        }
    });
}

/// Discard all unstaged changes to a file (restore from index).
pub fn discard_file(file_path: &str, state: SharedState) {
    let fp = file_path.to_string();
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else {
        state.lock().unwrap().pending_discard = None;
        return;
    };
    std::thread::spawn(move || {
        let result = do_discard_file(&rp, &fp);
        let mut s = state.lock().unwrap();
        s.pending_discard = None;
        match result {
            Ok(()) => {
                drop(s);
                reload(state);
            }
            Err(e) => s.error = Some(e.message().to_string()),
        }
    });
}

// Read (repo_path, file_path, hunk_clone) from state — returns None if anything is missing.
fn pluck_hunk(
    hunk_index: usize,
    state: &SharedState,
) -> Option<(std::path::PathBuf, String, crate::state::DiffHunk)> {
    let s = state.lock().unwrap();
    let rp = s.repo_path.clone()?;
    let diff = s.current_diff.as_ref()?;
    let file_path = diff.file_path.clone();
    let hunk = diff.hunks.get(hunk_index)?.clone();
    Some((rp, file_path, hunk))
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

/// Shell out to `git commit -m <msg>` so hooks (pre-commit, commit-msg, post-commit) run.
/// Returns combined stdout+stderr on success, Err(combined) on failure.
fn git_commit(repo_path: &Path, message: &str, amend: bool, signoff: bool) -> Result<String, String> {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C").arg(repo_path)
       .arg("commit")
       .arg("-m").arg(message);
    if amend   { cmd.arg("--amend"); }
    if signoff { cmd.arg("--signoff"); }
    cmd.env("GIT_TERMINAL_PROMPT", "0");

    let out = cmd.output().map_err(|e| e.to_string())?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    if out.status.success() { Ok(combined) } else { Err(combined) }
}

fn do_discard_file(repo_path: &Path, file_path: &str) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(repo_path)?;
    let mut co = git2::build::CheckoutBuilder::new();
    co.path(file_path).force();
    repo.checkout_index(None, Some(&mut co))?;
    Ok(())
}
