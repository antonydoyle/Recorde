use std::path::{Path, PathBuf};

use crate::state::SharedState;

/// Build Git's disposable history index; refs, the index and working files are untouched.
pub fn optimize_history(path: &Path) -> Result<(), String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    if repo.is_shallow() {
        return Err("Fetch full history before optimizing it.".into());
    }
    if repo
        .config()
        .map_err(|e| e.to_string())?
        .get_bool("core.commitGraph")
        .is_ok_and(|enabled| !enabled)
    {
        return Err(
            "Git's core.commitGraph setting is disabled. Enable it before optimizing history."
                .into(),
        );
    }
    if repo
        .references_glob("refs/replace/*")
        .map_err(|e| e.to_string())?
        .next()
        .is_some()
    {
        return Err(
            "History optimization is unavailable while replacement refs are present.".into(),
        );
    }
    super::merge::run_git_seq(path, &["commit-graph", "write", "--reachable", "--split"])
        .map_err(|e| e.user_message())
}

pub fn fetch_full_history(path: &Path) -> Result<(), String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    if !repo.is_shallow() {
        return Ok(());
    }
    let remote = super::remote::provider_remote(&repo)?;
    super::process::git_network(
        path,
        &["fetch", "--unshallow", "--", &remote],
        &std::sync::atomic::AtomicBool::new(false),
        &mut |_, _| {},
    )
    .map(|_| ())
    .map_err(|e| e.user_message())
}

pub fn set_sparse(path: &Path, directories: Option<Vec<String>>) -> Result<(), String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let status = super::process::git(
        path,
        &["status", "--porcelain=v1", "-z", "--untracked-files=normal"],
        &std::sync::atomic::AtomicBool::new(false),
    )
    .map_err(|e| e.user_message())?;
    if repo.state() != git2::RepositoryState::Clean || !status.stdout.is_empty() {
        return Err(
            "Commit or stash changes, including untracked files, before changing sparse checkout."
                .into(),
        );
    }
    let args = if let Some(directories) = &directories {
        if directories.is_empty() {
            return Err("Choose at least one directory.".into());
        }
        for directory in directories {
            super::safe_path::relative(directory)?;
        }
        let mut args = vec!["sparse-checkout", "set", "--cone", "--"];
        args.extend(directories.iter().map(String::as_str));
        args
    } else {
        vec!["sparse-checkout", "disable"]
    };
    super::merge::run_git_seq(path, &args).map_err(|e| e.user_message())
}

// ── Clone ─────────────────────────────────────────────────────────────────────

/// Clone `url` into `dest`, showing progress via `state.remote_op`.
/// Opens the cloned repo in the app on success.
pub fn clone_repo(url: &str, dest: PathBuf, depth: u32, state: SharedState) {
    let url = url.to_string();
    {
        let mut s = state.lock().unwrap();
        s.console_log.push(format!(
            "$ git clone {} {}",
            crate::auth::redact::text(&url),
            dest.display()
        ));
    }

    let request = {
        let s = state.lock().unwrap();
        (s.repo_load_id, s.repo_epoch)
    };
    let work_dest = dest.clone();
    crate::git::mutation::spawn_job_with_progress(
        dest.clone(),
        "Clone repository",
        state,
        move |progress| {
            do_clone_depth_progress(&url, &work_dest, depth, &mut |message, pct| {
                progress.update(message, pct);
            })
        },
        move |result, state| {
            let current = {
                let s = state.lock().unwrap();
                (s.repo_load_id, s.repo_epoch) == request
            };
            if !current {
                let mut s = state.lock().unwrap();
                match result {
                    Ok(()) => s
                        .console_log
                        .push(format!("Clone ready: {}", dest.display())),
                    Err(error) => s
                        .console_log
                        .push(crate::auth::redact::text(&format!("Clone failed: {error}"))),
                }
                return;
            }
            match result {
                Ok(()) => {
                    state.lock().unwrap().remote_op = None;
                    crate::git::operations::load_repo(dest, state);
                }
                Err(e) => {
                    let mut s = state.lock().unwrap();
                    s.remote_op = None;
                    s.error = Some(crate::auth::redact::text(&e));
                }
            }
        },
    );
}

#[cfg(test)]
fn do_clone(url: &str, dest: &Path) -> Result<(), String> {
    do_clone_depth(url, dest, 0)
}

