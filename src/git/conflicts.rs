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

    let Some(expected) = expected_version(&fp, &state) else {
        return;
    };
    crate::git::mutation::spawn_operation(rp, state, move |path| {
        verify_version(path, &fp, &expected)?;
        run_git_seq(path, &["--literal-pathspecs", "add", "--", &fp]).map_err(|e| e.user_message())
    });
}

/// Launch the configured merge tool for a conflicted file (fire-and-forget).
pub fn open_mergetool(file_path: &str, repo_path: &Path, state: SharedState) {
    let path = repo_path.to_path_buf();
    let file = file_path.to_string();
    crate::git::mutation::spawn_operation(path, state, move |path| {
        run_git_seq(
            path,
            &[
                "--literal-pathspecs",
                "mergetool",
                "--no-prompt",
                "--",
                &file,
            ],
        )
        .map_err(|e| e.user_message())
    });
}

// ── Internal ──────────────────────────────────────────────────────────────────

fn resolve_with(file_path: &str, state: SharedState, side: &'static str) {
    let fp = file_path.to_string();
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(rp) = rp else { return };

    let Some(expected) = expected_version(&fp, &state) else {
        return;
    };

    crate::git::mutation::spawn_operation(rp, state, move |path| {
        resolve_choice(path, &fp, side, expected)
    });
}

type Stages = [Option<(git2::Oid, u32)>; 3];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    stages: Stages,
    working: WorkingFile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WorkingFile {
    Missing,
    Link(std::path::PathBuf),
    File { digest: [u8; 32], executable: bool },
}

pub(crate) fn version(path: &Path, file: &str) -> Result<Version, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let stages = stages(path, file)?;
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let target = super::safe_path::worktree_entry(
        repo.workdir()
            .ok_or("Conflict resolution requires a worktree.")?,
        file,
    )?;
    let working = match std::fs::symlink_metadata(&target) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => WorkingFile::Missing,
        Err(e) => return Err(e.to_string()),
        Ok(metadata) if metadata.file_type().is_symlink() => {
            WorkingFile::Link(std::fs::read_link(&target).map_err(|e| e.to_string())?)
        }
        Ok(metadata) => {
            if !metadata.is_file() {
                return Err("Resolve directory conflicts using an external tool.".into());
            }
            let mut input = std::fs::File::open(&target).map_err(|e| e.to_string())?;
            let mut hasher = Sha256::new();
            let mut buffer = [0u8; 65536];
            loop {
                let count = input.read(&mut buffer).map_err(|e| e.to_string())?;
                if count == 0 {
                    break;
                }
                hasher.update(&buffer[..count]);
            }
            let after = std::fs::symlink_metadata(&target).map_err(|e| e.to_string())?;
            if after.file_type() != metadata.file_type()
                || after.len() != metadata.len()
                || after.modified().ok() != metadata.modified().ok()
            {
                return Err(
                    "The conflicted file changed while it was being read. Refresh and retry."
                        .into(),
                );
            }
            #[cfg(unix)]
            let executable = {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o111 != 0
            };
            #[cfg(not(unix))]
            let executable = false;
            WorkingFile::File {
                digest: hasher.finalize().into(),
                executable,
            }
        }
    };
    Ok(Version { stages, working })
}

fn expected_version(file: &str, state: &SharedState) -> Option<Version> {
    let mut state = state.lock().unwrap();
    let expected = state
        .conflicted_files
        .iter()
        .find(|entry| entry.path == file)
        .and_then(|entry| entry.version.clone());
    if expected.is_none() {
        state.error = Some("Refresh the conflict before resolving it. Directory conflicts require an external tool.".into());
    }
    expected
}

fn verify_version(path: &Path, file: &str, expected: &Version) -> Result<(), String> {
    if &version(path, file)? != expected {
        return Err("The conflicted file or index changed. Refresh and review those edits before resolving it.".into());
    }
    Ok(())
}

fn stages(path: &Path, file: &str) -> Result<Stages, String> {
    super::safe_path::relative(file)?;
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let index = repo.index().map_err(|e| e.to_string())?;
    let stages = [1, 2, 3].map(|stage| {
        index
            .get_path(Path::new(file), stage)
            .map(|e| (e.id, e.mode))
    });
    if stages.iter().all(Option::is_none) {
        return Err("This file no longer has an unresolved conflict.".into());
    }
    Ok(stages)
}

