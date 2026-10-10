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
            verify_merge_resolutions(path, &name)?;
            run_git_seq(
                path,
                &[
                    "-c",
                    "rebase.updateRefs=false",
                    "rebase",
                    "--rebase-merges",
                    "--no-autostash",
                    "--",
                    &name,
                ],
            )
            .map_err(|e| e.user_message())
        })
    });
}

pub fn continue_rebase(state: SharedState) {
    crate::git::merge::seq_op_pub("rebase --continue", state, |path| {
        verify_split_complete(path).map_err(|e| git2::Error::from_str(&e))?;
        run_git_seq(
            path,
            &["-c", "rebase.updateRefs=false", "rebase", "--continue"],
        )?;
        cleanup_finished(path);
        Ok(())
    });
}

pub fn abort_rebase(state: SharedState) {
    crate::git::merge::seq_op_pub("rebase --abort", state, |path| {
        run_git_seq(
            path,
            &["-c", "rebase.updateRefs=false", "rebase", "--abort"],
        )?;
        cleanup_finished(path);
        Ok(())
    });
}

pub fn skip_rebase(state: SharedState) {
    crate::git::merge::seq_op_pub("rebase --skip", state, |path| {
        let repo = git2::Repository::open(path)?;
        if repo.path().join("rebase-merge/recorde-split").exists() {
            return Err(git2::Error::from_str(
                "Finish and continue the split, or abort the rebase.",
            )
            .into());
        }
        run_git_seq(path, &["-c", "rebase.updateRefs=false", "rebase", "--skip"])?;
        cleanup_finished(path);
        Ok(())
    });
}

pub fn is_edit_pause(repo: &git2::Repository) -> bool {
    let dir = repo.path().join("rebase-merge");
    dir.join("amend").is_file()
        && std::fs::read_to_string(dir.join("done")).is_ok_and(|done| {
            done.lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .is_some_and(|line| line.starts_with("edit "))
        })
}

pub fn split_paused_commit(state: SharedState) {
    let Some(path) = state.lock().unwrap().repo_path.clone() else {
        return;
    };
    crate::git::mutation::spawn_operation(path, state, |path| {
        super::recovery::continue_sequence(path, || prepare_split(path))
    });
}

fn prepare_split(path: &Path) -> Result<(), String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let dir = repo.path().join("rebase-merge");
    if !is_edit_pause(&repo) || dir.join("recorde-split").exists() {
        return Err("Split is available only at an unchanged edit stop.".into());
    }
    let head_oid = repo
        .head()
        .map_err(|e| e.to_string())?
        .target()
        .ok_or("HEAD has no commit.")?;
    let head = super::replacement::commit(&repo, head_oid).map_err(|e| e.to_string())?;
    let expected = std::fs::read_to_string(dir.join("amend")).map_err(|e| e.to_string())?;
    if expected.trim() != head_oid.to_string() || head.parent_count() != 1 {
        return Err(
            "The edited commit changed or is a merge. Reopen the edit step before splitting."
                .into(),
        );
    }
    ensure_clean_split(path)?;
    let base = head.parent_id(0).map_err(|e| e.to_string())?.to_string();
    // Persist before resetting so a cancelled reset still leaves explicit guidance.
    std::fs::write(dir.join("recorde-split"), &base).map_err(|e| e.to_string())?;
    run_git_seq(path, &["reset", "--mixed", &base]).map_err(|e| e.user_message())
}

fn ensure_clean_split(path: &Path) -> Result<(), String> {
    let status = super::process::git(
        path,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        &std::sync::atomic::AtomicBool::new(false),
    )
    .map_err(|e| e.user_message())?;
    if !status.stdout.is_empty() {
        return Err("Stage selected files or lines and commit each part. Commit or preserve all remaining changes before continuing.".into());
    }
    Ok(())
}

fn verify_split_complete(path: &Path) -> Result<(), String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let marker = repo.path().join("rebase-merge/recorde-split");
    if !marker.exists() {
        return Ok(());
    }
    let base = std::fs::read_to_string(&marker).map_err(|e| e.to_string())?;
    let base = git2::Oid::from_str(base.trim()).map_err(|e| e.to_string())?;
    let head = repo
        .head()
        .and_then(|h| h.peel_to_commit())
        .map_err(|e| e.to_string())?
        .id();
    if head == base
        || !repo
            .graph_descendant_of(head, base)
            .map_err(|e| e.to_string())?
    {
        return Err("Create at least one replacement commit before continuing the split.".into());
    }
    ensure_clean_split(path)?;
    std::fs::remove_file(marker).map_err(|e| e.to_string())?;
    Ok(())
}

