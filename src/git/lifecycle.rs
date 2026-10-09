use std::path::{Path, PathBuf};

use crate::state::SharedState;

pub fn fetch_full_history(path: &Path) -> Result<(), String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    if !repo.is_shallow() { return Ok(()); }
    let remote = super::remote::provider_remote(&repo)?;
    super::merge::run_git_seq(path, &["fetch", "--unshallow", "--", &remote]).map_err(|e| e.user_message())
}

pub fn set_sparse(path: &Path, directories: Option<Vec<String>>) -> Result<(), String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let status = super::process::git(path, &["status", "--porcelain=v1", "-z", "--untracked-files=normal"], &std::sync::atomic::AtomicBool::new(false)).map_err(|e| e.user_message())?;
    if repo.state() != git2::RepositoryState::Clean || !status.stdout.is_empty() {
        return Err("Commit or stash changes, including untracked files, before changing sparse checkout.".into());
    }
    let args = if let Some(directories) = &directories {
        if directories.is_empty() { return Err("Choose at least one directory.".into()); }
        for directory in directories { super::safe_path::relative(directory)?; }
        let mut args = vec!["sparse-checkout", "set", "--cone", "--"];
        args.extend(directories.iter().map(String::as_str));
        args
    } else { vec!["sparse-checkout", "disable"] };
    super::merge::run_git_seq(path, &args).map_err(|e| e.user_message())
}

// ── Clone ─────────────────────────────────────────────────────────────────────

/// Clone `url` into `dest`, showing progress via `state.remote_op`.
/// Opens the cloned repo in the app on success.
pub fn clone_repo(url: &str, dest: PathBuf, depth: u32, state: SharedState) {
    let url = url.to_string();
    {
        let mut s = state.lock().unwrap();
        s.console_log.push(format!("$ git clone {} {}", crate::auth::redact::text(&url), dest.display()));
    }

    let request = { let s = state.lock().unwrap(); (s.repo_load_id, s.repo_epoch) };
    let work_dest = dest.clone();
    crate::git::mutation::spawn_job(dest.clone(), "Clone repository", state, move || do_clone_depth(&url, &work_dest, depth), move |result, state| {
        let current = { let s = state.lock().unwrap(); (s.repo_load_id, s.repo_epoch) == request };
        if !current {
            let mut s = state.lock().unwrap();
            match result {
                Ok(()) => s.console_log.push(format!("Clone ready: {}", dest.display())),
                Err(error) => s.console_log.push(crate::auth::redact::text(&format!("Clone failed: {error}"))),
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
    });
}

#[cfg(test)]
fn do_clone(url: &str, dest: &Path) -> Result<(), String> { do_clone_depth(url, dest, 0) }

fn do_clone_depth(url: &str, dest: &Path, depth: u32) -> Result<(), String> {
    let destination = if dest.is_absolute() { dest.to_path_buf() } else {
        std::env::current_dir().map_err(|e| e.to_string())?.join(dest)
    };
    let parent = destination.parent().ok_or("Invalid clone destination.")?;
    let mut command = super::process::command(parent);
    command.env_remove("RECORDE_CREDENTIAL_REPO");
    command.arg("clone");
    if depth > 0 { command.arg(format!("--depth={depth}")); }
    command.args(["--", url]).arg(&destination);
    let output = super::process::run(&mut command, &std::sync::atomic::AtomicBool::new(false), std::time::Duration::from_secs(600)).map_err(|e| e.user_message())?;
    if output.status.success() { Ok(()) } else {
        Err(crate::auth::redact::text(String::from_utf8_lossy(&output.stderr).trim()))
    }
}

// ── Init ──────────────────────────────────────────────────────────────────────

/// Initialise a new git repository at `path`, then open it in the app.
pub fn init_repo(path: PathBuf, state: SharedState) {
    {
        let mut s = state.lock().unwrap();
        s.console_log.push(format!("$ git init {}", path.display()));
    }
    let request = { let s = state.lock().unwrap(); (s.repo_load_id, s.repo_epoch) };
    let work_path = path.clone();
    crate::git::mutation::spawn_job(path.clone(), "Initialize repository", state, move || {
        git2::Repository::init(&work_path).map(|_| ()).map_err(|e| e.to_string())
    }, move |result, state| {
        let current = { let s = state.lock().unwrap(); (s.repo_load_id, s.repo_epoch) == request };
        match result {
            Ok(()) if current => crate::git::operations::load_repo(path, state),
            Ok(()) => state.lock().unwrap().console_log.push(format!("Repository ready: {}", path.display())),
            Err(error) => {
                let mut s = state.lock().unwrap();
                s.console_log.push(format!("Init failed: {error}"));
                if current { s.error = Some(error); }
            }
        }
    });
}

// ── Terminal ──────────────────────────────────────────────────────────────────

/// Open a terminal emulator in `repo_path`.
pub fn open_terminal(repo_path: &Path) {
    #[cfg(target_os = "linux")]
    {
        for term in &["xdg-terminal-exec", "xterm", "konsole", "gnome-terminal", "xfce4-terminal", "alacritty", "kitty"] {
            if std::process::Command::new(term).current_dir(repo_path).spawn().is_ok() {
                return;
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .args(["-a", "Terminal"])
            .arg(repo_path)
            .spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "cmd"])
            .current_dir(repo_path)
            .spawn();
    }
}

// ── File manager ──────────────────────────────────────────────────────────────

/// Reveal `repo_path` in the system file manager.
pub fn reveal_in_file_manager(repo_path: &Path) {
    #[cfg(all(unix, not(target_os = "macos")))]
    { let _ = std::process::Command::new("xdg-open").arg(repo_path).spawn(); }
    #[cfg(target_os = "macos")]
    { let _ = std::process::Command::new("open").arg(repo_path).spawn(); }
    #[cfg(target_os = "windows")]
    { let _ = std::process::Command::new("explorer.exe").arg(repo_path).spawn(); }
}


#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(std::fs::read(repo.path().join("included/file")).unwrap(), b"dirty\n");
        repo.write("included/file", b"keep\n");
        set_sparse(repo.path(), Some(vec!["included".into()])).unwrap();
        assert!(!repo.path().join("excluded/file").exists());
        assert_eq!(repo.index_bytes("excluded/file"), b"hide\n");
        set_sparse(repo.path(), None).unwrap();
        assert_eq!(std::fs::read(repo.path().join("excluded/file")).unwrap(), b"hide\n");
    }
    use crate::git::test_support::TestRepo;
    #[test]
    fn native_clone_preserves_existing_destination_and_checks_out_bytes() {
        let source = TestRepo::new();
        let head = source.commit("file", b"source bytes\n");
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("clone space");
        do_clone(source.path().to_str().unwrap(), &dest).unwrap();
        assert_eq!(git2::Repository::open(&dest).unwrap().head().unwrap().target(), Some(head));
        assert_eq!(std::fs::read(dest.join("file")).unwrap(), b"source bytes\n");
        std::fs::write(dest.join("file"), b"keep local changes\n").unwrap();
        assert!(do_clone(source.path().to_str().unwrap(), &dest).is_err());
        assert_eq!(std::fs::read(dest.join("file")).unwrap(), b"keep local changes\n");
    }
}
