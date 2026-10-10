use std::path::Path;

use crate::git::{merge::run_git_seq, patch};
use crate::state::SharedState;

// Sequencer and conflict indexes cannot be represented by a single tree.
fn protect_index<T>(
    path: &Path,
    label: &str,
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    if repo.state() == git2::RepositoryState::Clean
        && !repo.index().map_err(|e| e.to_string())?.has_conflicts()
    {
        super::recovery::protect(path, label, work)
    } else {
        work()
    }
}

fn reload(state: SharedState) {
    crate::git::branches::reload_repo(state);
}

pub fn ignore_file(file: &str, state: SharedState) {
    let Some(path) = state.lock().unwrap().repo_path.clone() else {
        return;
    };
    let file = file.to_owned();
    crate::git::mutation::spawn_operation(path, state, move |path| add_ignore_rule(path, &file));
}

fn add_ignore_rule(path: &Path, file: &str) -> Result<(), String> {
    super::safe_path::relative(file)?;
    if file.contains(['\r', '\n']) {
        return Err("Git ignore rules cannot represent a filename containing a newline.".into());
    }
    let target = super::safe_path::ScopedFile::new(path, ".gitignore")?;
    let original = target.read(1_048_576)?;
    let mut content = original.clone().unwrap_or_default();
    if !content.is_empty() && !content.ends_with(b"\n") {
        content.push(b'\n');
    }
    content.push(b'/');
    for character in file.chars() {
        if matches!(character, '\\' | '*' | '?' | '[' | ']' | ' ' | '#' | '!') {
            content.push(b'\\');
        }
        let mut buffer = [0; 4];
        content.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
    }
    content.push(b'\n');
    target.replace(original.as_deref(), &content)?;
    Ok(())
}

// ── Stage / Unstage individual files ─────────────────────────────────────────

pub fn stage_file(file_path: &str, state: SharedState) {
    let fp = file_path.to_string();
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else { return };
    crate::git::mutation::spawn_operation(rp, state, move |path| {
        protect_index(path, "Stage file", || {
            do_stage(path, &fp).map_err(|e| e.message().to_string())
        })
    });
}

pub fn unstage_file(file_path: &str, state: SharedState) {
    let fp = file_path.to_string();
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else { return };
    crate::git::mutation::spawn_operation(rp, state, move |path| {
        protect_index(path, "Unstage file", || {
            do_unstage(path, &fp).map_err(|e| e.message().to_string())
        })
    });
}

// ── Stage / Unstage all ───────────────────────────────────────────────────────

pub fn stage_all(state: SharedState) {
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else { return };
    crate::git::mutation::spawn_operation(rp, state, move |path| {
        protect_index(path, "Stage all", || {
            do_stage_all(path).map_err(|e| e.message().to_string())
        })
    });
}

pub fn unstage_all(state: SharedState) {
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else { return };
    crate::git::mutation::spawn_operation(rp, state, move |path| {
        protect_index(path, "Unstage all", || {
            do_unstage_all(path).map_err(|e| e.message().to_string())
        })
    });
}

pub fn stage_files(files: Vec<String>, reverse: bool, state: SharedState) {
    let Some(path) = state.lock().unwrap().repo_path.clone() else {
        return;
    };
    crate::git::mutation::spawn_operation(path, state, move |path| {
        protect_index(
            path,
            if reverse {
                "Unstage selected files"
            } else {
                "Stage selected files"
            },
            || do_stage_files(path, files, reverse),
        )
    });
}

fn do_stage_files(path: &Path, files: Vec<String>, reverse: bool) -> Result<(), String> {
    use std::io::Write;
    if files.is_empty() {
        return Err("Select files first.".into());
    }
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let mut paths: std::collections::BTreeSet<_> = files.into_iter().collect();
    for file in paths.clone() {
        super::safe_path::relative(&file)?;
        if reverse {
            let diff = super::operations::compute_workdir_diff(path, &file, true)
                .map_err(|e| e.to_string())?;
            if diff.is_rename {
                if let Some(old) = diff.old_path {
                    paths.insert(old);
                }
            }
        }
    }
    let mut specs = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    for file in paths {
        specs
            .write_all(file.as_bytes())
            .and_then(|_| specs.write_all(&[0]))
            .map_err(|e| e.to_string())?;
    }
    specs.flush().map_err(|e| e.to_string())?;
    let mut command = super::process::command(path);
    command.arg("--literal-pathspecs");
    if !reverse {
        command.args(["add", "--all"]);
    } else if repo.head().is_ok() {
        command.args(["reset", "HEAD"]);
    } else {
        command.args(["rm", "--cached", "-r", "-f", "--ignore-unmatch"]);
    }
    command
        .arg("--pathspec-file-nul")
        .arg("--pathspec-from-file")
        .arg(specs.path());
    let output = super::process::run(
        &mut command,
        &std::sync::atomic::AtomicBool::new(false),
        std::time::Duration::from_secs(600),
    )
    .map_err(|e| e.user_message())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(crate::auth::redact::text(&String::from_utf8_lossy(
            &output.stderr,
        )))
    }
}