#[cfg(test)]
fn do_clone_depth(url: &str, dest: &Path, depth: u32) -> Result<(), String> {
    do_clone_depth_progress(url, dest, depth, &mut |_, _| {})
}

fn do_clone_depth_progress(
    url: &str,
    dest: &Path,
    depth: u32,
    on_progress: &mut dyn FnMut(&str, Option<u8>),
) -> Result<(), String> {
    let destination = if dest.is_absolute() {
        dest.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(dest)
    };
    // Not canonicalized: on Windows that yields a `\\?\` path Git cannot clone into.
    let parent = destination
        .parent()
        .ok_or("Invalid clone destination.")?
        .to_path_buf();
    destination
        .file_name()
        .ok_or("Invalid clone destination.")?;
    let mut command = super::process::command(&parent);
    command.env_remove("RECORDE_CREDENTIAL_REPO");
    command.args(["clone", "--progress"]);
    if depth > 0 {
        command.arg(format!("--depth={depth}"));
    }
    command.args(["--", url]).arg(&destination);
    // Claim a new directory atomically; never clean up a destination owned by someone else.
    let created = match std::fs::symlink_metadata(&destination) {
        Ok(_) => false,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(&destination).map_err(|e| e.to_string())?;
            true
        }
        Err(error) => return Err(error.to_string()),
    };
    let result = super::process::run_network(
        &mut command,
        &std::sync::atomic::AtomicBool::new(false),
        on_progress,
    )
    .map_err(|e| e.user_message())
    .and_then(|output| {
        if output.status.success() {
            Ok(())
        } else {
            Err(crate::auth::redact::text(
                String::from_utf8_lossy(&output.stderr).trim(),
            ))
        }
    });
    if created && result.is_err() {
        cleanup_clone_destination(&destination).map_err(|error| {
            crate::auth::redact::text(&format!(
                "{}\nCould not remove the incomplete clone: {error}",
                result.as_ref().unwrap_err()
            ))
        })?;
    }
    result
}

