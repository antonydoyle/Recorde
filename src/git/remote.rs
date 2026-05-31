use std::path::Path;

use crate::state::SharedState;

// ── Public API ────────────────────────────────────────────────────────────────

pub fn fetch_all(state: SharedState) {
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(path) = rp else { return };
    set_op(&state, "Fetching…");

    std::thread::spawn(move || {
        let result = do_fetch_all(&path, &state);
        finish_op(&state, result);
        crate::git::branches::reload_repo(state);
    });
}

pub fn push_branch(state: SharedState) {
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(path) = rp else { return };
    set_op(&state, "Pushing…");

    std::thread::spawn(move || {
        let result = do_push(&path, &state);
        finish_op(&state, result);
        crate::git::branches::reload_repo(state);
    });
}

pub fn pull_branch(state: SharedState) {
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(path) = rp else { return };
    set_op(&state, "Pulling…");

    std::thread::spawn(move || {
        let result = do_pull(&path, &state);
        finish_op(&state, result);
        crate::git::branches::reload_repo(state);
    });
}

// ── State helpers ─────────────────────────────────────────────────────────────

fn set_op(state: &SharedState, msg: &str) {
    let mut s = state.lock().unwrap();
    s.remote_op = Some(msg.to_string());
    s.console_log.push(format!("$ {}", msg));
}

fn finish_op(state: &SharedState, result: Result<(), git2::Error>) {
    let mut s = state.lock().unwrap();
    s.remote_op = None;
    match result {
        Err(e) => {
            let msg = e.message().to_string();
            s.console_log.push(format!("error: {}", msg));
            s.error = Some(msg);
        }
        Ok(()) => {
            s.console_log.push("Done.".into());
            s.error = None;
        }
    }
}

// ── Fetch ─────────────────────────────────────────────────────────────────────

fn do_fetch_all(path: &Path, state: &SharedState) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(path)?;
    let remote_names = repo.remotes()?;

    for name in remote_names.iter().flatten() {
        let mut remote = repo.find_remote(name)?;
        let mut callbacks = make_callbacks(state);

        let state2 = state.clone();
        let name2 = name.to_string();
        callbacks.transfer_progress(move |stats| {
            let pct = if stats.total_objects() > 0 {
                stats.received_objects() * 100 / stats.total_objects()
            } else {
                100
            };
            state2.lock().unwrap().remote_op =
                Some(format!("Fetching {} … {}%", name2, pct));
            true
        });

        let mut opts = git2::FetchOptions::new();
        opts.remote_callbacks(callbacks);
        opts.download_tags(git2::AutotagOption::Unspecified);

        remote.fetch(&[] as &[&str], Some(&mut opts), None)?;
    }
    Ok(())
}

// ── Push ──────────────────────────────────────────────────────────────────────

fn do_push(path: &Path, state: &SharedState) -> Result<(), git2::Error> {
    let repo = git2::Repository::open(path)?;
    let head = repo.head()?;
    let branch_name = head
        .shorthand()
        .ok_or_else(|| git2::Error::from_str("HEAD is not on a branch"))?
        .to_string();

    // Find which remote to push to (configured upstream remote, or "origin")
    let remote_name = repo
        .branch_upstream_remote(&format!("refs/heads/{}", branch_name))
        .map(|b| b.as_str().unwrap_or("origin").to_string())
        .unwrap_or_else(|_| "origin".to_string());

    let mut remote = repo.find_remote(&remote_name)?;

    let mut rejection: Option<String> = None;

    // Explicit scope so `callbacks` (which borrows `rejection`) is dropped
    // before we try to read `rejection` below.
    {
        let mut callbacks = make_callbacks(state);
        callbacks.push_update_reference(|_refname, status| {
            if let Some(msg) = status {
                rejection = Some(msg.to_string());
            }
            Ok(())
        });

        let state2 = state.clone();
        let remote2 = remote_name.clone();
        callbacks.push_transfer_progress(move |current, total, _bytes| {
            if total > 0 {
                state2.lock().unwrap().remote_op =
                    Some(format!("Pushing to {} … {}/{}", remote2, current, total));
            }
        });

        let mut push_opts = git2::PushOptions::new();
        push_opts.remote_callbacks(callbacks);
        let refspec = format!("refs/heads/{0}:refs/heads/{0}", branch_name);
        remote.push(&[refspec.as_str()], Some(&mut push_opts))?;
        // push_opts + callbacks dropped here → borrow of `rejection` ends
    }

    if let Some(msg) = rejection {
        return Err(git2::Error::from_str(&format!("Push rejected: {}", msg)));
    }
    Ok(())
}

// ── Pull (fetch + fast-forward) ───────────────────────────────────────────────

