//! Text conflict snapshots with optimistic checks before replacing the result.
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Snapshot {
    pub repo: PathBuf,
    pub file: String,
    pub base: String,
    pub ours: String,
    pub theirs: String,
    pub result: String,
    stages: [Option<git2::Oid>; 3],
    original: Vec<u8>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Draft {
    file: String,
    stages: [Option<String>; 3],
    original: String,
    pub result: String,
}

impl Draft {
    pub fn matches(&self, snapshot: &Snapshot) -> bool {
        self.file == snapshot.file
            && self.stages == snapshot.stages.map(|oid| oid.map(|oid| oid.to_string()))
            && self.original
                == git2::Oid::hash_object(git2::ObjectType::Blob, &snapshot.original)
                    .expect("Git blob hashing")
                    .to_string()
    }
}

fn draft_path(snapshot: &Snapshot) -> Result<PathBuf, String> {
    let repo = git2::Repository::open(&snapshot.repo).map_err(|e| e.to_string())?;
    let key = git2::Oid::hash_object(git2::ObjectType::Blob, snapshot.file.as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(repo
        .path()
        .join("recorde/conflict-drafts")
        .join(format!("{key}.json")))
}

pub fn read_draft(snapshot: &Snapshot) -> Result<Option<Draft>, String> {
    match std::fs::read(draft_path(snapshot)?) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| e.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

pub fn write_draft(snapshot: &Snapshot, result: &str) -> Result<(), String> {
    use std::io::Write;
    let path = draft_path(snapshot)?;
    let parent = path.parent().ok_or("Invalid draft path.")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let draft = Draft {
        file: snapshot.file.clone(),
        stages: snapshot.stages.map(|oid| oid.map(|oid| oid.to_string())),
        original: git2::Oid::hash_object(git2::ObjectType::Blob, &snapshot.original)
            .map_err(|e| e.to_string())?
            .to_string(),
        result: result.into(),
    };
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temp.write_all(&serde_json::to_vec(&draft).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

fn clear_saved_draft(snapshot: &Snapshot, result: &str) {
    // A second editor or instance may have saved a newer draft in the meantime.
    if read_draft(snapshot)
        .ok()
        .flatten()
        .is_some_and(|draft| draft.matches(snapshot) && draft.result == result)
    {
        if let Ok(path) = draft_path(snapshot) {
            if let Err(error) = std::fs::remove_file(path) {
                log::warn!("Could not remove resolved conflict draft: {error}");
            }
        }
    }
}

pub fn load(path: &Path, file: &str) -> Result<Snapshot, String> {
    super::safe_path::relative(file)?;
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let index = repo.index().map_err(|e| e.to_string())?;
    if [1, 2, 3]
        .iter()
        .filter_map(|stage| index.get_path(Path::new(file), *stage))
        .any(|entry| entry.mode == 0o120000 || entry.mode == 0o160000)
    {
        return Err(
            "Symlink and submodule conflicts require choosing a side or an external tool.".into(),
        );
    }
    let stages = [1, 2, 3].map(|stage| index.get_path(Path::new(file), stage).map(|e| e.id));
    if stages.iter().all(Option::is_none) {
        return Err("This file no longer has an unresolved conflict.".into());
    }
    let decode = |oid: Option<git2::Oid>| -> Result<String, String> {
        let bytes = match oid {
            Some(oid) => {
                let (size, _) = repo
                    .odb()
                    .and_then(|odb| odb.read_header(oid))
                    .map_err(|e| e.to_string())?;
                if size > 8 * 1024 * 1024 {
                    return Err("Conflict editor supports files up to 8 MiB. Use a side choice or external tool.".into());
                }
                repo.find_blob(oid)
                    .map_err(|e| e.to_string())?
                    .content()
                    .to_vec()
            }
            None => Vec::new(),
        };
        if bytes.contains(&0) {
            return Err(
                "Binary conflicts require choosing a side or using an external merge tool.".into(),
            );
        }
        String::from_utf8(bytes)
            .map_err(|_| "This encoding requires an external merge tool.".into())
    };
    let workdir = repo
        .workdir()
        .ok_or("Conflict editing requires a worktree.")?;
    let target = super::safe_path::worktree_file(workdir, file)?;
    let original = match std::fs::File::open(&target) {
        Ok(file) => {
            use std::io::Read;
            let mut bytes = Vec::new();
            file.take(8 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() > 8 * 1024 * 1024 {
                return Err("Conflict editor supports files up to 8 MiB.".into());
            }
            bytes
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.to_string()),
    };
    let result = String::from_utf8(original.clone())
        .map_err(|_| "This encoding requires an external merge tool.".to_string())?;
    Ok(Snapshot {
        repo: path.into(),
        file: file.into(),
        base: decode(stages[0])?,
        ours: decode(stages[1])?,
        theirs: decode(stages[2])?,
        result,
        stages,
        original,
    })
}

pub fn save(snapshot: &Snapshot, result: &str) -> Result<(), String> {
    use std::io::Write;
    if result.len() > 8 * 1024 * 1024 || result.contains('\0') {
        return Err(
            "Text conflict result must be UTF-8 text up to 8 MiB without NUL bytes.".into(),
        );
    }
    if result.lines().any(|line| {
        line.starts_with("<<<<<<< ") || line.starts_with(">>>>>>> ") || line == "======="
    }) {
        return Err("Resolve every conflict marker before saving.".into());
    }
    let current = load(&snapshot.repo, &snapshot.file)?;
    if current.stages != snapshot.stages || current.original != snapshot.original {
        return Err("The file or index changed while the editor was open. Reopen it to load the latest versions.".into());
    }
    let repo = git2::Repository::open(&snapshot.repo).map_err(|e| e.to_string())?;
    let path =
        super::safe_path::worktree_file(repo.workdir().ok_or("No worktree.")?, &snapshot.file)?;
    let parent = path.parent().ok_or("Invalid result path.")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    if let Ok(metadata) = std::fs::metadata(&path) {
        temp.as_file()
            .set_permissions(metadata.permissions())
            .map_err(|e| e.to_string())?;
    }
    temp.write_all(result.as_bytes())
        .map_err(|e| e.to_string())?;
    temp.persist(path).map_err(|e| e.to_string())?;
    super::merge::run_git_seq(
        &snapshot.repo,
        &["--literal-pathspecs", "add", "--", &snapshot.file],
    )
    .map_err(|e| e.user_message())?;
    clear_saved_draft(snapshot, result);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::TestRepo;

    #[cfg(unix)]
    #[test]
    fn conflict_editor_refuses_replaced_parent_and_oversized_working_file() {
        let repo = TestRepo::new();
        std::fs::create_dir(repo.path().join("dir")).unwrap();
        repo.commit("dir/file", b"base\n");
        repo.git_ok(&["checkout", "-b", "other"]);
        repo.commit("dir/file", b"theirs\n");
        repo.git_ok(&["checkout", "main"]);
        repo.commit("dir/file", b"ours\n");
        assert!(!repo.git(&["merge", "other"]).status.success());
        let snapshot = load(repo.path(), "dir/file").unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::rename(repo.path().join("dir"), outside.path().join("saved")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("saved"), repo.path().join("dir")).unwrap();
        assert!(load(repo.path(), "dir/file").is_err());
        assert!(save(&snapshot, "must not escape\n").is_err());
        assert_eq!(
            std::fs::read(outside.path().join("saved/file")).unwrap(),
            snapshot.original
        );
        std::fs::remove_file(repo.path().join("dir")).unwrap();
        std::fs::rename(outside.path().join("saved"), repo.path().join("dir")).unwrap();
        repo.write("dir/file", &vec![b'a'; 8 * 1024 * 1024 + 1]);
        assert!(load(repo.path(), "dir/file")
            .err()
            .unwrap()
            .contains("8 MiB"));
    }
    fn conflicted() -> TestRepo {
        let repo = TestRepo::new();
        repo.commit("file.txt", b"base\n");
        repo.git_ok(&["checkout", "-b", "other"]);
        repo.commit("file.txt", b"theirs\n");
        repo.git_ok(&["checkout", "main"]);
        repo.commit("file.txt", b"ours\n");
        assert!(!repo.git(&["merge", "other"]).status.success());
        repo
    }
    #[test]
    fn conflict_draft_survives_reopen_and_detects_changed_worktree() {
        let repo = conflicted();
        let snapshot = load(repo.path(), "file.txt").unwrap();
        write_draft(&snapshot, "unfinished combination\r\n").unwrap();
        let reopened = load(repo.path(), "file.txt").unwrap();
        let draft = read_draft(&reopened).unwrap().unwrap();
        assert!(draft.matches(&reopened));
        assert_eq!(draft.result, "unfinished combination\r\n");
        repo.write("file.txt", b"external changes\n");
        let changed = load(repo.path(), "file.txt").unwrap();
        assert!(!read_draft(&changed).unwrap().unwrap().matches(&changed));
        assert!(save(&snapshot, &draft.result).is_err());
        write_draft(&changed, "resolved\n").unwrap();
        save(&changed, "resolved\n").unwrap();
        assert!(read_draft(&changed).unwrap().is_none());
        assert_eq!(repo.index_bytes("file.txt"), b"resolved\n");
    }

    #[test]
    fn text_resolution_checks_staleness_and_stages_exact_result() {
        let repo = conflicted();
        let snapshot = load(repo.path(), "file.txt").unwrap();
        assert_eq!(snapshot.base, "base\n");
        assert_eq!(snapshot.ours, "ours\n");
        assert_eq!(snapshot.theirs, "theirs\n");
        assert!(save(&snapshot, &snapshot.result).is_err());
        repo.write("file.txt", b"external edit\n");
        assert!(save(&snapshot, "combined\n").is_err());
        assert_eq!(
            std::fs::read(repo.path().join("file.txt")).unwrap(),
            b"external edit\n"
        );
        let snapshot = load(repo.path(), "file.txt").unwrap();
        save(&snapshot, "combined\r\n").unwrap();
        assert!(!repo.open().index().unwrap().has_conflicts());
        assert_eq!(repo.index_bytes("file.txt"), b"combined\r\n");
        assert!(repo.path().join(".git/MERGE_HEAD").exists());
    }
}
