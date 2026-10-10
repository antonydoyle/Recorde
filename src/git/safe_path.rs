//! Reject path traversal and existing symlink/reparse ancestors before app file I/O.
use std::path::{Component, Path, PathBuf};

pub(crate) fn relative(file: &str) -> Result<&Path, String> {
    let path = Path::new(file);
    if file.is_empty() || file.contains('\0') || path.components().any(|part| {
        !matches!(part, Component::Normal(name) if !name.to_string_lossy().eq_ignore_ascii_case(".git"))
    }) {
        return Err("Expected a file path inside the repository worktree.".into());
    }
    #[cfg(windows)]
    for component in path.components() {
        let name = component.as_os_str().to_string_lossy();
        let stem = name
            .split('.')
            .next()
            .unwrap_or_default()
            .trim_end_matches(' ')
            .to_ascii_uppercase();
        if name.contains(':')
            || name.ends_with(['.', ' '])
            || matches!(
                stem.as_str(),
                "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
            )
            || stem
                .strip_prefix("COM")
                .or_else(|| stem.strip_prefix("LPT"))
                .is_some_and(|suffix| {
                    matches!(
                        suffix,
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    )
                })
            || stem.strip_prefix("GIT~").is_some_and(|suffix| {
                !suffix.is_empty() && suffix.bytes().all(|c| c.is_ascii_digit())
            })
        {
            return Err(
                "Windows device names, alternate streams and ambiguous paths are not supported."
                    .into(),
            );
        }
    }
    Ok(path)
}

/// Retain directory handles for application-owned file reads and replacements.
/// Each component is opened without following links. Unix uses relative handle
/// operations; Windows directory handles deny rename/delete sharing. Retain all
/// ancestors because cap-primitives uses path-based rename on Windows.
pub(crate) struct ScopedFile {
    parent: cap_std::fs::Dir,
    name: std::ffi::OsString,
    _ancestors: Vec<cap_std::fs::Dir>,
}

impl ScopedFile {
    pub fn new(root: &Path, file: &str) -> Result<Self, String> {
        use cap_fs_ext::DirExt;
        let relative = relative(file)?;
        let mut parent = cap_std::fs::Dir::open_ambient_dir(root, cap_std::ambient_authority())
            .map_err(|e| e.to_string())?;
        let mut parts = relative.components().peekable();
        let mut name = None;
        let mut ancestors = Vec::new();
        while let Some(part) = parts.next() {
            if parts.peek().is_some() {
                let next = parent.open_dir_nofollow(part.as_os_str()).map_err(|_| {
                    "Cannot open parent directory without following a link or reparse point."
                        .to_string()
                })?;
                #[cfg(windows)]
                {
                    use cap_std::fs::MetadataExt;
                    if next
                        .dir_metadata()
                        .map_err(|e| e.to_string())?
                        .file_attributes()
                        & 0x400
                        != 0
                    {
                        return Err("Cannot open a reparse point for worktree file I/O.".into());
                    }
                }
                ancestors.push(parent);
                parent = next;
            } else {
                name = Some(part.as_os_str().to_os_string());
            }
        }
        Ok(Self {
            parent,
            name: name.ok_or("Expected a file name.")?,
            _ancestors: ancestors,
        })
    }