// ── Interactive rebase ────────────────────────────────────────────────────────

/// Build the list of commits that would be rebased when right-clicking `upstream_oid`.
/// Returns entries ordered oldest → newest (the natural todo order).
pub fn load_rebase_entries(repo_path: &Path, upstream_oid: &str) -> Option<Vec<RebaseEntry>> {
    let repo = git2::Repository::open(repo_path).ok()?;
    let upstream = git2::Oid::from_str(upstream_oid).ok()?;

    let output = super::process::git(
        repo_path,
        &[
            "rev-list",
            "--topo-order",
            "--reverse",
            "HEAD",
            &format!("^{upstream}"),
        ],
        &std::sync::atomic::AtomicBool::new(false),
    )
    .ok()?;
    let entries: Vec<RebaseEntry> = std::str::from_utf8(&output.stdout)
        .ok()?
        .lines()
        .map(|id| {
            let oid = git2::Oid::from_str(id).ok()?;
            let c = super::replacement::commit(&repo, oid).ok()?;
            Some(RebaseEntry {
                oid: oid.to_string(),
                message: c.message().unwrap_or("").to_string(),
                summary: c.summary().ok().flatten().unwrap_or("").to_string(),
                action: RebaseAction::Pick,
            })
        })
        .collect::<Option<Vec<_>>>()?;

    if entries.is_empty() {
        None
    } else {
        Some(entries)
    }
}

/// Branch-rebase callers share the same protection against losing manual merge
/// resolutions. Run before launching native Git with --rebase-merges.
pub fn verify_merge_resolutions(path: &Path, upstream: &str) -> Result<(), String> {
    let output = super::process::git(
        path,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{upstream}^{{commit}}"),
        ],
        &std::sync::atomic::AtomicBool::new(false),
    )
    .map_err(|e| e.user_message())?;
    let upstream = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let Some(entries) = load_rebase_entries(path, &upstream) else {
        let count = super::process::git(
            path,
            &["rev-list", "--count", "HEAD", &format!("^{upstream}")],
            &std::sync::atomic::AtomicBool::new(false),
        )
        .map_err(|e| e.user_message())?;
        return if String::from_utf8_lossy(&count.stdout).trim() == "0" {
            Ok(())
        } else {
            Err("Unable to verify the replay range.".into())
        };
    };
    verify_merge_entries(path, &repo, &entries)
}

