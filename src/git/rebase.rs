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

    crate::git::mutation::spawn_operation(path, state, move |path| {
        super::recovery::protect_sequence(path, "Rebase", || {
            run_git_seq(path, &["rebase", "--no-autostash", "--", &name])
                .map_err(|e| e.user_message())
        })
    });
}

pub fn continue_rebase(state: SharedState) {
    crate::git::merge::seq_op_pub("rebase --continue", state, |path| {
        run_git_seq(path, &["rebase", "--continue"])?;
        cleanup_finished(path);
        Ok(())
    });
}

pub fn abort_rebase(state: SharedState) {
    crate::git::merge::seq_op_pub("rebase --abort", state, |path| {
        run_git_seq(path, &["rebase", "--abort"])?;
        cleanup_finished(path);
        Ok(())
    });
}

pub fn skip_rebase(state: SharedState) {
    crate::git::merge::seq_op_pub("rebase --skip", state, |path| {
        run_git_seq(path, &["rebase", "--skip"])?;
        cleanup_finished(path);
        Ok(())
    });
}

// ── Interactive rebase ────────────────────────────────────────────────────────

/// Build the list of commits that would be rebased when right-clicking `upstream_oid`.
/// Returns entries ordered oldest → newest (the natural todo order).
pub fn load_rebase_entries(repo_path: &Path, upstream_oid: &str) -> Option<Vec<RebaseEntry>> {
    let repo = git2::Repository::open(repo_path).ok()?;
    let upstream = git2::Oid::from_str(upstream_oid).ok()?;

    let mut walk = repo.revwalk().ok()?;
    walk.push_head().ok()?;
    walk.hide(upstream).ok()?;
    walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)
        .ok()?;

    let mut entries: Vec<RebaseEntry> = walk
        .filter_map(|r| r.ok())
        .filter_map(|oid| repo.find_commit(oid).ok())
        .map(|c| RebaseEntry {
            oid: c.id().to_string(),
            message: c.message().unwrap_or("").to_string(),
            summary: c.summary().ok().flatten().unwrap_or("").to_string(),
            action: RebaseAction::Pick,
        })
        .collect();

    entries.reverse(); // oldest first, matches git-rebase-todo order
    if entries.is_empty() {
        None
    } else {
        Some(entries)
    }
}

/// Execute the interactive rebase in a background thread.
pub fn start_rebase(upstream_oid: &str, entries: Vec<RebaseEntry>, state: SharedState) {
    let upstream = upstream_oid.to_string();
    let repo_path = state.lock().unwrap().repo_path.clone();
    let Some(path) = repo_path else { return };

    crate::git::mutation::spawn_operation(path, state, move |path| {
        super::recovery::protect_sequence(path, "Interactive rebase", || {
            execute(path, &upstream, &entries)
        })
    });
}

