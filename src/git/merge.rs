use std::path::Path;

use crate::git::error::GitError;
use crate::state::SharedState;

// ── Shared shell helper ───────────────────────────────────────────────────────

/// Run `git -C <repo_path> <args>` and return stderr as a GitError on failure.
pub(crate) fn run_git_seq(repo_path: &Path, args: &[&str]) -> Result<(), GitError> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(args)
        .env("GIT_EDITOR", "true")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| GitError::Command {
            cmd: format!("git {}", args.join(" ")),
            stderr: e.to_string(),
        })?;

    if out.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(GitError::Command {
            cmd: format!("git {}", args.join(" ")),
            stderr,
        })
    }
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

    std::thread::spawn(move || {
        let result = run_git_seq(&path, &["merge", "--no-edit", &name]);
        {
            let mut s = state.lock().unwrap();
            s.remote_op = None;
            if let Err(ref e) = result {
                s.console_log.push(format!("error: {}", e.user_message()));
                // Don't surface as UI error — capabilities reload will show conflict banner
            }
        }
        crate::git::branches::reload_repo(state);
    });
}

pub fn continue_merge(state: SharedState) {
    seq_op("merge --continue", state, |path| {
        run_git_seq(path, &["merge", "--continue", "--no-edit"])
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
    std::thread::spawn(move || {
        let result = f(&path);
        {
            let mut s = state.lock().unwrap();
            s.remote_op = None;
            if let Err(ref e) = result {
                let msg = e.user_message();
                s.console_log.push(format!("error: {}", msg));
                s.error = Some(msg);
            }
        }
        crate::git::branches::reload_repo(state);
    });
}