fn do_pull(path: &Path, state: &SharedState) -> Result<(), git2::Error> {
    // Step 1: fetch
    do_fetch_all(path, state)?;

    // Step 2: fast-forward if possible
    let repo = git2::Repository::open(path)?;
    let head = repo.head()?;
    let branch_name = head
        .shorthand()
        .ok_or_else(|| git2::Error::from_str("HEAD is not on a branch"))?
        .to_string();

    let branch = repo.find_branch(&branch_name, git2::BranchType::Local)?;
    let upstream = branch.upstream().map_err(|_| {
        git2::Error::from_str("No upstream configured — set one with 'git branch --set-upstream-to'")
    })?;

    let upstream_oid = upstream
        .get()
        .target()
        .ok_or_else(|| git2::Error::from_str("Upstream ref has no target"))?;

    let local_oid = head
        .target()
        .ok_or_else(|| git2::Error::from_str("HEAD has no target"))?;

    if local_oid == upstream_oid {
        return Ok(()); // Already up to date
    }

    let (ahead, behind) = repo.graph_ahead_behind(local_oid, upstream_oid)?;

    if behind == 0 {
        return Ok(()); // Nothing to pull
    }

    if ahead > 0 {
        return Err(git2::Error::from_str(
            "Branch has diverged from upstream — pull with rebase or merge manually",
        ));
    }

    // Fast-forward
    let upstream_commit = repo.find_commit(upstream_oid)?;
    let mut co_opts = git2::build::CheckoutBuilder::new();
    co_opts.safe();
    repo.checkout_tree(upstream_commit.as_object(), Some(&mut co_opts))?;
    repo.reference(
        &format!("refs/heads/{}", branch_name),
        upstream_oid,
        true,
        "pull: Fast-forward",
    )?;
    Ok(())
}

// ── Credential callback ───────────────────────────────────────────────────────

fn make_callbacks<'a>(state: &SharedState) -> git2::RemoteCallbacks<'a> {
    let _ = state; // reserved for future: pass credentials from a modal
    let mut attempts = 0u8;
    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.credentials(move |url, username, allowed| {
        attempts += 1;
        if attempts > 3 {
            return Err(git2::Error::from_str("Authentication failed — check your SSH key or credentials"));
        }
        build_creds(url, username, allowed)
    });
    callbacks
}

fn build_creds(
    url: &str,
    username: Option<&str>,
    allowed: git2::CredentialType,
) -> Result<git2::Cred, git2::Error> {
    // 1. SSH agent
    if allowed.contains(git2::CredentialType::SSH_KEY) {
        let user = username.unwrap_or("git");
        if let Ok(cred) = git2::Cred::ssh_key_from_agent(user) {
            return Ok(cred);
        }
        // Fall back to well-known key files
        if let Some(cred) = try_ssh_key_files(user) {
            return Ok(cred);
        }
    }

    // 2. HTTPS — try git-credential helper (works with GitHub CLI, GNOME Keyring, etc.)
    if allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
        if let Some((u, p)) = git_credential_fill(url) {
            return git2::Cred::userpass_plaintext(&u, &p);
        }
    }

    // 3. Default credentials (Kerberos / NTLM)
    if allowed.contains(git2::CredentialType::DEFAULT) {
        return git2::Cred::default();
    }

    Err(git2::Error::from_str(
        "No credentials available — configure an SSH key or run 'gh auth login'",
    ))
}

fn try_ssh_key_files(username: &str) -> Option<git2::Cred> {
    let home = std::env::var("HOME").ok()?;
    let ssh_dir = std::path::Path::new(&home).join(".ssh");
    for name in &["id_ed25519", "id_ecdsa", "id_rsa"] {
        let priv_key = ssh_dir.join(name);
        if priv_key.exists() {
            let pub_key = ssh_dir.join(format!("{}.pub", name));
            let pub_key = if pub_key.exists() { Some(pub_key) } else { None };
            if let Ok(cred) = git2::Cred::ssh_key(
                username,
                pub_key.as_deref(),
                &priv_key,
                None,
            ) {
                return Some(cred);
            }
        }
    }
    None
}

/// Call `git credential fill` to retrieve stored credentials (GitHub CLI,
/// GNOME Keyring, macOS Keychain, Windows Credential Manager, etc.)
fn git_credential_fill(url: &str) -> Option<(String, String)> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let mut child = Command::new("git")
        .args(["credential", "fill"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    // Write the URL so the credential helper knows which host to look up.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "url={}\n", url);
    }

    let out = child.wait_with_output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;

    let mut username = None;
    let mut password = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("username=") {
            username = Some(v.to_string());
        }
        if let Some(v) = line.strip_prefix("password=") {
            password = Some(v.to_string());
        }
    }
    Some((username?, password?))
}

// ── Ahead / behind (called from operations::load_repo_inner) ─────────────────

pub fn compute_all_ahead_behind(repo: &git2::Repository) -> std::collections::HashMap<String, (usize, usize)> {
    let mut map = std::collections::HashMap::new();
    let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) else { return map };
    for branch in branches.flatten() {
        let (branch, _) = branch;
        let Some(name) = branch.name().ok().flatten().map(str::to_owned) else { continue };
        let Some(local_oid) = branch.get().target() else { continue };
        let Ok(upstream) = branch.upstream() else { continue };
        let Some(upstream_oid) = upstream.get().target() else { continue };
        if let Ok((a, b)) = repo.graph_ahead_behind(local_oid, upstream_oid) {
            map.insert(name, (a, b));
        }
    }
    map
}

pub fn compute_ahead_behind(repo: &git2::Repository) -> Option<(usize, usize)> {
    let head = repo.head().ok()?;
    let local_oid = head.target()?;
    let branch_name = head.shorthand()?;
    let branch = repo.find_branch(branch_name, git2::BranchType::Local).ok()?;
    let upstream = branch.upstream().ok()?;
    let upstream_oid = upstream.get().target()?;
    repo.graph_ahead_behind(local_oid, upstream_oid).ok()
}
