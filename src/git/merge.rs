use std::path::Path;

use crate::git::error::GitError;
use crate::state::SharedState;

// ── Shared shell helper ───────────────────────────────────────────────────────

/// Run `git -C <repo_path> <args>` and return stderr as a GitError on failure.
pub(crate) fn run_git_seq(repo_path: &Path, args: &[&str]) -> Result<(), GitError> {
    crate::git::process::git(repo_path, args, &std::sync::atomic::AtomicBool::new(false)).map(|_| ())

}

// ── Merge ─────────────────────────────────────────────────────────────────────

/// Merge `branch_name` into the current branch.
/// Aborts with an error if the working tree is dirty.
pub fn merge_branch(branch_name: &str, state: SharedState) {
    let name = branch_name.to_string();
    let (path, dirty) = {
        let s = state.lock().unwrap();
        (s.repo_path.clone(), s.dirty_count)
    };
    let Some(path) = path else { return };

    if dirty > 0 {
        state.lock().unwrap().error =
            Some("Stash or commit changes before merging.".into());
        return;
    }

    {
        let mut s = state.lock().unwrap();
        s.remote_op = Some(format!("Merging {}…", name));
        s.console_log.push(format!("$ git merge {}", name));
    }

    crate::git::mutation::spawn_operation(path, state, move |path| {
        super::recovery::protect_sequence(path, "Merge", || run_git_seq(path, &["merge", "--no-edit", "--", &name]).map_err(|e| e.user_message()))
    });
}

pub fn continue_merge(state: SharedState) {
    seq_op("merge --continue", state, |path| {
        run_git_seq(path, &["merge", "--continue"])
    });
}

pub fn abort_merge(state: SharedState) {
    seq_op("merge --abort", state, |path| run_git_seq(path, &["merge", "--abort"]));
}

// ── Shared pattern for continue/abort/skip ops ────────────────────────────────

pub(crate) fn seq_op_pub(label: &'static str, state: SharedState, f: impl FnOnce(&Path) -> Result<(), GitError> + Send + 'static) {
    seq_op(label, state, f);
}

fn seq_op(label: &'static str, state: SharedState, f: impl FnOnce(&Path) -> Result<(), GitError> + Send + 'static) {
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
            super::recovery::continue_sequence(path, || f(path).map_err(|e| e.user_message()))
        }
    });
}