// ── Commit ────────────────────────────────────────────────────────────────────

pub fn create_commit(
    message: &str,
    signoff: bool,
    state: SharedState,
) -> Option<std::sync::mpsc::Receiver<bool>> {
    submit_commit(message, false, signoff, state)
}

pub fn amend_commit(
    message: &str,
    signoff: bool,
    state: SharedState,
) -> Option<std::sync::mpsc::Receiver<bool>> {
    submit_commit(message, true, signoff, state)
}

fn submit_commit(
    message: &str,
    amend: bool,
    signoff: bool,
    state: SharedState,
) -> Option<std::sync::mpsc::Receiver<bool>> {
    let msg = message.to_string();
    let rp = state.lock().unwrap().repo_path.clone()?;
    let (tx, rx) = std::sync::mpsc::channel();
    let epoch = state.lock().unwrap().repo_epoch;
    let work_path = rp.clone();
    crate::git::mutation::spawn_job(
        rp.clone(),
        if amend { "Amend commit" } else { "Commit" },
        state,
        move || {
            let rp = work_path;
            if git2::Repository::open(&rp).is_ok_and(|r| r.state() == git2::RepositoryState::Clean)
            {
                super::recovery::protect(&rp, if amend { "Amend commit" } else { "Commit" }, || {
                    git_commit(&rp, &msg, amend, signoff)
                })
            } else {
                super::recovery::continue_sequence(&rp, || git_commit(&rp, &msg, amend, signoff))
            }
        },
        move |result, state| {
            let success = result.is_ok();
            let current = {
                let mut s = state.lock().unwrap();
                let current = s.repo_path.as_ref() == Some(&rp) && s.repo_epoch == epoch;
                match result {
                    Ok(out) => s.console_log.push(out.trim().to_string()),
                    Err(e) => {
                        s.console_log
                            .push(format!("error ({}): {}", rp.display(), e.trim()));
                        if current {
                            s.error = Some(e.trim().to_string());
                        }
                    }
                }
                current
            };
            let _ = tx.send(success);
            if current {
                reload(state);
            }
        },
    );
    Some(rx)
}

// ── Reset ─────────────────────────────────────────────────────────────────────

pub fn reset_soft(state: SharedState) {
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else { return };
    state.lock().unwrap().pending_discard = None;
    crate::git::mutation::spawn_operation(rp, state, move |path| {
        super::recovery::protect(path, "Soft reset", || {
            run_git_seq(path, &["reset", "--soft", "HEAD~1"]).map_err(|e| e.user_message())
        })
    });
}

pub fn reset_mixed(state: SharedState) {
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else { return };
    state.lock().unwrap().pending_discard = None;
    crate::git::mutation::spawn_operation(rp, state, move |path| {
        super::recovery::protect(path, "Mixed reset", || {
            run_git_seq(path, &["reset", "--mixed", "HEAD~1"]).map_err(|e| e.user_message())
        })
    });
}

pub fn reset_hard(state: SharedState) {
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else {
        state.lock().unwrap().pending_discard = None;
        return;
    };
    state.lock().unwrap().pending_discard = None;
    crate::git::mutation::spawn_operation(rp, state, move |path| {
        super::recovery::protect(path, "Hard reset", || {
            run_git_seq(path, &["reset", "--hard", "HEAD~1"]).map_err(|e| e.user_message())
        })
    });
}

