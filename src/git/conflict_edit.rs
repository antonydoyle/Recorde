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
    original: Option<Vec<u8>>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Draft {
    file: String,
    stages: [Option<String>; 3],
    original: String,
    #[serde(default)]
    original_missing: bool,
    pub result: String,
}

impl Draft {
    pub fn matches(&self, snapshot: &Snapshot) -> bool {
        self.file == snapshot.file
            && self.stages == snapshot.stages.map(|oid| oid.map(|oid| oid.to_string()))
            && self.original_missing == snapshot.original.is_none()
            && self.original
                == git2::Oid::hash_object(
                    git2::ObjectType::Blob,
                    snapshot.original.as_deref().unwrap_or_default(),
                )
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
    use std::io::Read;
    match std::fs::File::open(draft_path(snapshot)?) {
        Ok(file) => {
            // JSON escaping can expand an 8 MiB result by up to six times.
            let mut bytes = Vec::new();
            file.take(64 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() > 64 * 1024 * 1024 {
                return Err("Conflict draft exceeds its size limit.".into());
            }
            serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| e.to_string())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

pub fn write_draft(snapshot: &Snapshot, result: &str) -> Result<(), String> {
    use std::io::Write;
    if result.len() > 8 * 1024 * 1024 {
        return Err("Conflict drafts support up to 8 MiB.".into());
    }
    let path = draft_path(snapshot)?;
    let parent = path.parent().ok_or("Invalid draft path.")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let draft = Draft {
        file: snapshot.file.clone(),
        stages: snapshot.stages.map(|oid| oid.map(|oid| oid.to_string())),
        original: git2::Oid::hash_object(
            git2::ObjectType::Blob,
            snapshot.original.as_deref().unwrap_or_default(),
        )
        .map_err(|e| e.to_string())?
        .to_string(),
        original_missing: snapshot.original.is_none(),
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
    let original = super::safe_path::ScopedFile::new(workdir, file)?.read(8 * 1024 * 1024)?;
    let result = String::from_utf8(original.clone().unwrap_or_default())
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

fn has_conflict_markers(text: &str, size: usize) -> bool {
    if !(1..=64).contains(&size) {
        return false;
    }
    let is_marker = |line: &str, marker: u8| {
        let bytes = line.as_bytes();
        bytes.len() >= size
            && bytes[..size].iter().all(|byte| *byte == marker)
            && (bytes.len() == size || bytes[size] == b' ')
    };
    let mut has_boundary = false;
    for line in text.lines() {
        let start = is_marker(line, b'<');
        let base = is_marker(line, b'|');
        let end = is_marker(line, b'>');
        if start || base || end {
            has_boundary = true;
        }
    }
    // A separator only counts with a start, base, or end marker, all of which
    // are rejected on their own here.
    has_boundary
}

pub fn save(snapshot: &Snapshot, result: &str) -> Result<(), String> {
    if result.len() > 8 * 1024 * 1024 || result.contains('\0') {
        return Err(
            "Text conflict result must be UTF-8 text up to 8 MiB without NUL bytes.".into(),
        );
    }
    let repo = git2::Repository::open(&snapshot.repo).map_err(|e| e.to_string())?;
    let marker_size = repo
        .get_attr(
            Path::new(&snapshot.file),
            "conflict-marker-size",
            git2::AttrCheckFlags::FILE_THEN_INDEX,
        )
        .ok()
        .flatten()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|size| (1..=64).contains(size))
        .unwrap_or(7);
    if has_conflict_markers(result, marker_size) {
        return Err("Resolve every conflict marker before saving.".into());
    }
    let current = load(&snapshot.repo, &snapshot.file)?;
    if current.stages != snapshot.stages || current.original != snapshot.original {
        return Err("The file or index changed while the editor was open. Reopen it to load the latest versions.".into());
    }
    let target =
        super::safe_path::ScopedFile::new(repo.workdir().ok_or("No worktree.")?, &snapshot.file)?;
    let original = target.read(8 * 1024 * 1024)?;
    if original != snapshot.original {
        return Err("The conflicted file changed before saving. Refresh and retry.".into());
    }
    target.replace(original.as_deref(), result.as_bytes())?;
    if target.read(8 * 1024 * 1024)?.as_deref() != Some(result.as_bytes()) {
        return Err("The resolution changed before staging. Inspect the file and retry.".into());
    }
    super::merge::run_git_seq(
        &snapshot.repo,
        &["--literal-pathspecs", "add", "--", &snapshot.file],
    )
    .map_err(|e| e.user_message())?;
    if target.read(8 * 1024 * 1024)?.as_deref() != Some(result.as_bytes()) {
        return Err("The resolution changed during staging. Inspect the index and working file before continuing; your draft is retained.".into());
    }
    clear_saved_draft(snapshot, result);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::TestRepo;

    #[test]
    fn conflict_marker_detection_handles_labels_bare_markers_and_diff3_bases() {
        assert!(has_conflict_markers(
            "<<<<<<< ours\n=======\n>>>>>>> theirs",
            7
        ));
        assert!(has_conflict_markers("<<<<<<<\n=======\n>>>>>>>", 7));
        assert!(has_conflict_markers("||||||| base\n", 7));
    }

    #[test]
    fn conflict_marker_detection_handles_crlf_and_lone_separators() {
        assert!(has_conflict_markers(
            "<<<<<<< ours\r\n=======\r\n>>>>>>> theirs\r\n",
            7
        ));
        assert!(!has_conflict_markers("Title\n=======\nclean text\n", 7));
        assert!(has_conflict_markers("=======\n>>>>>>> theirs\n", 7));
    }

    #[test]
    fn conflict_marker_detection_uses_exact_configured_size() {
        let marker = "<<<<<<<<<< ours\n==========\n>>>>>>>>>> theirs\n";
        assert!(has_conflict_markers(marker, 10));
        assert!(!has_conflict_markers(marker, 7));
        assert!(!has_conflict_markers("<<<<<<<<\n", 7));
    }

    #[test]
    fn save_honors_conflict_marker_size_attribute() {
        let repo = conflicted();
        repo.write(".gitattributes", b"file.txt conflict-marker-size=10\n");
        let snapshot = load(repo.path(), "file.txt").unwrap();
        assert_eq!(
            save(&snapshot, "<<<<<<<<<< ours\n").unwrap_err(),
            "Resolve every conflict marker before saving."
        );
    }

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
            snapshot.original.unwrap()
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
    fn empty_file_deletion_is_a_new_resolution_and_is_not_overwritten() {
        let repo = conflicted();
        repo.write("file.txt", b"");
        let snapshot = load(repo.path(), "file.txt").unwrap();
        write_draft(&snapshot, "draft").unwrap();
        std::fs::remove_file(repo.path().join("file.txt")).unwrap();
        let missing = load(repo.path(), "file.txt").unwrap();
        assert!(!read_draft(&missing).unwrap().unwrap().matches(&missing));
        assert!(save(&snapshot, "must not recreate").is_err());
        assert!(!repo.path().join("file.txt").exists());
        repo.write("file.txt", b"");
        assert!(save(&missing, "must not overwrite new file").is_err());
        assert!(repo.open().index().unwrap().has_conflicts());
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
