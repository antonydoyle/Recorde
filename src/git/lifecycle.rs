use std::path::{Path, PathBuf};

use crate::state::SharedState;

// ── Clone ─────────────────────────────────────────────────────────────────────

/// Clone `url` into `dest`, showing progress via `state.remote_op`.
/// Opens the cloned repo in the app on success.
pub fn clone_repo(url: &str, dest: PathBuf, state: SharedState) {
    let url = url.to_string();
    {
        let mut s = state.lock().unwrap();
        s.remote_op = Some("Cloning…".into());
        s.console_log.push(format!("$ git clone {} {}", url, dest.display()));
    }

    std::thread::spawn(move || {
        let result = do_clone(&url, &dest, &state);
        match result {
            Ok(()) => {
                state.lock().unwrap().remote_op = None;
                crate::git::operations::load_repo(dest, state);
            }
            Err(e) => {
                let mut s = state.lock().unwrap();
                s.remote_op = None;
                s.error = Some(e);
            }
        }
    });
}

fn do_clone(url: &str, dest: &Path, state: &SharedState) -> Result<(), String> {
    let mut attempts = 0u8;
    let mut callbacks = git2::RemoteCallbacks::new();

    callbacks.credentials(move |u, username, allowed| {
        attempts += 1;
        if attempts > 3 {
            return Err(git2::Error::from_str("Authentication failed — check SSH key or credentials"));
        }
        build_creds(u, username, allowed)
    });

    let state2 = state.clone();
    callbacks.transfer_progress(move |stats| {
        let pct = if stats.total_objects() > 0 {
            stats.received_objects() * 100 / stats.total_objects()
        } else {
            100
        };
        state2.lock().unwrap().remote_op = Some(format!("Cloning… {}%", pct));
        true
    });

    let mut fetch_opts = git2::FetchOptions::new();
    fetch_opts.remote_callbacks(callbacks);

    git2::build::RepoBuilder::new()
        .fetch_options(fetch_opts)
        .clone(url, dest)
        .map(|_| ())
        .map_err(|e| e.message().to_string())
}

// ── Init ──────────────────────────────────────────────────────────────────────

/// Initialise a new git repository at `path`, then open it in the app.
pub fn init_repo(path: PathBuf, state: SharedState) {
    {
        let mut s = state.lock().unwrap();
        s.console_log.push(format!("$ git init {}", path.display()));
    }
    std::thread::spawn(move || {
        match git2::Repository::init(&path) {
            Ok(_) => crate::git::operations::load_repo(path, state),
            Err(e) => state.lock().unwrap().error = Some(e.message().to_string()),
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

// ── Credential helper (mirrors remote.rs — kept local to avoid coupling) ──────

fn build_creds(url: &str, username: Option<&str>, allowed: git2::CredentialType) -> Result<git2::Cred, git2::Error> {
    if allowed.contains(git2::CredentialType::SSH_KEY) {
        let user = username.unwrap_or("git");
        if let Ok(c) = git2::Cred::ssh_key_from_agent(user) { return Ok(c); }
        if let Some(c) = try_ssh_key_files(user) { return Ok(c); }
    }
    if allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
        if let Some((u, p)) = git_credential_fill(url) {
            return git2::Cred::userpass_plaintext(&u, &p);
        }
    }
    if allowed.contains(git2::CredentialType::DEFAULT) { return git2::Cred::default(); }
    Err(git2::Error::from_str("No credentials available — configure an SSH key or run 'gh auth login'"))
}

fn try_ssh_key_files(username: &str) -> Option<git2::Cred> {
    let home = std::env::var("HOME").ok()?;
    let ssh = std::path::Path::new(&home).join(".ssh");
    for name in &["id_ed25519", "id_ecdsa", "id_rsa"] {
        let priv_key = ssh.join(name);
        if priv_key.exists() {
            let pub_key = ssh.join(format!("{}.pub", name));
            let pub_key = if pub_key.exists() { Some(pub_key) } else { None };
            if let Ok(c) = git2::Cred::ssh_key(username, pub_key.as_deref(), &priv_key, None) {
                return Some(c);
            }
        }
    }
    None
}

fn git_credential_fill(url: &str) -> Option<(String, String)> {
    use std::io::Write as _;
    let mut child = std::process::Command::new("git")
        .args(["credential", "fill"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn().ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "url={}\n", url);
    }
    let out = child.wait_with_output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let mut u = None; let mut p = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("username=") { u = Some(v.to_string()); }
        if let Some(v) = line.strip_prefix("password=") { p = Some(v.to_string()); }
    }
    Some((u?, p?))
}