/// Load the HEAD commit message into `out` (used to pre-populate the amend form).
pub fn load_head_message(state: SharedState) -> Option<String> {
    let rp = state.lock().unwrap().repo_path.clone()?;
    let repo = git2::Repository::open(&rp).ok()?;
    let msg = repo
        .head()
        .ok()?
        .peel_to_commit()
        .ok()?
        .message()
        .ok()?
        .to_string();
    Some(msg)
}

// ── Hunk / line staging ───────────────────────────────────────────────────────

pub fn stage_hunk(hunk_index: usize, state: SharedState) {
    apply_selection(hunk_index, None, false, state);
}

pub fn unstage_hunk(hunk_index: usize, state: SharedState) {
    apply_selection(hunk_index, None, true, state);
}

pub fn discard_hunk(hunk_index: usize, state: SharedState) {
    let Some((repo, diff, hunk)) = pluck_hunk(hunk_index, &state) else {
        return;
    };
    let lines = hunk
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l)| matches!(l.origin, '+' | '-'))
        .map(|(l, _)| (hunk_index, l))
        .collect();
    discard_selection(repo, diff, lines, state);
}

pub fn discard_selection(
    repo: std::path::PathBuf,
    diff: std::sync::Arc<crate::state::FileDiff>,
    lines: Vec<(usize, usize)>,
    state: SharedState,
) {
    {
        let mut s = state.lock().unwrap();
        s.pending_discard = None;
        if s.repo_path.as_ref() != Some(&repo) {
            return;
        }
    }
    crate::git::mutation::spawn_operation(repo, state, move |path| {
        ensure_no_filter(path, &diff.file_path)?;
        super::recovery::protect(path, "Discard selection", || {
            discard_captured_selection(path, &diff, &lines)
        })
    });
}

fn discard_captured_selection(
    path: &Path,
    diff: &crate::state::FileDiff,
    lines: &[(usize, usize)],
) -> Result<(), String> {
    let current = super::operations::compute_workdir_diff(path, &diff.file_path, false)
        .map_err(|e| e.to_string())?;
    if current.raw_patch != diff.raw_patch {
        return Err(
            "The file or index changed after this diff was loaded. Refresh before discarding."
                .into(),
        );
    }
    let patch = patch::multi_selection_patch(diff, lines, true)?;
    patch::discard_from_workdir(path, &patch).map_err(|e| e.user_message())
}

pub fn stage_lines(hunk_index: usize, line_indices: Vec<usize>, state: SharedState) {
    apply_selection(hunk_index, Some(line_indices), false, state);
}

pub fn unstage_lines(hunk_index: usize, line_indices: Vec<usize>, state: SharedState) {
    apply_selection(hunk_index, Some(line_indices), true, state);
}

fn apply_selection(
    hunk_index: usize,
    selected: Option<Vec<usize>>,
    reverse: bool,
    state: SharedState,
) {
    let Some((_, _, hunk)) = pluck_hunk(hunk_index, &state) else {
        return;
    };
    let selected = selected.unwrap_or_else(|| (0..hunk.lines.len()).collect());
    let selected = selected
        .into_iter()
        .filter(|&i| {
            hunk.lines
                .get(i)
                .is_some_and(|line| matches!(line.origin, '+' | '-'))
        })
        .map(|line| (hunk_index, line))
        .collect();
    apply_selected_lines(selected, reverse, state);
}

pub fn apply_selected_lines(selected: Vec<(usize, usize)>, reverse: bool, state: SharedState) {
    let (rp, diff) = {
        let s = state.lock().unwrap();
        let expected = if reverse {
            crate::state::DiffContext::StagedFile
        } else {
            crate::state::DiffContext::UnstagedFile
        };
        if s.diff_context != expected {
            return;
        }
        let (Some(rp), Some(diff)) = (s.repo_path.clone(), s.current_diff.clone()) else {
            return;
        };
        (rp, diff)
    };
    crate::git::mutation::spawn_operation(rp, state, move |path| {
        ensure_no_filter(path, &diff.file_path)?;
        protect_index(
            path,
            if reverse {
                "Unstage selection"
            } else {
                "Stage selection"
            },
            || apply_captured_selection(path, &diff, &selected, reverse),
        )
    });
}