fn resolve_choice(path: &Path, file: &str, side: &str, expected: Version) -> Result<(), String> {
    verify_version(path, file, &expected)?;
    let current = expected.stages;
    let index = match side {
        "ours" => 1,
        "theirs" => 2,
        _ => return Err("Invalid conflict side.".into()),
    };
    if current[index].is_none() {
        // A missing side means deletion, including modify/delete and rename/delete.
        run_git_seq(path, &["--literal-pathspecs", "rm", "--force", "--", file])
    } else {
        run_git_seq(
            path,
            &[
                "--literal-pathspecs",
                "checkout",
                &format!("--{side}"),
                "--",
                file,
            ],
        )
        .and_then(|()| run_git_seq(path, &["--literal-pathspecs", "add", "--", file]))
    }
    .map_err(|e| e.user_message())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::TestRepo;

    fn deleted_by_theirs(file: &str) -> TestRepo {
        let repo = TestRepo::new();
        repo.commit(file, b"base\n");
        repo.git_ok(&["checkout", "-b", "other"]);
        repo.git_ok(&["--literal-pathspecs", "rm", "--", file]);
        repo.git_ok(&["commit", "-m", "delete"]);
        repo.git_ok(&["checkout", "main"]);
        repo.commit(file, b"ours\n");
        assert!(!repo.git(&["merge", "other"]).status.success());
        repo
    }

    #[test]
    fn modify_delete_choices_preserve_or_delete_exact_literal_path() {
        for side in ["ours", "theirs"] {
            let repo = deleted_by_theirs("file[1].txt");
            repo.write("file1.txt", b"unrelated\n");
            let expected = version(repo.path(), "file[1].txt").unwrap();
            resolve_choice(repo.path(), "file[1].txt", side, expected).unwrap();
            assert!(!repo.open().index().unwrap().has_conflicts());
            assert_eq!(
                std::fs::read(repo.path().join("file1.txt")).unwrap(),
                b"unrelated\n"
            );
            if side == "ours" {
                assert_eq!(repo.index_bytes("file[1].txt"), b"ours\n");
            } else {
                assert!(!repo.path().join("file[1].txt").exists());
            }
        }
    }

    #[test]
    fn stale_conflict_choice_cannot_overwrite_a_resolved_file() {
        let repo = deleted_by_theirs("file.txt");
        let expected = version(repo.path(), "file.txt").unwrap();
        repo.write("file.txt", b"manual resolution\n");
        repo.git_ok(&["add", "file.txt"]);
        assert!(resolve_choice(repo.path(), "file.txt", "theirs", expected).is_err());
        assert_eq!(repo.index_bytes("file.txt"), b"manual resolution\n");
    }

    #[test]
    fn queued_choice_refuses_unstaged_manual_edits() {
        let repo = deleted_by_theirs("file.txt");
        let expected = version(repo.path(), "file.txt").unwrap();
        repo.write("file.txt", b"manual edits not staged\n");
        assert!(resolve_choice(repo.path(), "file.txt", "theirs", expected).is_err());
        assert!(repo.open().index().unwrap().has_conflicts());
        assert_eq!(
            std::fs::read(repo.path().join("file.txt")).unwrap(),
            b"manual edits not staged\n"
        );
    }

    #[test]
    fn binary_conflict_choices_preserve_exact_bytes() {
        for side in ["ours", "theirs"] {
            let repo = TestRepo::new();
            repo.commit("binary", b"\0base");
            repo.git_ok(&["checkout", "-b", "other"]);
            repo.commit("binary", b"\0theirs\xff");
            repo.git_ok(&["checkout", "main"]);
            repo.commit("binary", b"\0ours\xfe");
            assert!(!repo.git(&["merge", "other"]).status.success());
            let expected = version(repo.path(), "binary").unwrap();
            resolve_choice(repo.path(), "binary", side, expected).unwrap();
            let bytes = if side == "ours" {
                b"\0ours\xfe".as_slice()
            } else {
                b"\0theirs\xff".as_slice()
            };
            assert_eq!(repo.index_bytes("binary"), bytes);
            assert_eq!(std::fs::read(repo.path().join("binary")).unwrap(), bytes);
            assert!(!repo.open().index().unwrap().has_conflicts());
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_choice_changes_link_without_touching_target() {
        use std::os::unix::fs::symlink;
        let repo = TestRepo::new();
        repo.commit("sentinel", b"must survive\n");
        let link = repo.path().join("link");
        symlink("base", &link).unwrap();
        repo.git_ok(&["add", "link"]);
        repo.git_ok(&["commit", "-m", "base link"]);
        repo.git_ok(&["checkout", "-b", "other"]);
        std::fs::remove_file(&link).unwrap();
        symlink("sentinel", &link).unwrap();
        repo.git_ok(&["commit", "-am", "other link"]);
        repo.git_ok(&["checkout", "main"]);
        std::fs::remove_file(&link).unwrap();
        symlink("ours", &link).unwrap();
        repo.git_ok(&["commit", "-am", "our link"]);
        assert!(!repo.git(&["merge", "other"]).status.success());
        let expected = version(repo.path(), "link").unwrap();
        resolve_choice(repo.path(), "link", "theirs", expected).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("sentinel"));
        assert_eq!(repo.index_bytes("link"), b"sentinel");
        assert_eq!(
            std::fs::read(repo.path().join("sentinel")).unwrap(),
            b"must survive\n"
        );
    }
}
