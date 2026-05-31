use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc::Sender, Arc};

use crate::git::error::GitError;
use crate::git::job::{JobId, JobKind, JobProgress};
use crate::git::runner::GitRunner;
use crate::state::SharedState;

// ── Public API (called from app.rs) ──────────────────────────────────────────

pub fn fetch_all(runner: &mut GitRunner, state: &SharedState) {
    let Some(path) = repo_path(state) else { return };
    runner.submit(JobKind::FetchAll, path);
}

pub fn push_branch(runner: &mut GitRunner, state: &SharedState) {
    let Some(path) = repo_path(state) else { return };
    runner.submit(JobKind::PushBranch, path);
}

pub fn pull_branch(runner: &mut GitRunner, state: &SharedState) {
    let Some(path) = repo_path(state) else { return };
    runner.submit(JobKind::PullBranch, path);
}

fn repo_path(state: &SharedState) -> Option<PathBuf> {
    state.lock().unwrap().repo_path.clone()
}

// ── Runner dispatch functions (called from runner::dispatch) ──────────────────

pub fn run_fetch_all(
    path: &Path,
    cancel: &Arc<AtomicBool>,
    job_id: JobId,
    progress_tx: &Sender<JobProgress>,
) -> Result<(), GitError> {
    let repo = git2::Repository::open(path)?;
    let remote_names = repo.remotes()?;

    for name in remote_names.iter().flatten() {
        if cancel.load(Ordering::Relaxed) {
            return Err(GitError::Cancelled);
        }

        let mut remote = repo.find_remote(name)?;
        let cancel2 = cancel.clone();
        let progress_tx2 = progress_tx.clone();
        let name2 = name.to_string();

        let mut callbacks = make_callbacks();
        callbacks.transfer_progress(move |stats| {
            if cancel2.load(Ordering::Relaxed) {
                return false;
            }
            let pct = if stats.total_objects() > 0 {
                Some((stats.received_objects() * 100 / stats.total_objects()) as u8)
            } else {
                Some(100)
            };
            let _ = progress_tx2.send(JobProgress::Update {
                id: job_id,
                message: format!("Fetching {}…", name2),
                pct,
            });
            true
        });

        let mut opts = git2::FetchOptions::new();
        opts.remote_callbacks(callbacks);
        opts.download_tags(git2::AutotagOption::Unspecified);
        remote.fetch(&[] as &[&str], Some(&mut opts), None)?;
    }
    Ok(())
}

pub fn run_push(
    path: &Path,
    cancel: &Arc<AtomicBool>,
    job_id: JobId,
    progress_tx: &Sender<JobProgress>,
) -> Result<(), GitError> {
    let repo = git2::Repository::open(path)?;
    let head = repo.head()?;
    let branch_name = head
        .shorthand()
        .ok_or_else(|| git2::Error::from_str("HEAD is not on a branch"))?
        .to_string();

    let remote_name = repo
        .branch_upstream_remote(&format!("refs/heads/{}", branch_name))
        .map(|b| b.as_str().unwrap_or("origin").to_string())
        .unwrap_or_else(|_| "origin".to_string());

    let mut remote = repo.find_remote(&remote_name)?;
    let mut rejection: Option<String> = None;

    {
        let cancel2 = cancel.clone();
        let progress_tx2 = progress_tx.clone();
        let remote2 = remote_name.clone();

        let mut callbacks = make_callbacks();
        callbacks.push_update_reference(|_refname, status| {
            if let Some(msg) = status {
                rejection = Some(msg.to_string());
            }
            Ok(())
        });
        callbacks.push_transfer_progress(move |current, total, _bytes| {
            if total > 0 && !cancel2.load(Ordering::Relaxed) {
                let _ = progress_tx2.send(JobProgress::Update {
                    id: job_id,
                    message: format!("Pushing to {}…", remote2),
                    pct: Some((current * 100 / total) as u8),
                });
            }
        });

        let mut push_opts = git2::PushOptions::new();
        push_opts.remote_callbacks(callbacks);
        let refspec = format!("refs/heads/{0}:refs/heads/{0}", branch_name);
        remote.push(&[refspec.as_str()], Some(&mut push_opts))?;
    }

    if let Some(msg) = rejection {
        return Err(GitError::Git2(git2::Error::from_str(&format!("Push rejected: {}", msg))));
    }
    Ok(())
}

pub fn run_pull(
    path: &Path,
    cancel: &Arc<AtomicBool>,
    job_id: JobId,
    progress_tx: &Sender<JobProgress>,
) -> Result<(), GitError> {
    // Step 1: fetch
    run_fetch_all(path, cancel, job_id, progress_tx)?;

    if cancel.load(Ordering::Relaxed) {
        return Err(GitError::Cancelled);
    }

    // Step 2: fast-forward if possible
    let repo = git2::Repository::open(path)?;
    let head = repo.head()?;
    let branch_name = head
        .shorthand()
        .ok_or_else(|| git2::Error::from_str("HEAD is not on a branch"))?
        .to_string();

    let branch = repo.find_branch(&branch_name, git2::BranchType::Local)?;
    let upstream = branch.upstream().map_err(|_| {
        git2::Error::from_str(
            "No upstream configured — set one with 'git branch --set-upstream-to'",
        )
    })?;

    let upstream_oid = upstream
        .get()
        .target()
        .ok_or_else(|| git2::Error::from_str("Upstream ref has no target"))?;

    let local_oid = head
        .target()
        .ok_or_else(|| git2::Error::from_str("HEAD has no target"))?;

    if local_oid == upstream_oid {
        return Ok(());
    }

    let (ahead, behind) = repo.graph_ahead_behind(local_oid, upstream_oid)?;

    if behind == 0 {
        return Ok(());
    }

    if ahead > 0 {
        return Err(GitError::Diverged { ahead, behind });
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

fn make_callbacks<'a>() -> git2::RemoteCallbacks<'a> {
    let mut attempts = 0u8;
    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.credentials(move |url, username, allowed| {
        attempts += 1;
        if attempts > 3 {
            return Err(git2::Error::from_str(
                "Authentication failed — check your SSH key or credentials",
            ));
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
    if allowed.contains(git2::CredentialType::SSH_KEY) {
        let user = username.unwrap_or("git");
        if let Ok(cred) = git2::Cred::ssh_key_from_agent(user) {
            return Ok(cred);
        }
        if let Some(cred) = try_ssh_key_files(user) {
            return Ok(cred);
        }
    }

    if allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
        if let Some((u, p)) = git_credential_fill(url) {
            return git2::Cred::userpass_plaintext(&u, &p);
        }
    }

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
            if let Ok(cred) = git2::Cred::ssh_key(username, pub_key.as_deref(), &priv_key, None) {
                return Some(cred);
            }
        }
    }
    None
}

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

pub fn compute_all_ahead_behind(
    repo: &git2::Repository,
) -> std::collections::HashMap<String, (usize, usize)> {
    let mut map = std::collections::HashMap::new();
    let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) else {
        return map;
    };
    for branch in branches.flatten() {
        let (branch, _) = branch;
        let Some(name) = branch.name().ok().flatten().map(str::to_owned) else {
            continue;
        };
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
    let branch = repo
        .find_branch(branch_name, git2::BranchType::Local)
        .ok()?;
    let upstream = branch.upstream().ok()?;
    let upstream_oid = upstream.get().target()?;
    repo.graph_ahead_behind(local_oid, upstream_oid).ok()
}