fn apply_captured_selection(
    path: &Path,
    diff: &crate::state::FileDiff,
    selected: &[(usize, usize)],
    reverse: bool,
) -> Result<(), String> {
    let current = super::operations::compute_workdir_diff(path, &diff.file_path, reverse)
        .map_err(|e| e.to_string())?;
    if current.raw_patch != diff.raw_patch {
        return Err("The file or index changed after this diff was loaded. Refresh before applying the selection.".into());
    }
    let patch = patch::multi_selection_patch(diff, selected, reverse)?;
    (if reverse {
        patch::remove_from_index(path, &patch)
    } else {
        patch::apply_to_index(path, &patch)
    })
    .map_err(|e| e.user_message())
}

fn ensure_no_filter(path: &Path, file: &str) -> Result<(), String> {
    let output = super::process::git(
        path,
        &["check-attr", "-z", "filter", "--", file],
        &std::sync::atomic::AtomicBool::new(false),
    )
    .map_err(|e| e.user_message())?;
    let fields: Vec<_> = output.stdout.split(|byte| *byte == 0).collect();
    if fields
        .get(2)
        .is_some_and(|value| !matches!(*value, b"unspecified" | b"unset" | b""))
    {
        return Err("This file uses a Git content filter. Use a whole-file operation to preserve its filtered representation.".into());
    }
    Ok(())
}

/// Discard all unstaged changes to a file (restore from index).
pub fn discard_file(file_path: &str, state: SharedState) {
    let fp = file_path.to_string();
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else {
        state.lock().unwrap().pending_discard = None;
        return;
    };
    state.lock().unwrap().pending_discard = None;
    crate::git::mutation::spawn_operation(rp, state, move |path| {
        super::recovery::protect(path, "Discard file", || {
            do_discard_file(path, &fp).map_err(|e| e.message().to_string())
        })
    });
}

// Read (repo_path, file_path, hunk_clone) from state — returns None if anything is missing.
fn pluck_hunk(
    hunk_index: usize,
    state: &SharedState,
) -> Option<(
    std::path::PathBuf,
    std::sync::Arc<crate::state::FileDiff>,
    crate::state::DiffHunk,
)> {
    let s = state.lock().unwrap();
    let rp = s.repo_path.clone()?;
    let diff = s.current_diff.as_ref()?;
    let hunk = diff.hunks.get(hunk_index)?.clone();
    Some((rp, diff.clone(), hunk))
}

// ── Internals ─────────────────────────────────────────────────────────────────

fn do_stage(repo_path: &Path, file_path: &str) -> Result<(), git2::Error> {
    run_git_seq(repo_path, &["--literal-pathspecs", "add", "--", file_path])
        .map_err(|e| git2::Error::from_str(&e.user_message()))
}

fn do_unstage(repo_path: &Path, file_path: &str) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(repo_path)?;
    // reset_default = git reset HEAD -- <path>
    let target = repo
        .head()
        .ok()
        .and_then(|h| h.peel(git2::ObjectType::Commit).ok());
    if target.is_some() {
        let diff = super::operations::compute_workdir_diff(repo_path, file_path, true)?;
        let mut args = vec!["--literal-pathspecs", "reset", "HEAD", "--", file_path];
        if diff.is_rename {
            if let Some(old) = &diff.old_path {
                args.push(old);
            }
        }
        run_git_seq(repo_path, &args).map_err(|e| git2::Error::from_str(&e.user_message()))?;
    } else {
        // Initial commit: just remove the path from the index
        let mut index = repo.index()?;
        index.remove_path(Path::new(file_path))?;
        index.write()?;
    }
    Ok(())
}

fn do_stage_all(repo_path: &Path) -> Result<(), git2::Error> {
    run_git_seq(repo_path, &["add", "--all"]).map_err(|e| git2::Error::from_str(&e.user_message()))
}

