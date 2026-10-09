//! Reject path traversal and existing symlink/reparse ancestors before app file I/O.
use std::path::{Component, Path, PathBuf};

pub(crate) fn relative(file: &str) -> Result<&Path, String> {
    let path = Path::new(file);
    if file.is_empty() || file.contains('\0') || path.components().any(|part| {
        !matches!(part, Component::Normal(name) if !name.to_string_lossy().eq_ignore_ascii_case(".git"))
    }) {
        return Err("Expected a file path inside the repository worktree.".into());
    }
    Ok(path)
}

pub(crate) fn worktree_file(root: &Path, file: &str) -> Result<PathBuf, String> {
    checked_entry(root, file, false)
}

pub(crate) fn worktree_entry(root: &Path, file: &str) -> Result<PathBuf, String> {
    checked_entry(root, file, true)
}

fn checked_entry(root: &Path, file: &str, allow_link: bool) -> Result<PathBuf, String> {
    let relative = relative(file)?;
    let mut path = root.canonicalize().map_err(|e| e.to_string())?;
    let count = relative.components().count();
    for (index, component) in relative.components().enumerate() {
        path.push(component.as_os_str());
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => {
                let link = metadata.file_type().is_symlink();
                #[cfg(windows)]
                let link = {
                    use std::os::windows::fs::MetadataExt;
                    link || metadata.file_attributes() & 0x400 != 0
                };
                if link && !(allow_link && index + 1 == count && metadata.file_type().is_symlink())
                {
                    return Err(
                        "File preview/editing cannot follow a symlink or reparse point.".into(),
                    );
                }
                if index + 1 < count && !metadata.is_dir() {
                    return Err("A parent path is not a directory.".into());
                }
                if index + 1 == count
                    && !metadata.is_file()
                    && !(allow_link && metadata.file_type().is_symlink())
                {
                    return Err("Expected a regular file.".into());
                }
            }
            // A deleted directory may contain the missing ancestor side of a
            // rename conflict. Existing ancestors were still checked above.
            Err(e)
                if e.kind() == std::io::ErrorKind::NotFound
                    && (allow_link || index + 1 == count) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_traversal_absolute_and_git_metadata_paths() {
        for file in [
            "",
            "../outside",
            "/absolute",
            ".git/config",
            "dir/../../out",
            "dir/.GIT/hooks",
        ] {
            assert!(relative(file).is_err(), "{file}");
        }
        assert!(relative("src/normal[1].rs").is_ok());
    }
    #[cfg(unix)]
    #[test]
    fn refuses_symlink_ancestors_and_non_regular_files() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"outside").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("linked")).unwrap();
        assert!(worktree_file(root.path(), "linked/secret").is_err());
        assert!(worktree_file(root.path(), "linked/new-file").is_err());
        std::fs::create_dir(root.path().join("directory")).unwrap();
        assert!(worktree_file(root.path(), "directory").is_err());
    }
}