    fn open_cap(&self) -> std::io::Result<cap_std::fs::File> {
        use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt, OpenOptionsSyncExt};
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No).nonblock(true);
        let file = self.parent.open_with(&self.name, &options)?;
        #[cfg(windows)]
        {
            use cap_std::fs::MetadataExt;
            if file.metadata()?.file_attributes() & 0x400 != 0 {
                return Err(std::io::Error::other("Cannot open a reparse point."));
            }
        }
        if !file.metadata()?.is_file() {
            return Err(std::io::Error::other("Expected a regular worktree file."));
        }
        Ok(file)
    }

    pub fn open(&self) -> std::io::Result<std::fs::File> {
        self.open_cap().map(|file| file.into_std())
    }

    pub fn read(&self, limit: u64) -> Result<Option<Vec<u8>>, String> {
        use std::io::Read;
        let file = match self.open() {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        let mut bytes = Vec::new();
        file.take(limit.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > limit {
            let size = if limit > 0 && limit % (1024 * 1024) == 0 {
                format!("{} MiB", limit / (1024 * 1024))
            } else {
                format!("{limit} bytes")
            };
            return Err(format!("File exceeds the editor size limit of {size}."));
        }
        Ok(Some(bytes))
    }

    pub fn replace(&self, expected: Option<&[u8]>, bytes: &[u8]) -> Result<(), String> {
        self.replace_checked(expected, bytes, || {})
    }

    fn replace_checked(
        &self,
        expected: Option<&[u8]>,
        bytes: &[u8],
        before_commit: impl FnOnce(),
    ) -> Result<(), String> {
        use std::io::Write;
        if self
            .read(expected.map_or(0, |data| data.len() as u64))?
            .as_deref()
            != expected
        {
            return Err("The file changed while it was being edited. Refresh and retry.".into());
        }
        let permissions = self
            .open_cap()
            .ok()
            .and_then(|f| f.metadata().ok())
            .map(|m| m.permissions());
        if permissions.as_ref().is_some_and(|p| p.readonly()) {
            return Err("The file is read-only. Make it writable before saving.".into());
        }
        let mut temporary = cap_tempfile::TempFile::new(&self.parent).map_err(|e| e.to_string())?;
        if let Some(permissions) = permissions {
            temporary
                .as_file()
                .set_permissions(permissions)
                .map_err(|e| e.to_string())?;
        }
        temporary.write_all(bytes).map_err(|e| e.to_string())?;
        temporary.as_file().sync_all().map_err(|e| e.to_string())?;
        before_commit();
        // Recheck after the potentially slow write/fsync, immediately before
        // replacement. External applications do not participate in our locks;
        // this is optimistic checking, not a filesystem compare-and-swap.
        if self
            .read(expected.map_or(0, |data| data.len() as u64))?
            .as_deref()
            != expected
        {
            return Err("The file changed while saving. Refresh and retry.".into());
        }
        temporary.replace(&self.name).map_err(|e| e.to_string())
    }
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
    fn edit_during_temporary_file_write_is_preserved() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("file"), b"original").unwrap();
        let target = ScopedFile::new(root.path(), "file").unwrap();
        assert!(target
            .replace_checked(Some(b"original"), b"ours", || {
                std::fs::write(root.path().join("file"), b"external edit").unwrap();
            })
            .is_err());
        assert_eq!(
            std::fs::read(root.path().join("file")).unwrap(),
            b"external edit"
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn windows_retained_ancestors_refuse_rename_and_readonly_leaves_no_temporary_files() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("outer/inner");
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("file");
        std::fs::write(&file, b"original").unwrap();
        let target = ScopedFile::new(root.path(), "outer/inner/file").unwrap();
        assert!(std::fs::rename(root.path().join("outer"), root.path().join("moved")).is_err());
        assert!(std::fs::rename(&folder, root.path().join("inner-moved")).is_err());
        let mut permissions = std::fs::metadata(&file).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&file, permissions.clone()).unwrap();
        let result = target.replace(Some(b"original"), b"replacement");
        permissions.set_readonly(false);
        std::fs::set_permissions(&file, permissions).unwrap();
        assert!(result.unwrap_err().contains("read-only"));
        assert_eq!(std::fs::read_dir(folder).unwrap().count(), 1);
        assert_eq!(std::fs::read(file).unwrap(), b"original");
    }
    #[test]
    fn scoped_replacement_rejects_concurrent_edits_and_preserves_missing_state() {
        let root = tempfile::tempdir().unwrap();
        let target = ScopedFile::new(root.path(), "file").unwrap();
        assert!(target.read(100).unwrap().is_none());
        target.replace(None, b"original").unwrap();
        assert_eq!(target.read(100).unwrap().unwrap(), b"original");
        assert!(target.replace(None, b"wrong missing assumption").is_err());
        std::fs::write(root.path().join("file"), b"external edits").unwrap();
        assert!(target.replace(Some(b"original"), b"overwrite").is_err());
        assert_eq!(
            std::fs::read(root.path().join("file")).unwrap(),
            b"external edits"
        );
    }

    #[cfg(unix)]
    #[test]
    fn retained_parent_cannot_be_redirected_by_a_concurrent_symlink_swap() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("folder")).unwrap();
        std::fs::write(root.path().join("folder/file"), b"original").unwrap();
        std::fs::write(outside.path().join("file"), b"outside secret").unwrap();
        let target = ScopedFile::new(root.path(), "folder/file").unwrap();
        std::fs::rename(root.path().join("folder"), root.path().join("parked")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("folder")).unwrap();
        assert_eq!(target.read(100).unwrap().unwrap(), b"original");
        target.replace(Some(b"original"), b"resolved").unwrap();
        assert_eq!(
            std::fs::read(outside.path().join("file")).unwrap(),
            b"outside secret"
        );
        assert!(ScopedFile::new(root.path(), "folder/file").is_err());
        std::fs::remove_file(root.path().join("parked/file")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("file"), root.path().join("parked/file"))
            .unwrap();
        assert!(target.read(100).is_err());
        assert!(target.replace(Some(b"resolved"), b"overwrite").is_err());
        assert_eq!(
            std::fs::read(outside.path().join("file")).unwrap(),
            b"outside secret"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_device_stream_and_ambiguous_names_are_rejected() {
        for name in [
            ".git./config",
            "file:stream",
            "NUL",
            "dir/COM1.txt",
            "LPT9",
            "dir/file.",
            "dir/file ",
            "GIT~1/config",
            "dir/COM¹.txt",
            "LPT³",
            "CONIN$",
            "CONOUT$",
            "CON .txt",
        ] {
            assert!(relative(name).is_err(), "{name}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_junction_cannot_redirect_scoped_reads_or_replacements() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("file"), b"outside secret").unwrap();
        let junction = root.path().join("junction");
        let mut command = super::super::process::background_command("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command",
            "$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path $env:RECORDE_TEST_JUNCTION -Target $env:RECORDE_TEST_TARGET | Out-Null"])
            .env("RECORDE_TEST_JUNCTION", &junction).env("RECORDE_TEST_TARGET", outside.path());
        let output = super::super::process::run(
            &mut command,
            &std::sync::atomic::AtomicBool::new(false),
            std::time::Duration::from_secs(20),
        )
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result = ScopedFile::new(root.path(), "junction/file");
        assert!(result.is_err());
        assert_eq!(
            std::fs::read(outside.path().join("file")).unwrap(),
            b"outside secret"
        );
        // Remove only the link. Never recursively traverse a test junction.
        std::fs::remove_dir(junction).unwrap();
    }
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