fn do_unstage_all(repo_path: &Path) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(repo_path)?;
    let target = repo
        .head()
        .ok()
        .and_then(|h| h.peel(git2::ObjectType::Commit).ok());
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
pub(super) fn git_commit(
    repo_path: &Path,
    message: &str,
    amend: bool,
    signoff: bool,
) -> Result<String, String> {
    super::commit_settings::check_identity(repo_path, amend)?;
    let mut cmd = super::process::command(repo_path);
    cmd.arg("commit").arg("-m").arg(message);
    if amend {
        cmd.arg("--amend");
    }
    if signoff {
        cmd.arg("--signoff");
    }
    if !amend {
        let repo = git2::Repository::open(repo_path).map_err(|e| e.to_string())?;
        let rebase = repo.path().join("rebase-merge");
        if rebase.join("recorde-split").is_file() {
            let split = super::rebase::split_state(&repo)?;
            let original = super::replacement::commit(
                &repo,
                git2::Oid::from_str(&split.original).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            let author = original.author();
            let time = author.when();
            let offset = time.offset_minutes();
            cmd.env("GIT_AUTHOR_NAME", author.name().map_err(|e| e.to_string())?)
                .env(
                    "GIT_AUTHOR_EMAIL",
                    author.email().map_err(|e| e.to_string())?,
                )
                .env(
                    "GIT_AUTHOR_DATE",
                    format!(
                        "{} {}{:02}{:02}",
                        time.seconds(),
                        if offset < 0 { "-" } else { "+" },
                        offset.abs() / 60,
                        offset.abs() % 60
                    ),
                );
        }
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0");

    let out = super::process::run(
        &mut cmd,
        &std::sync::atomic::AtomicBool::new(false),
        std::time::Duration::from_secs(600),
    )
    .map_err(|e| e.user_message())?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let combined = crate::auth::redact::text(&combined);
    if out.status.success() {
        Ok(combined)
    } else {
        let signing = super::commit_settings::load(repo_path).is_ok_and(|s| s.enabled);
        if signing {
            Err(format!("{combined}\nCommit signing is enabled. Check the key and signing agent in Settings > Repository if signing failed. The draft is retained; Recorde does not retry unsigned."))
        } else {
            Err(combined)
        }
    }
}

fn do_discard_file(repo_path: &Path, file_path: &str) -> Result<(), git2::Error> {
    run_git_seq(
        repo_path,
        &[
            "--literal-pathspecs",
            "restore",
            "--worktree",
            "--",
            file_path,
        ],
    )
    .map_err(|e| git2::Error::from_str(&e.user_message()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::TestRepo;

    #[test]
    fn batch_stage_uses_literal_paths_and_unstages_unborn_and_renamed_files() {
        let repo = TestRepo::new();
        repo.write("a[1]", b"one");
        repo.write("a1", b"unselected");
        repo.write("second", b"two");
        do_stage_files(repo.path(), vec!["a[1]".into(), "second".into()], false).unwrap();
        assert_eq!(repo.index_bytes("a[1]"), b"one");
        assert_eq!(repo.index_bytes("second"), b"two");
        assert!(repo
            .open()
            .index()
            .unwrap()
            .get_path(Path::new("a1"), 0)
            .is_none());
        repo.write("second", b"unstaged edits");
        do_stage_files(repo.path(), vec!["a[1]".into(), "second".into()], true).unwrap();
        assert!(repo.open().index().unwrap().is_empty());
        assert_eq!(
            std::fs::read(repo.path().join("second")).unwrap(),
            b"unstaged edits"
        );
        repo.commit("old", b"rename\n");
        repo.git_ok(&["mv", "old", "new"]);
        do_stage_files(repo.path(), vec!["new".into()], true).unwrap();
        assert_eq!(repo.index_bytes("old"), b"rename\n");
        assert!(repo
            .open()
            .index()
            .unwrap()
            .get_path(Path::new("new"), 0)
            .is_none());
        assert_eq!(std::fs::read(repo.path().join("new")).unwrap(), b"rename\n");
    }

    #[test]
    fn ignore_rule_is_literal_and_preserves_existing_bytes() {
        let repo = TestRepo::new();
        repo.write(".gitignore", b"existing");
        add_ignore_rule(repo.path(), "literal[1]* file ").unwrap();
        assert_eq!(
            std::fs::read(repo.path().join(".gitignore")).unwrap(),
            b"existing\n/literal\\[1\\]\\*\\ file\\ \n"
        );
        assert!(repo
            .git(&["check-ignore", "--", "literal[1]* file "])
            .status
            .success());
        assert!(!repo
            .git(&["check-ignore", "--", "literal1-other file "])
            .status
            .success());
        assert!(add_ignore_rule(repo.path(), "bad\npattern").is_err());
    }

    #[test]
    #[cfg(unix)]
    fn ignore_rule_refuses_symlink_target() {
        let repo = TestRepo::new();
        let external = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(external.path(), b"untouched").unwrap();
        std::os::unix::fs::symlink(external.path(), repo.path().join(".gitignore")).unwrap();
        assert!(add_ignore_rule(repo.path(), "file").is_err());
        assert_eq!(std::fs::read(external.path()).unwrap(), b"untouched");
    }

    #[test]
    fn selected_discard_preserves_other_changes_and_refuses_stale_bytes() {
        let repo = TestRepo::new();
        repo.commit("file", b"base\nold\nend\n");
        repo.write("file", b"base\nkeep\nnew\nend\n");
        let diff =
            super::super::operations::compute_workdir_diff(repo.path(), "file", false).unwrap();
        let selected: Vec<_> = diff
            .hunks
            .iter()
            .enumerate()
            .flat_map(|(h, hunk)| {
                hunk.lines
                    .iter()
                    .enumerate()
                    .filter(|(_, line)| line.content == "old\n" || line.content == "new\n")
                    .map(move |(l, _)| (h, l))
            })
            .collect();
        super::super::recovery::protect(repo.path(), "Discard selection", || {
            discard_captured_selection(repo.path(), &diff, &selected)
        })
        .unwrap();
        assert_eq!(
            std::fs::read(repo.path().join("file")).unwrap(),
            b"base\nold\nkeep\nend\n"
        );
        assert_eq!(repo.index_bytes("file"), b"base\nold\nend\n");
        let saved = super::super::recovery::list(repo.path()).remove(0);
        super::super::recovery::restore(repo.path(), &saved).unwrap();
        assert_eq!(
            std::fs::read(repo.path().join("file")).unwrap(),
            b"base\nkeep\nnew\nend\n"
        );
        repo.write("file", b"external edit\n");
        assert!(discard_captured_selection(repo.path(), &diff, &selected).is_err());
        assert_eq!(
            std::fs::read(repo.path().join("file")).unwrap(),
            b"external edit\n"
        );
        assert_eq!(repo.index_bytes("file"), b"base\nold\nend\n");
    }

    #[test]
    fn queued_selection_rejects_stale_diff_without_modifying_index() {
        let repo = TestRepo::new();
        repo.commit("file.txt", b"base\n");
        repo.write("file.txt", b"base\nselected\n");
        let diff =
            super::super::operations::compute_workdir_diff(repo.path(), "file.txt", false).unwrap();
        let line = diff.hunks[0]
            .lines
            .iter()
            .position(|line| line.origin == '+')
            .unwrap();
        repo.write("file.txt", b"base\nselected\nexternal change\n");
        assert!(apply_captured_selection(repo.path(), &diff, &[(0, line)], false).is_err());
        assert_eq!(repo.index_bytes("file.txt"), b"base\n");
    }

    #[test]
    fn unstaging_rename_restores_both_index_paths_without_touching_worktree() {
        let repo = TestRepo::new();
        repo.commit("old.txt", b"content\n");
        repo.git_ok(&["mv", "old.txt", "new.txt"]);
        do_unstage(repo.path(), "new.txt").unwrap();
        assert_eq!(repo.index_bytes("old.txt"), b"content\n");
        assert!(repo
            .open()
            .index()
            .unwrap()
            .get_path(Path::new("new.txt"), 0)
            .is_none());
        assert_eq!(
            std::fs::read(repo.path().join("new.txt")).unwrap(),
            b"content\n"
        );
        assert!(!repo.path().join("old.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn cancelling_a_running_commit_hook_preserves_head_and_staged_content() {
        use std::os::unix::fs::PermissionsExt;
        let repo = TestRepo::new();
        let head = repo.commit("file.txt", b"original\n");
        repo.write("file.txt", b"staged\n");
        repo.git_ok(&["add", "--", "file.txt"]);
        let hook = repo.path().join("empty-hooks/pre-commit");
        std::fs::write(
            &hook,
            "#!/bin/sh\nprintf ready > .git/hook-started\nwhile :; do sleep 1; done\n",
        )
        .unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        let state = crate::state::new_shared_state();
        state.lock().unwrap().repo_path = Some(repo.path().into());
        let completion = create_commit("retained message", false, state.clone()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !repo.path().join(".git/hook-started").exists()
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(repo.path().join(".git/hook-started").exists());
        state.lock().unwrap().local_jobs[0]
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(!completion
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap());
        assert_eq!(repo.head(), head);
        assert_eq!(repo.index_bytes("file.txt"), b"staged\n");
        assert!(state.lock().unwrap().local_jobs.is_empty());
    }

    #[test]
    fn missing_commit_identity_preserves_head_index_and_worktree() {
        let repo = TestRepo::new();
        let head = repo.commit("file", b"base\n");
        repo.write("file", b"staged\n");
        repo.git_ok(&["add", "file"]);
        repo.write("file", b"unstaged\n");
        repo.git_ok(&["config", "user.name", ""]);
        repo.git_ok(&["config", "user.email", ""]);
        repo.git_ok(&["config", "user.useConfigOnly", "true"]);
        let error = git_commit(repo.path(), "keep draft", false, false).unwrap_err();
        assert!(error.contains("Settings > Repository"), "{error}");
        assert_eq!(repo.head(), head);
        assert_eq!(repo.index_bytes("file"), b"staged\n");
        assert_eq!(
            std::fs::read(repo.path().join("file")).unwrap(),
            b"unstaged\n"
        );
    }

    #[test]
    fn failed_commit_signing_never_falls_back_to_unsigned() {
        let repo = TestRepo::new();
        let head = repo.commit("file", b"base\n");
        repo.write("file", b"staged\n");
        repo.git_ok(&["add", "file"]);
        let settings = super::super::commit_settings::Signing {
            enabled: true,
            format: "ssh".into(),
            key: repo
                .path()
                .join("missing-key")
                .to_string_lossy()
                .into_owned(),
        };
        super::super::commit_settings::save(repo.path(), &settings).unwrap();
        let error = git_commit(repo.path(), "keep signed draft", false, false).unwrap_err();
        assert!(error.contains("does not retry unsigned"), "{error}");
        assert_eq!(repo.head(), head);
        assert_eq!(repo.index_bytes("file"), b"staged\n");
        assert_eq!(
            std::fs::read(repo.path().join("file")).unwrap(),
            b"staged\n"
        );
    }

    #[test]
    #[ignore = "requires ssh-keygen; uses a disposable unencrypted fixture key"]
    fn ssh_signed_commit_verifies_with_disposable_key() {
        let repo = TestRepo::new();
        repo.commit("file", b"base\n");
        let keys = tempfile::tempdir().unwrap();
        let key = keys.path().join("fixture-key");
        let mut command = crate::git::process::background_command("ssh-keygen");
        command
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(&key);
        let output = crate::git::process::run(
            &mut command,
            &std::sync::atomic::AtomicBool::new(false),
            std::time::Duration::from_secs(30),
        )
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let public = std::fs::read_to_string(key.with_extension("pub")).unwrap();
        let allowed = keys.path().join("allowed_signers");
        std::fs::write(&allowed, format!("committer@example.com {public}")).unwrap();
        repo.git_ok(&[
            "config",
            "gpg.ssh.allowedSignersFile",
            allowed.to_str().unwrap(),
        ]);
        super::super::commit_settings::save(
            repo.path(),
            &super::super::commit_settings::Signing {
                enabled: true,
                format: "ssh".into(),
                key: key.to_string_lossy().into_owned(),
            },
        )
        .unwrap();
        repo.write("file", b"signed bytes\n");
        repo.git_ok(&["add", "file"]);
        git_commit(repo.path(), "signed fixture", false, false).unwrap();
        repo.git_ok(&["verify-commit", "HEAD"]);
        assert_eq!(repo.index_bytes("file"), b"signed bytes\n");
        git_commit(repo.path(), "signed amend", true, false).unwrap();
        repo.git_ok(&["verify-commit", "HEAD"]);
    }

    #[test]
    fn rejected_hook_does_not_change_head_or_index() {
        let repo = TestRepo::new();
        let head = repo.commit("file.txt", b"original\n");
        repo.write("file.txt", b"staged\n");
        repo.git_ok(&["add", "--", "file.txt"]);
        let hook = repo.path().join("empty-hooks/pre-commit");
        std::fs::write(&hook, "#!/bin/sh\necho rejected-by-test >&2\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let error = git_commit(repo.path(), "keep this message", false, false).unwrap_err();
        assert!(error.contains("rejected-by-test"));
        assert_eq!(repo.head(), head);
        assert_eq!(repo.index_bytes("file.txt"), b"staged\n");
    }

    #[test]
    fn message_only_amend_preserves_tree_and_authorship() {
        let repo = TestRepo::new();
        repo.commit("file.txt", b"original\n");
        git_commit(repo.path(), "reworded", true, false).unwrap();
        let opened = repo.open();
        let commit = opened.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(commit.message().ok(), Some("reworded\n"));
        assert_eq!(commit.author().email().ok(), Some("author@example.com"));
        assert_eq!(repo.index_bytes("file.txt"), b"original\n");
    }
    #[test]
    fn discard_restores_smudged_index_and_preserves_untracked_files() {
        let repo = TestRepo::new();
        repo.commit("base", b"base");
        repo.write(".gitattributes", b"*.filtered filter=recorde-fixture\n");
        repo.git_ok(&[
            "config",
            "filter.recorde-fixture.clean",
            "sed s/working/cleaned/g",
        ]);
        repo.git_ok(&[
            "config",
            "filter.recorde-fixture.smudge",
            "sed s/cleaned/working/g",
        ]);
        repo.write("file.filtered", b"working staged bytes\n");
        do_stage(repo.path(), "file.filtered").unwrap();
        repo.write("file.filtered", b"unstaged changes\n");
        do_discard_file(repo.path(), "file.filtered").unwrap();
        assert_eq!(repo.index_bytes("file.filtered"), b"cleaned staged bytes\n");
        assert_eq!(
            std::fs::read(repo.path().join("file.filtered")).unwrap(),
            b"working staged bytes\n"
        );
        repo.write("untracked", b"keep me");
        assert!(do_discard_file(repo.path(), "untracked").is_err());
        assert_eq!(
            std::fs::read(repo.path().join("untracked")).unwrap(),
            b"keep me"
        );
    }

    #[test]
    fn full_file_staging_runs_clean_filters_and_partial_staging_refuses_them() {
        let repo = TestRepo::new();
        repo.commit("base", b"base");
        repo.write(".gitattributes", b"*.filtered filter=recorde-fixture\n");
        repo.git_ok(&[
            "config",
            "filter.recorde-fixture.clean",
            "sed s/working/cleaned/g",
        ]);
        repo.write("file.filtered", b"working bytes\n");
        do_stage(repo.path(), "file.filtered").unwrap();
        assert_eq!(repo.index_bytes("file.filtered"), b"cleaned bytes\n");
        assert_eq!(
            std::fs::read(repo.path().join("file.filtered")).unwrap(),
            b"working bytes\n"
        );
        assert!(ensure_no_filter(repo.path(), "file.filtered").is_err());
        assert!(ensure_no_filter(repo.path(), "base").is_ok());
    }

    #[test]
    #[ignore = "requires git-lfs; run in the LFS CI step"]
    fn lfs_stage_push_clone_round_trip() {
        let repo = TestRepo::new();
        repo.commit("base", b"base");
        repo.git_ok(&["lfs", "install", "--local"]);
        repo.git_ok(&["lfs", "track", "*.asset"]);
        let bytes = b"binary fixture\0with content\r\n";
        repo.write("file.asset", bytes);
        do_stage_all(repo.path()).unwrap();
        assert!(repo
            .index_bytes("file.asset")
            .starts_with(b"version https://git-lfs.github.com/spec/v1\n"));
        git_commit(repo.path(), "LFS fixture", false, false).unwrap();
        let remote = tempfile::tempdir().unwrap();
        let bare = git2::Repository::init_bare(remote.path()).unwrap();
        bare.set_head("refs/heads/main").unwrap();
        repo.git_ok(&["remote", "add", "upstream", remote.path().to_str().unwrap()]);
        repo.git_ok(&["push", "-u", "upstream", "main"]);
        let root = tempfile::tempdir().unwrap();
        let cloned = root.path().join("cloned");
        // Explicit filter config keeps this fixture independent of global Git config.
        repo.git_ok(&[
            "-c",
            "filter.lfs.process=git-lfs filter-process",
            "-c",
            "filter.lfs.required=true",
            "clone",
            remote.path().to_str().unwrap(),
            cloned.to_str().unwrap(),
        ]);
        assert_eq!(std::fs::read(cloned.join("file.asset")).unwrap(), bytes);
        assert!(ensure_no_filter(repo.path(), "file.asset").is_err());
    }
}