pub fn validate_entries(entries: &[RebaseEntry]) -> Result<(), String> {
    let first = entries
        .iter()
        .find(|e| e.action != RebaseAction::Drop)
        .ok_or("Retain at least one commit.")?;
    if matches!(first.action, RebaseAction::Squash | RebaseAction::Fixup) {
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
    format!("'{}'", git_path(value).replace('\'', "'\\''"))
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
    let expected_order: Vec<_> = expected.iter().map(|e| e.oid.clone()).collect();
    let expected: std::collections::HashSet<_> = expected.iter().map(|e| &e.oid).collect();
    let actual: std::collections::HashSet<_> = entries.iter().map(|e| &e.oid).collect();
    if expected != actual {
        return Err("History changed; reopen the rebase preview.".into());
    }
    let preserve_merges = entries.iter().any(|entry| {
        repo.find_commit(git2::Oid::from_str(&entry.oid).unwrap())
            .is_ok_and(|c| c.parent_count() > 1)
    });
    if preserve_merges
        && (entries.iter().map(|e| e.oid.clone()).collect::<Vec<_>>() != expected_order
            || entries
                .iter()
                .any(|e| !matches!(e.action, RebaseAction::Pick | RebaseAction::Reword)))
    {
        return Err("Merge-preserving replay supports Pick and Reword in the original order. Choose a linear range to reorder, squash or drop commits.".into());
    }
    let directory = repo.path().join("recorde/rebases");
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let tmp = tempfile::Builder::new()
        .prefix("replay-")
        .tempdir_in(directory)
        .map_err(|e| e.to_string())?;
    std::fs::write(tmp.path().join("recorde-owned"), b"1").map_err(|e| e.to_string())?;
    let mut edits = Vec::new();
    let mut todo = String::new();
    for (index, entry) in entries.iter().enumerate() {
        let verb = match entry.action {
            RebaseAction::Pick | RebaseAction::Reword => "pick",
            RebaseAction::Squash | RebaseAction::Fixup => "fixup",
            RebaseAction::Drop => "drop",
        };
        todo.push_str(&format!("{verb} {}\n", entry.oid));
        if matches!(entry.action, RebaseAction::Reword | RebaseAction::Squash) {
            let file = tmp.path().join(format!("message-{index}"));
            std::fs::write(&file, &entry.message).map_err(|e| e.to_string())?;
            let command = format!(
                "exec git -c core.editor=true commit --amend --file {}",
                quote_shell(&file.to_string_lossy())
            );
            edits.push((entry.oid.clone(), command.clone()));
            todo.push_str(&format!("{command}\n"));
        }
    }
    let todo_file = tmp.path().join("todo");
    std::fs::write(&todo_file, &todo).map_err(|e| e.to_string())?;
    let mut command = super::process::command(repo_path);
    if preserve_merges {
        let editor = tmp.path().join("editor.sh");
        std::fs::write(&editor, topology_editor(&edits)).map_err(|e| e.to_string())?;
        command
            .args([
                "-c",
                "core.abbrev=40",
                "-c",
                "rebase.abbreviateCommands=false",
                "rebase",
                "-i",
                "--rebase-merges",
                "--no-autostash",
                "--reschedule-failed-exec",
                "--no-update-refs",
                "--no-autosquash",
                &upstream.to_string(),
            ])
            .env(
                "GIT_SEQUENCE_EDITOR",
                format!("sh {}", quote_shell(&editor.to_string_lossy())),
            );
    } else {
        command
            .args([
                "rebase",
                "-i",
                "--no-autostash",
                "--reschedule-failed-exec",
                "--no-update-refs",
                "--no-autosquash",
                &upstream.to_string(),
            ])
            .env("GIT_SEQUENCE_EDITOR", "cp -- \"$RECORDE_REBASE_TODO\"")
            .env(
                "RECORDE_REBASE_TODO",
                git_path(&todo_file.to_string_lossy()),
            );
    }
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
                        || name.strip_prefix("message-").is_some_and(|s| {
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

// Modify only message steps in Git's generated topology todo; preserve labels,
// resets, merge commands and their order byte-for-byte. Message text stays in files.
fn topology_editor(edits: &[(String, String)]) -> String {
    let mut script = String::from(
        r#"#!/bin/sh
set -eu
result="$1.recorde"
trap 'rm -f -- "$result"' EXIT
while IFS= read -r line || [ -n "$line" ]; do
  printf '%s\n' "$line"
  case "$line" in
"#,
    );
    for (oid, command) in edits {
        script.push_str(&format!(
            "    {}*|{}*|{}*) printf '%s\\n' {} ;;\n",
            quote_shell(&format!("pick {oid} ")),
            quote_shell(&format!("merge -C {oid} ")),
            quote_shell(&format!("merge -c {oid} ")),
            quote_shell(command)
        ));
    }
    script.push_str(
        r#"  esac
done < "$1" > "$result"
mv -- "$result" "$1"
"#,
    );
    script
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::TestRepo;

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