fn verify_merge_entries(
    path: &Path,
    repo: &git2::Repository,
    entries: &[RebaseEntry],
) -> Result<(), String> {
    for entry in entries {
        let commit = super::replacement::commit(
            repo,
            git2::Oid::from_str(&entry.oid).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if commit.parent_count() > 2 {
            return Err("This range contains an octopus merge whose manual resolutions Git cannot verify. Exclude it from the replay range to preserve its recorded tree.".into());
        }
        if commit.parent_count() == 2 {
            let diff = super::process::git(
                path,
                &[
                    "show",
                    "--remerge-diff",
                    "--format=",
                    "--no-ext-diff",
                    "--no-textconv",
                    &entry.oid,
                    "--",
                ],
                &std::sync::atomic::AtomicBool::new(false),
            )
            .map_err(|e| e.user_message())?;
            if !diff.stdout.is_empty() {
                return Err(format!("Merge {} contains manual resolution changes. Recreating it could lose those changes; replay a range that excludes this merge or retain and reapply its resolution explicitly.", &entry.oid[..7]));
            }
        }
    }
    Ok(())
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
    format!("'{}'", value.replace('\'', "'\\''"))
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
    let expected_set: std::collections::HashSet<_> = expected.iter().map(|e| &e.oid).collect();
    let actual: std::collections::HashSet<_> = entries.iter().map(|e| &e.oid).collect();
    if expected_set != actual {
        return Err("History changed; reopen the rebase preview.".into());
    }
    let preserve_merges = entries.iter().any(|entry| {
        super::replacement::commit(&repo, git2::Oid::from_str(&entry.oid).unwrap())
            .is_ok_and(|c| c.parent_count() > 1)
    });
    verify_merge_entries(repo_path, &repo, entries)?;
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
            RebaseAction::Edit => "edit",
            RebaseAction::Squash | RebaseAction::Fixup => "fixup",
            RebaseAction::Drop => "drop",
        };
        todo.push_str(&format!("{verb} {}\n", entry.oid));
        if matches!(entry.action, RebaseAction::Reword | RebaseAction::Squash) {
            let file = tmp.path().join(format!("message-{index}"));
            std::fs::write(&file, &entry.message).map_err(|e| e.to_string())?;
            let command = format!(
                "exec git -c core.editor=true commit --amend --file {}",
                quote_shell(&git_path(&file.to_string_lossy()))
            );
            edits.push((entry.oid.clone(), command.clone()));
            todo.push_str(&format!("{command}\n"));
        }
    }
    let todo_file = tmp.path().join("todo");
    std::fs::write(&todo_file, &todo).map_err(|e| e.to_string())?;
    let mut command = super::process::command(repo_path);
    command.args(["-c", "rebase.updateRefs=false"]);
    if preserve_merges {
        let editor = tmp.path().join("editor.sh");
        std::fs::write(&editor, topology_editor(entries, &expected, &edits))
            .map_err(|e| e.to_string())?;
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
                "--reapply-cherry-picks",
                "--empty=keep",
                "--reschedule-failed-exec",
                "--no-update-refs",
                "--no-autosquash",
                &upstream.to_string(),
            ])
            .env(
                "GIT_SEQUENCE_EDITOR",
                format!("sh {}", quote_shell(&git_path(&editor.to_string_lossy()))),
            );
    } else {
        command
            .args([
                "rebase",
                "-i",
                "--no-autostash",
                "--reapply-cherry-picks",
                "--empty=keep",
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

// Git owns the topology program. Only consecutive pick commands can be moved
// or combined; labels/resets/merges are hard boundaries. Validate the complete
// transformation before replacing Git's todo so rejection leaves HEAD untouched.
fn topology_editor(
    entries: &[RebaseEntry],
    expected: &[RebaseEntry],
    edits: &[(String, String)],
) -> String {
    fn awk(value: &str) -> String {
        format!(
            "\"{}\"",
            value
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
        )
    }
    let mut program = String::from("BEGIN {\n");
    for (rank, entry) in entries.iter().enumerate() {
        let verb = match entry.action {
            RebaseAction::Pick | RebaseAction::Reword => "pick",
            RebaseAction::Edit => "edit",
            RebaseAction::Squash | RebaseAction::Fixup => "fixup",
            RebaseAction::Drop => "drop",
        };
        program.push_str(&format!(
            "rank[{}]={}; verb[{}]={};\n",
            awk(&entry.oid),
            rank + 1,
            awk(&entry.oid),
            awk(verb)
        ));
    }
    for (rank, entry) in expected.iter().enumerate() {
        program.push_str(&format!("original[{}]={};\n", awk(&entry.oid), rank + 1));
    }
    for (oid, command) in edits {
        program.push_str(&format!("edit[{}]={};\n", awk(oid), awk(command)));
    }
    program.push_str(r#"}
function fail(message) { print message > "/dev/stderr"; failed=1 }
{
  text[NR]=$0
  if ($1 == "pick") {
    if (!($2 in rank)) fail("History changed; reopen the rebase preview.")
    seen[$2]=1; group[$2]=block; picks[block,++size[block]]=$2; oid[NR]=$2
  } else {
    block++
    if ($1 == "merge") {
      if (!($3 in rank)) fail("History changed; reopen the rebase preview.")
      seen[$3]=1; group[$3]=block; oid[NR]=$3
      if (verb[$3] != "pick") fail("Merge commits retain their topology: use Pick or Reword for a merge itself.")
    }
    block++
  }
}
END {
  for (a in rank) if (!(a in seen)) fail("Git omitted a planned commit; reopen the rebase preview.")
  for (a in rank) for (b in rank)
    if (original[a] < original[b] && rank[a] > rank[b] && group[a] != group[b])
      fail("Reorder within one branch segment. Moving across a branch point or merge would change topology.")
  for (g in size) {
    for (i=1;i<=size[g];i++) for (j=i+1;j<=size[g];j++)
      if (rank[picks[g,i]] > rank[picks[g,j]]) { t=picks[g,i]; picks[g,i]=picks[g,j]; picks[g,j]=t }
    retained=0
    for (i=1;i<=size[g];i++) {
      a=picks[g,i]
      if (verb[a] == "fixup" && !retained) fail("Squash or fixup needs an earlier retained commit in the same branch segment.")
      if (verb[a] != "drop") retained=1
    }
    if (!retained) fail("Retain a commit in each branch segment so dropping its tip cannot collapse merge topology.")
  }
  if (failed) exit 1
  for (line=1;line<=NR;line++) {
    a=oid[line]
    if (a != "" && text[line] ~ /^pick /) {
      g=group[a]
      if (emitted[g]++) continue
      for (i=1;i<=size[g];i++) {
        a=picks[g,i]; print verb[a] " " a
        if (a in edit) print edit[a]
      }
    } else {
      print text[line]
      if (a != "" && a in edit) print edit[a]
    }
  }
}
"#);
    format!("#!/bin/sh\nset -eu\nresult=\"$1.recorde\"\ntrap 'rm -f -- \"$result\"' EXIT\nawk {} \"$1\" > \"$result\"\nmv -- \"$result\" \"$1\"\n", quote_shell(&program))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::TestRepo;

    #[test]
    fn edit_split_survives_reload_and_refuses_uncommitted_parts() {
        let repo = TestRepo::new();
        let base = repo.commit("base", b"base\n");
        repo.write("one", b"one\n");
        repo.write("two", b"two\n");
        repo.git_ok(&["add", "one", "two"]);
        repo.git_ok(&[
            "commit",
            "--author",
            "Split Author <split@example.com>",
            "-m",
            "combined",
        ]);
        let original = repo.head();
        let mut entries = load_rebase_entries(repo.path(), &base.to_string()).unwrap();
        entries[0].action = RebaseAction::Edit;
        super::super::recovery::protect_sequence(repo.path(), "Split", || {
            execute(repo.path(), &base.to_string(), &entries)
        })
        .unwrap();
        assert!(is_edit_pause(&repo.open()));
        super::super::recovery::continue_sequence(repo.path(), || prepare_split(repo.path()))
            .unwrap();
        assert_eq!(repo.head(), base);
        assert_eq!(std::fs::read(repo.path().join("two")).unwrap(), b"two\n");
        assert!(prepare_split(repo.path()).is_err());
        assert!(verify_split_complete(repo.path()).is_err());
        repo.git_ok(&["add", "one"]);
        super::super::recovery::continue_sequence(repo.path(), || {
            super::super::staging::git_commit(repo.path(), "first part", false, false)
        })
        .unwrap();
        assert!(verify_split_complete(repo.path()).is_err());
        repo.git_ok(&["add", "two"]);
        super::super::recovery::continue_sequence(repo.path(), || {
            super::super::staging::git_commit(repo.path(), "second part", false, false)
        })
        .unwrap();
        super::super::recovery::continue_sequence(repo.path(), || {
            verify_split_complete(repo.path())?;
            run_git_seq(repo.path(), &["rebase", "--continue"]).map_err(|e| e.user_message())
        })
        .unwrap();
        let opened = repo.open();
        let head = opened.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(
            head.tree_id(),
            opened.find_commit(original).unwrap().tree_id()
        );
        assert_eq!(head.parent(0).unwrap().parent_id(0).unwrap(), base);
        assert_eq!(head.author().email().unwrap(), "split@example.com");
        assert_eq!(
            head.parent(0).unwrap().author().email().unwrap(),
            "split@example.com"
        );
        let checkpoint = super::super::recovery::list(repo.path()).remove(0);
        super::super::recovery::restore(repo.path(), &checkpoint).unwrap();
        assert_eq!(repo.head(), original);
    }

    #[test]
    fn merge_range_reorders_squashes_and_drops_within_branch_segments() {
        let repo = TestRepo::new();
        let base = repo.commit("base", b"base\n");
        repo.git_ok(&["checkout", "-b", "side"]);
        let one = repo.commit("one", b"one\n");
        let two = repo.commit("two", b"two\n");
        let dropped = repo.commit("dropped", b"drop\n");
        let side = repo.head();
        repo.git_ok(&["checkout", "main"]);
        let main = repo.commit("main", b"main\n");
        repo.git_ok(&["merge", "--no-ff", "side", "-m", "merge side"]);
        let mut entries = load_rebase_entries(repo.path(), &base.to_string()).unwrap();
        let first = entries
            .iter()
            .position(|e| e.oid == one.to_string())
            .unwrap();
        let second = entries
            .iter()
            .position(|e| e.oid == two.to_string())
            .unwrap();
        entries.swap(first, second);
        entries[second].action = RebaseAction::Squash;
        entries[second].message = "combined side".into();
        entries
            .iter_mut()
            .find(|e| e.oid == dropped.to_string())
            .unwrap()
            .action = RebaseAction::Drop;
        execute(repo.path(), &base.to_string(), &entries).unwrap();
        let opened = repo.open();
        let head = opened.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.parent_count(), 2);
        assert_eq!(head.parent_id(0).unwrap(), main);
        let rewritten_side = head.parent(1).unwrap();
        assert_eq!(rewritten_side.message().unwrap(), "combined side\n");
        assert_eq!(rewritten_side.parent_id(0).unwrap(), base);
        assert!(head.tree().unwrap().get_path(Path::new("dropped")).is_err());
        assert_eq!(
            opened.find_reference("refs/heads/side").unwrap().target(),
            Some(side)
        );
        assert_eq!(repo.index_bytes("one"), b"one\n");
        assert_eq!(repo.index_bytes("two"), b"two\n");
    }

    #[test]
    fn manual_merge_resolution_is_never_silently_discarded() {
        let repo = TestRepo::new();
        let base = repo.commit("base", b"base\n");
        repo.git_ok(&["checkout", "-b", "side"]);
        repo.commit("side", b"side\n");
        repo.git_ok(&["checkout", "main"]);
        repo.commit("main", b"main\n");
        repo.git_ok(&["merge", "--no-ff", "--no-commit", "side"]);
        repo.write("resolution", b"manual merge change\n");
        repo.git_ok(&["add", "resolution"]);
        repo.git_ok(&["commit", "-m", "manual merge"]);
        let before = repo.head();
        let entries = load_rebase_entries(repo.path(), &base.to_string()).unwrap();
        assert!(execute(repo.path(), &base.to_string(), &entries)
            .unwrap_err()
            .contains("manual resolution"));
        assert_eq!(repo.head(), before);
        assert_eq!(repo.open().state(), git2::RepositoryState::Clean);
        assert_eq!(repo.index_bytes("resolution"), b"manual merge change\n");
    }

    #[test]
    fn moving_across_merge_branch_points_is_rejected_without_mutation() {
        let repo = TestRepo::new();
        let base = repo.commit("base", b"base\n");
        repo.git_ok(&["checkout", "-b", "side"]);
        let side = repo.commit("side", b"side\n");
        repo.git_ok(&["checkout", "main"]);
        let main = repo.commit("main", b"main\n");
        repo.git_ok(&["merge", "--no-ff", "side", "-m", "merge"]);
        let before = repo.head();
        let mut entries = load_rebase_entries(repo.path(), &base.to_string()).unwrap();
        let a = entries
            .iter()
            .position(|e| e.oid == side.to_string())
            .unwrap();
        let b = entries
            .iter()
            .position(|e| e.oid == main.to_string())
            .unwrap();
        entries.swap(a, b);
        assert!(execute(repo.path(), &base.to_string(), &entries).is_err());
        assert_eq!(repo.head(), before);
        assert_eq!(repo.open().state(), git2::RepositoryState::Clean);
        assert_eq!(repo.index_bytes("main"), b"main\n");
        assert_eq!(std::fs::read(repo.path().join("side")).unwrap(), b"side\n");
    }

    #[test]
    fn split_refuses_dirty_reset_and_cancelled_continue_then_aborts_natively() {
        let repo = TestRepo::new();
        let base = repo.commit("base", b"base\n");
        let original = repo.commit("part", b"committed\n");
        let mut entries = load_rebase_entries(repo.path(), &base.to_string()).unwrap();
        entries[0].action = RebaseAction::Edit;
        execute(repo.path(), &base.to_string(), &entries).unwrap();
        repo.write("part", b"later work\n");
        assert!(prepare_split(repo.path()).is_err());
        assert_eq!(repo.head(), original);
        assert_eq!(
            std::fs::read(repo.path().join("part")).unwrap(),
            b"later work\n"
        );
        repo.write("part", b"committed\n");
        prepare_split(repo.path()).unwrap();
        repo.git_ok(&["add", "part"]);
        super::super::staging::git_commit(repo.path(), "split part", false, false).unwrap();
        let split_head = repo.head();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        assert!(
            super::super::process::with_cancellation(cancel, || verify_split_complete(repo.path()))
                .is_err()
        );
        assert_eq!(repo.head(), split_head);
        assert!(repo
            .open()
            .path()
            .join("rebase-merge/recorde-split")
            .is_file());
        run_git_seq(repo.path(), &["rebase", "--abort"]).unwrap();
        cleanup_finished(repo.path());
        assert_eq!(repo.head(), original);
        assert_eq!(repo.index_bytes("part"), b"committed\n");
        assert_eq!(repo.open().state(), git2::RepositoryState::Clean);
    }

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
