//! Reviewed single-file actions. Ignore is additive and deliberately outside Undo.
use super::{recovery, safe_path};
use std::path::{Path, PathBuf};

const LIMIT: u64 = 16 * 1024 * 1024;

/// The checkout policy under which raw bytes were verified in the saved tree.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct CheckoutGuard {
    file: String,
    attributes: Vec<u8>,
    configuration: Vec<(String, Option<String>)>,
}
impl CheckoutGuard {
    fn capture(path: &Path, file: &str) -> Result<Self, String> {
        safe_path::relative(file)?;
        if file.len() > 4096
            || Path::new(file)
                .file_name()
                .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(".gitattributes"))
        {
            return Err("Delete does not support attribute-control files or paths longer than 4096 bytes. Move this file outside the repository instead.".into());
        }
        let mut command = super::process::command(path);
        command.args([
            "--literal-pathspecs",
            "check-attr",
            "-z",
            "filter",
            "text",
            "eol",
            "working-tree-encoding",
            "ident",
            "--",
            file,
        ]);
        let output = super::process::run(
            &mut command,
            &std::sync::atomic::AtomicBool::new(false),
            std::time::Duration::from_secs(10),
        )
        .map_err(|e| e.user_message())?;
        if !output.status.success() {
            return Err("Cannot verify Git attributes for exact-byte recovery.".into());
        }
        let fields: Vec<_> = output.stdout.split(|b| *b == 0).collect();
        let attributes = ["filter", "text", "eol", "working-tree-encoding", "ident"];
        if fields.len() != 16
            || fields.last() != Some(&b"".as_slice())
            || fields[..15]
                .chunks_exact(3)
                .zip(attributes)
                .any(|(parts, name)| {
                    parts[0] != file.as_bytes()
                        || parts[1] != name.as_bytes()
                        || !matches!(parts[2], b"unspecified" | b"unset")
                })
        {
            return Err("Delete cannot promise exact-byte Undo for files with Git filters, text/EOL conversion, encoding or ident attributes. Move this file outside the repository instead.".into());
        }
        let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
        let config = repo.config().map_err(|e| e.to_string())?;
        if config.get_bool("core.autocrlf").unwrap_or(false)
            || config
                .get_string("core.autocrlf")
                .is_ok_and(|s| s.eq_ignore_ascii_case("input"))
        {
            return Err("Delete cannot promise exact-byte Undo while core.autocrlf is enabled. Move this file outside the repository instead.".into());
        }
        let configuration = ["core.autocrlf", "core.eol", "core.safecrlf"]
            .iter()
            .map(|name| (name.to_string(), config.get_string(name).ok()))
            .collect();
        Ok(Self {
            file: file.into(),
            attributes: output.stdout,
            configuration,
        })
    }
    pub(crate) fn validate(&self, path: &Path) -> Result<(), String> {
        if &Self::capture(path, &self.file)? != self {
            return Err("Git attributes or EOL configuration changed since this recovery snapshot. Restore the reviewed policy before using Undo or Redo; saved bytes are retained.".into());
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct Preview {
    pub path: PathBuf,
    pub epoch: u64,
    pub file: String,
    pub bytes: Vec<u8>,
    head: (Option<String>, Option<git2::Oid>),
    index: git2::Oid,
    guard: CheckoutGuard,
}
fn head(repo: &git2::Repository) -> Result<(Option<String>, Option<git2::Oid>), String> {
    let reference = repo.find_reference("HEAD").map_err(|e| e.to_string())?;
    Ok((
        reference
            .symbolic_target()
            .map_err(|e| e.to_string())?
            .map(str::to_owned),
        reference.resolve().ok().and_then(|r| r.target()),
    ))
}
fn untracked(repo: &git2::Repository, file: &str) -> Result<(), String> {
    if repo.state() != git2::RepositoryState::Clean
        || repo
            .status_file(Path::new(file))
            .map_err(|e| e.to_string())?
            != git2::Status::WT_NEW
        || repo
            .status_should_ignore(Path::new(file))
            .map_err(|e| e.to_string())?
    {
        return Err("The selected path is no longer an untracked, nonignored file in a clean repository. Refresh and review again.".into());
    }
    Ok(())
}
impl Preview {
    pub fn capture(path: &Path, epoch: u64, file: &str) -> Result<Self, String> {
        let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
        untracked(&repo, file)?;
        let guard = CheckoutGuard::capture(path, file)?;
        let bytes = safe_path::ScopedFile::new(path, file)?
            .read(LIMIT)?
            .ok_or("The file no longer exists.")?;
        let index = repo
            .index()
            .map_err(|e| e.to_string())?
            .write_tree()
            .map_err(|e| e.to_string())?;
        let result = Self {
            path: path.canonicalize().map_err(|e| e.to_string())?,
            epoch,
            file: file.into(),
            bytes,
            head: head(&repo)?,
            index,
            guard,
        };
        result.validate(path)?;
        Ok(result)
    }
    fn validate(&self, path: &Path) -> Result<(), String> {
        if path.canonicalize().map_err(|e| e.to_string())? != self.path {
            return Err("Repository changed; review the file again.".into());
        }
        let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
        untracked(&repo, &self.file)?;
        if head(&repo)? != self.head
            || repo
                .index()
                .map_err(|e| e.to_string())?
                .write_tree()
                .map_err(|e| e.to_string())?
                != self.index
            || safe_path::ScopedFile::new(path, &self.file)?
                .read(LIMIT)?
                .as_deref()
                != Some(self.bytes.as_slice())
        {
            return Err(
                "The file, HEAD or staging area changed. Refresh and review Delete again.".into(),
            );
        }
        self.guard.validate(path)
    }
    pub fn submit(self, state: crate::state::SharedState) {
        let (path, epoch) = {
            let s = state.lock().unwrap();
            (s.repo_path.clone(), s.repo_epoch)
        };
        if path.as_ref() != Some(&self.path) || epoch != self.epoch {
            return;
        }
        let completion_state = state.clone();
        super::mutation::spawn_operation(self.path.clone(), state, move |path| {
            {
                let s = completion_state.lock().unwrap();
                if s.repo_path.as_ref() != Some(&self.path) || s.repo_epoch != self.epoch {
                    return Err("Repository session changed; review Delete again.".into());
                }
            }
            self.execute(path)
        });
    }
    fn execute(&self, path: &Path) -> Result<(), String> {
        super::process::check_local_cancellation()?;
        self.validate(path)?;
        recovery::protect_deleted_file(path, &self.file, &self.bytes, self.guard.clone(), || {
            self.validate(path)?;
            super::process::check_local_cancellation()?;
            safe_path::ScopedFile::new(path, &self.file)?.remove(&self.bytes)
        })
    }
}

#[derive(Clone)]
pub struct IgnorePreview {
    pub path: PathBuf,
    pub epoch: u64,
    pub file: String,
    pub rule: String,
    original: Option<Vec<u8>>,
    replacement: Vec<u8>,
}
impl IgnorePreview {
    pub fn capture(path: &Path, epoch: u64, file: &str) -> Result<Self, String> {
        safe_path::relative(file)?;
        if file.contains(['\r', '\n']) {
            return Err(
                "Git ignore rules cannot represent a filename containing a newline.".into(),
            );
        }
        let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
        untracked(&repo, file)?;
        let target = safe_path::ScopedFile::new(path, ".gitignore")?;
        let original = target.read(1_048_576)?;
        let mut rule = String::from("/");
        for ch in file.chars() {
            if matches!(ch, '\\' | '*' | '?' | '[' | ']' | ' ' | '#' | '!') {
                rule.push('\\');
            }
            rule.push(ch);
        }
        let mut replacement = original.clone().unwrap_or_default();
        if !replacement
            .split(|b| *b == b'\n')
            .any(|line| line.strip_suffix(b"\r").unwrap_or(line) == rule.as_bytes())
        {
            if !replacement.is_empty() && !replacement.ends_with(b"\n") {
                replacement.push(b'\n');
            }
            replacement.extend_from_slice(rule.as_bytes());
            replacement.push(b'\n');
        }
        Ok(Self {
            path: path.canonicalize().map_err(|e| e.to_string())?,
            epoch,
            file: file.into(),
            rule,
            original,
            replacement,
        })
    }
    pub fn submit(self, state: crate::state::SharedState) {
        let (path, epoch) = {
            let s = state.lock().unwrap();
            (s.repo_path.clone(), s.repo_epoch)
        };
        if path.as_ref() != Some(&self.path) || epoch != self.epoch {
            return;
        }
        let current = state.clone();
        super::mutation::spawn_operation(self.path.clone(), state, move |path| {
            {
                let s = current.lock().unwrap();
                if s.repo_path.as_ref() != Some(&self.path) || s.repo_epoch != self.epoch {
                    return Err("Repository session changed; review Ignore again.".into());
                }
            }
            self.execute(path)
        });
    }
    fn execute(&self, path: &Path) -> Result<(), String> {
        untracked(
            &git2::Repository::open(path).map_err(|e| e.to_string())?,
            &self.file,
        )?;
        super::process::check_local_cancellation()?;
        safe_path::ScopedFile::new(path, ".gitignore")?
            .replace(self.original.as_deref(), &self.replacement)
    }
}
