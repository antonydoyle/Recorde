use std::path::Path;

use crate::state::{RebaseAction, RebaseEntry, SharedState};

/// Build the list of commits that would be rebased when right-clicking `upstream_oid`.
/// Returns entries ordered oldest → newest (the natural todo order).
pub fn load_rebase_entries(repo_path: &Path, upstream_oid: &str) -> Option<Vec<RebaseEntry>> {
    let repo = git2::Repository::open(repo_path).ok()?;
    let upstream = git2::Oid::from_str(upstream_oid).ok()?;

    let mut walk = repo.revwalk().ok()?;
    walk.push_head().ok()?;
    walk.hide(upstream).ok()?;
    walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME).ok()?;

    let mut entries: Vec<RebaseEntry> = walk
        .filter_map(|r| r.ok())
        .filter_map(|oid| repo.find_commit(oid).ok())
        .map(|c| RebaseEntry {
            oid: c.id().to_string(),
            summary: c.summary().unwrap_or("").to_string(),
            action: RebaseAction::Pick,
        })
        .collect();

    entries.reverse(); // oldest first, matches git-rebase-todo order
    if entries.is_empty() { None } else { Some(entries) }
}

/// Execute the interactive rebase in a background thread.
pub fn start_rebase(upstream_oid: &str, entries: Vec<RebaseEntry>, state: SharedState) {
    let upstream = upstream_oid.to_string();
    let repo_path = state.lock().unwrap().repo_path.clone();
    let Some(path) = repo_path else { return };

    std::thread::spawn(move || {
        match execute(&path, &upstream, &entries) {
            Ok(()) => crate::git::branches::reload_repo(state),
            Err(e) => state.lock().unwrap().error = Some(e),
        }
    });
}

fn execute(repo_path: &Path, upstream_oid: &str, entries: &[RebaseEntry]) -> Result<(), String> {
    // Build the todo content
    let mut todo = String::new();
    for entry in entries {
        let verb = match entry.action {
            RebaseAction::Pick   => "pick",
            RebaseAction::Squash => "squash",
            RebaseAction::Fixup  => "fixup",
            RebaseAction::Drop   => "drop",
        };
        // git rebase-todo uses the short SHA
        let short = &entry.oid[..7.min(entry.oid.len())];
        todo.push_str(&format!("{} {} {}\n", verb, short, entry.summary));
    }

    let tmp = std::env::temp_dir();
    let todo_file   = tmp.join("recorde_rebase_todo");
    let editor_path = tmp.join("recorde_seq_editor.sh");

    std::fs::write(&todo_file, &todo).map_err(|e| e.to_string())?;

    // Tiny shell script: copy our pre-built todo over whatever git prepared
    let script = format!("#!/bin/sh\ncp '{}' \"$1\"\n", todo_file.display());
    std::fs::write(&editor_path, script).map_err(|e| e.to_string())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&editor_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }

    let out = std::process::Command::new("git")
        .arg("-C").arg(repo_path)
        .args(["rebase", "-i", upstream_oid])
        .env("GIT_SEQUENCE_EDITOR", &editor_path)
        .env("GIT_EDITOR", "true")   // suppress editor pop-ups
        .env("GIT_TERMINAL_PROMPT", "0") // no interactive prompts
        .output()
        .map_err(|e| format!("Could not run git: {}", e))?;

    // Clean up temp files regardless of outcome
    let _ = std::fs::remove_file(&todo_file);
    let _ = std::fs::remove_file(&editor_path);

    if out.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let msg = stderr.trim();
        // Provide a helpful hint for the conflict case
        if msg.contains("conflict") || msg.contains("CONFLICT") {
            Err(format!(
                "Rebase paused on conflict — resolve in a terminal then run \
                 `git rebase --continue` or `git rebase --abort`.\n\n{}",
                msg
            ))
        } else {
            Err(format!("git rebase failed:\n{}", msg))
        }
    }
}