/// remove_dir_all does not traverse directory symlinks; also reject a replaced root.
fn cleanup_clone_destination(destination: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            std::fs::remove_dir_all(destination)
        }
        Ok(_) => Err(std::io::Error::other(
            "Clone destination was replaced; left in place.",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

// ── Init ──────────────────────────────────────────────────────────────────────

/// Initialise a new git repository at `path`, add the chosen starter files,
/// then open it in the app.
pub fn init_repo(path: PathBuf, options: super::templates::InitOptions, state: SharedState) {
    {
        let mut s = state.lock().unwrap();
        s.console_log.push(format!("$ git init {}", path.display()));
    }
    let request = {
        let s = state.lock().unwrap();
        (s.repo_load_id, s.repo_epoch)
    };
    let work_path = path.clone();
    crate::git::mutation::spawn_job(
        path.clone(),
        "Initialize repository",
        state,
        move || do_init(&work_path, &options),
        move |result, state| {
            let current = {
                let s = state.lock().unwrap();
                (s.repo_load_id, s.repo_epoch) == request
            };
            match result {
                Ok(skipped) => {
                    let mut s = state.lock().unwrap();
                    if !skipped.is_empty() {
                        s.error = Some(format!(
                            "Kept existing {}; the template was not written over it.",
                            skipped.join(", ")
                        ));
                    }
                    if !current {
                        s.console_log
                            .push(format!("Repository ready: {}", path.display()));
                        return;
                    }
                    drop(s);
                    crate::git::operations::load_repo(path, state);
                }
                Err(error) => {
                    let mut s = state.lock().unwrap();
                    s.console_log.push(format!("Init failed: {error}"));
                    if current {
                        s.error = Some(error);
                    }
                }
            }
        },
    );
}

/// Returns starter files that already existed and were left unchanged.
pub(crate) fn do_init(
    path: &Path,
    options: &super::templates::InitOptions,
) -> Result<Vec<String>, String> {
    let (written, skipped) = create_with_templates(path, options)?;
    if options.initial_commit {
        commit_starter_files(path, &written)?;
    }
    Ok(skipped)
}

/// Returns (written, skipped) starter files.
fn create_with_templates(
    path: &Path,
    options: &super::templates::InitOptions,
) -> Result<(Vec<&'static str>, Vec<String>), String> {
    let branch = options.default_branch.trim();
    if branch.is_empty() || !git2::Branch::name_is_valid(branch).unwrap_or(false) {
        return Err(format!("\"{branch}\" is not a valid branch name."));
    }
    if path.join(".git").exists() {
        return Err("This folder is already a Git repository. Open it instead.".into());
    }
    std::fs::create_dir_all(path).map_err(|e| e.to_string())?;
    let mut init = git2::RepositoryInitOptions::new();
    init.initial_head(branch);
    git2::Repository::init_opts(path, &init).map_err(|e| e.to_string())?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Repository".into());
    let mut written = Vec::new();
    let mut skipped = Vec::new();
    for (file, contents) in super::templates::files(options, &name) {
        let target = super::safe_path::ScopedFile::new(path, file)?;
        // Any existing entry, even one too large or unreadable, is left alone.
        if !matches!(target.read(0), Ok(None)) {
            skipped.push(file.to_string());
            continue;
        }
        target.replace(None, contents.as_bytes())?;
        written.push(file);
    }
    Ok((written, skipped))
}

/// Commit only the generated files; anything already in the folder stays untracked.
fn commit_starter_files(path: &Path, written: &[&str]) -> Result<(), String> {
    if !written.is_empty() {
        let mut add = vec!["add", "--"];
        add.extend(written.iter().copied());
        super::merge::run_git_seq(path, &add).map_err(|e| e.user_message())?;
    }
    super::merge::run_git_seq(path, &["commit", "--allow-empty", "-m", "Initial commit"])
        .map_err(|e| e.user_message())
}

// ── Terminal ──────────────────────────────────────────────────────────────────

/// Open a terminal emulator in `repo_path`.
pub fn open_terminal(repo_path: &Path) {
    let _ = try_open_terminal(repo_path);
}

/// Open a terminal and report launch failures to the caller.
pub fn try_open_terminal(repo_path: &Path) -> Result<(), String> {
    crate::external::open_terminal(repo_path, &crate::config::Config::load().terminal)
}

// ── File manager ──────────────────────────────────────────────────────────────

/// Reveal `repo_path` in the system file manager.
pub fn reveal_in_file_manager(repo_path: &Path) {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg(repo_path)
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(repo_path).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer.exe")
            .arg(repo_path)
            .spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_clone_removes_only_a_new_destination() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing-source");
        let new_destination = root.path().join("new-clone");
        assert!(do_clone(missing.to_str().unwrap(), &new_destination).is_err());
        assert!(!new_destination.exists());

        let existing_destination = root.path().join("existing-clone");
        std::fs::create_dir(&existing_destination).unwrap();
        assert!(do_clone(missing.to_str().unwrap(), &existing_destination).is_err());
        assert!(existing_destination.is_dir());
        assert_eq!(std::fs::read_dir(existing_destination).unwrap().count(), 0);
    }

    #[test]
    fn cancelled_clone_removes_only_a_new_destination() {
        let root = tempfile::tempdir().unwrap();
        for existed in [false, true] {
            let destination = root.path().join(if existed { "existing" } else { "new" });
            if existed {
                std::fs::create_dir(&destination).unwrap();
            }
            let result = super::super::process::with_cancellation(
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
                || do_clone("unused-source", &destination),
            );
            assert!(result.unwrap_err().contains("cancelled"));
            assert_eq!(destination.is_dir(), existed);
        }
    }

    #[cfg(unix)]
    #[test]
    fn clone_cleanup_never_follows_symlinks() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("keep");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("file"), b"keep").unwrap();
        let destination = root.path().join("clone");
        symlink(&target, &destination).unwrap();
        assert!(cleanup_clone_destination(&destination).is_err());
        assert_eq!(std::fs::read(target.join("file")).unwrap(), b"keep");
        std::fs::remove_file(&destination).unwrap();
        std::fs::create_dir(&destination).unwrap();
        symlink(&target, destination.join("link")).unwrap();
        cleanup_clone_destination(&destination).unwrap();
        assert!(!destination.exists());
        assert_eq!(std::fs::read(target.join("file")).unwrap(), b"keep");
    }

    #[test]
    fn shallow_clone_can_fetch_full_history() {
        let source = crate::git::test_support::TestRepo::new();
        source.commit("file", b"first\n");
        let tip = source.commit("file", b"second\n");
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("shallow");
        let url = url::Url::from_file_path(source.path()).unwrap();
        do_clone_depth(url.as_str(), &dest, 1).unwrap();
        assert!(git2::Repository::open(&dest).unwrap().is_shallow());
        fetch_full_history(&dest).unwrap();
        let repo = git2::Repository::open(&dest).unwrap();
        assert!(!repo.is_shallow());
        assert_eq!(repo.head().unwrap().target(), Some(tip));
    }

    #[test]
    fn sparse_checkout_preserves_dirty_work_and_can_be_disabled() {
        let repo = crate::git::test_support::TestRepo::new();
        std::fs::create_dir(repo.path().join("included")).unwrap();
        std::fs::create_dir(repo.path().join("excluded")).unwrap();
        repo.commit("included/file", b"keep\n");
        repo.commit("excluded/file", b"hide\n");
        repo.write("included/file", b"dirty\n");
        assert!(set_sparse(repo.path(), Some(vec!["included".into()])).is_err());
        assert_eq!(
            std::fs::read(repo.path().join("included/file")).unwrap(),
            b"dirty\n"
        );
        repo.write("included/file", b"keep\n");
        set_sparse(repo.path(), Some(vec!["included".into()])).unwrap();
        assert!(!repo.path().join("excluded/file").exists());
        assert_eq!(repo.index_bytes("excluded/file"), b"hide\n");
        set_sparse(repo.path(), None).unwrap();
        assert_eq!(
            std::fs::read(repo.path().join("excluded/file")).unwrap(),
            b"hide\n"
        );
    }
    use crate::git::test_support::TestRepo;

    #[test]
    fn init_writes_starter_files_keeps_existing_ones_and_commits() {
        use crate::git::templates::{InitOptions, License};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("new project");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(
            path.join("README.md"),
            b"keep me
",
        )
        .unwrap();
        let mut options = InitOptions {
            default_branch: "trunk".into(),
            gitignore: Some(0),
            readme: true,
            license: Some(License::Mit),
            author: "Test Author".into(),
            initial_commit: false,
        };
        options.default_branch = "bad..name".into();
        assert!(do_init(&path, &options).is_err());
        assert!(!path.join(".git").exists());
        options.default_branch = "trunk".into();
        let skipped = do_init(&path, &options).unwrap();
        assert_eq!(skipped, vec!["README.md".to_string()]);
        assert_eq!(
            std::fs::read(path.join("README.md")).unwrap(),
            b"keep me
"
        );
        assert!(std::fs::read_to_string(path.join(".gitignore"))
            .unwrap()
            .contains("/target/"));
        assert!(std::fs::read_to_string(path.join("LICENSE"))
            .unwrap()
            .contains("Test Author"));
        let repo = git2::Repository::open(&path).unwrap();
        assert_eq!(
            repo.find_reference("HEAD")
                .unwrap()
                .symbolic_target()
                .ok()
                .flatten(),
            Some("refs/heads/trunk")
        );
        assert!(do_init(&path, &options).is_err(), "never re-initializes");

        // The initial commit records only the generated files.
        let other = root.path().join("committed");
        std::fs::create_dir(&other).unwrap();
        std::fs::write(
            other.join("notes.txt"),
            b"private
",
        )
        .unwrap();
        options.readme = false;
        let (written, _) = create_with_templates(&other, &options).unwrap();
        let repo = git2::Repository::open(&other).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test Committer").unwrap();
        config
            .set_str("user.email", "committer@example.com")
            .unwrap();
        config.set_bool("commit.gpgsign", false).unwrap();
        commit_starter_files(&other, &written).unwrap();
        let tree = repo.head().unwrap().peel_to_tree().unwrap();
        let names: Vec<_> = tree.iter().map(|e| e.name().unwrap().to_string()).collect();
        assert_eq!(names, [".gitignore", "LICENSE"]);
    }

    #[test]
    fn native_clone_preserves_existing_destination_and_checks_out_bytes() {
        let source = TestRepo::new();
        let head = source.commit("file", b"source bytes\n");
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("clone space");
        do_clone(source.path().to_str().unwrap(), &dest).unwrap();
        assert_eq!(
            git2::Repository::open(&dest)
                .unwrap()
                .head()
                .unwrap()
                .target(),
            Some(head)
        );
        assert_eq!(std::fs::read(dest.join("file")).unwrap(), b"source bytes\n");
        std::fs::write(dest.join("file"), b"keep local changes\n").unwrap();
        assert!(do_clone(source.path().to_str().unwrap(), &dest).is_err());
        assert_eq!(
            std::fs::read(dest.join("file")).unwrap(),
            b"keep local changes\n"
        );
    }
}
