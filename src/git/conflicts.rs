use std::path::Path;

use crate::git::merge::run_git_seq;
use crate::state::SharedState;

/// Resolve a conflicted file by taking the current-branch (ours) version.
/// Runs `git checkout --ours <path>` then `git add <path>`.
pub fn choose_ours(file_path: &str, state: SharedState) {
    resolve_with(file_path, state, "ours");
}

/// Resolve a conflicted file by taking the incoming (theirs) version.
/// Runs `git checkout --theirs <path>` then `git add <path>`.
pub fn choose_theirs(file_path: &str, state: SharedState) {
    resolve_with(file_path, state, "theirs");
}

/// Mark a file as resolved after manual editing.
/// Runs `git add <path>`.
pub fn mark_resolved(file_path: &str, state: SharedState) {
    let fp = file_path.to_string();
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else { return };

    std::thread::spawn(move || {
        let result = run_git_seq(&rp, &["add", &fp]);
        if let Err(ref e) = result {
            state.lock().unwrap().error = Some(e.user_message());
        }
        crate::git::branches::reload_repo(state);
    });
}

/// Launch the configured merge tool for a conflicted file (fire-and-forget).
pub fn open_mergetool(file_path: &str, repo_path: &Path) {
    let _ = std::process::Command::new("git")
        .arg("-C").arg(repo_path)
        .args(["mergetool", "--no-prompt", file_path])
        .spawn();
}

// ── Internal ──────────────────────────────────────────────────────────────────

fn resolve_with(file_path: &str, state: SharedState, side: &'static str) {
    let fp = file_path.to_string();
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else { return };

    std::thread::spawn(move || {
        let result = run_git_seq(&rp, &["checkout", &format!("--{}", side), &fp])
            .and_then(|()| run_git_seq(&rp, &["add", &fp]));
        if let Err(ref e) = result {
            state.lock().unwrap().error = Some(e.user_message());
        }
        crate::git::branches::reload_repo(state);
    });
}
