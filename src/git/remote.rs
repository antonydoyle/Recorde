use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc::Sender, Arc};

use crate::git::error::GitError;
use crate::git::job::{JobId, JobKind, JobProgress};
use crate::git::runner::GitRunner;
use crate::state::SharedState;

#[cfg(test)]
#[path = "remote_tests.rs"]
mod tests;

// ── Public API (called from app.rs) ──────────────────────────────────────────

pub fn fetch_all(runner: &mut GitRunner, state: &SharedState) {
    let Some(path) = repo_path(state) else { return };
    runner.submit(JobKind::FetchAll, path);
}

pub fn pull_branch(runner: &mut GitRunner, state: &SharedState) {
    let Some(path) = repo_path(state) else { return };
    let kind = match crate::config::Config::load().pull_strategy {
        crate::config::PullStrategy::Merge => JobKind::PullBranch,
        crate::config::PullStrategy::Rebase => JobKind::PullWithRebase,
        crate::config::PullStrategy::FfOnly => JobKind::PullFfOnly,
    };
    runner.submit(kind, path);
}

pub fn pull_with_rebase(runner: &mut GitRunner, state: &SharedState) {
    let Some(path) = repo_path(state) else { return };
    runner.submit(JobKind::PullWithRebase, path);
}

pub fn pull_ff_only(runner: &mut GitRunner, state: &SharedState) {
    let Some(path) = repo_path(state) else { return };
    runner.submit(JobKind::PullFfOnly, path);
}

pub fn push_branch(runner: &mut GitRunner, state: &SharedState) {
    let Some(path) = repo_path(state) else { return };
    runner.submit(JobKind::PushBranch, path);
}

pub fn push_with_lease(runner: &mut GitRunner, state: &SharedState) {
    let Some(path) = repo_path(state) else { return };
    match capture_lease(&path) {
        Ok(lease) => {
            runner.submit(JobKind::PushWithLease(lease), path);
        }
        Err(error) => state.lock().unwrap().error = Some(error.user_message()),
    }
}

pub fn push_set_upstream(runner: &mut GitRunner, state: &SharedState) {
    let Some(path) = repo_path(state) else { return };
    runner.submit(JobKind::PushSetUpstream, path);
}

fn repo_path(state: &SharedState) -> Option<PathBuf> {
    state.lock().unwrap().repo_path.clone()
}

// ── Runner dispatch functions (called from runner::dispatch) ──────────────────

pub fn run_pull_with_rebase(
    path: &Path,
    cancel: &Arc<AtomicBool>,
    _id: JobId,
    _tx: &Sender<JobProgress>,
) -> Result<(), GitError> {
    pull(path, cancel, "rebase")
}
pub fn run_pull_ff_only(
    path: &Path,
    cancel: &Arc<AtomicBool>,
    _id: JobId,
    _tx: &Sender<JobProgress>,
) -> Result<(), GitError> {
    pull(path, cancel, "ff-only")
}
pub fn run_pull(
    path: &Path,
    cancel: &Arc<AtomicBool>,
    _id: JobId,
    _tx: &Sender<JobProgress>,
) -> Result<(), GitError> {
    pull(path, cancel, "merge")
}
fn pull(path: &Path, cancel: &AtomicBool, strategy: &str) -> Result<(), GitError> {
    let (remote, _, _) = upstream(path)?;
    fetch_remote(
        path,
        &remote,
        cancel,
        crate::config::Config::load().fetch_prune,
    )?;
    let args: &[&str] = match strategy {
        "rebase" => &["rebase", "--no-autostash", "@{upstream}"],
        "ff-only" => &["merge", "--ff-only", "@{upstream}"],
        _ => &["merge", "--no-edit", "@{upstream}"],
    };
    super::process::git(path, args, cancel).map(|_| ())
}

fn upstream(path: &Path) -> Result<(String, String, String), GitError> {
    let repo = git2::Repository::open(path)?;
    if repo.head_detached()? {
        return Err(git2::Error::from_str("Select a branch before syncing.").into());
    }
    let head = repo.head()?;
    let branch = head
        .shorthand()
        .ok()
        .ok_or_else(|| git2::Error::from_str("HEAD has no branch."))?;
    let config = repo.config()?;
    let remote = config
        .get_string(&format!("branch.{branch}.remote"))
        .map_err(|_| {
            git2::Error::from_str("No upstream configured. Use Publish branch and select a remote.")
        })?;
    let destination = config.get_string(&format!("branch.{branch}.merge"))?;
    if !destination.starts_with("refs/heads/") {
        return Err(git2::Error::from_str("Upstream is not a branch.").into());
    }
    Ok((remote, format!("refs/heads/{branch}"), destination))
}

#[derive(Debug, Clone)]
pub struct PushLease {
    remote: String,
    source: String,
    destination: String,
    expected: String,
}

pub fn capture_lease(path: &Path) -> Result<PushLease, GitError> {
    let (remote, source, destination) = push_target(path)?;
    let repo = git2::Repository::open(path)?;
    let expected = if remote == "." {
        repo.refname_to_id(&destination).ok()
    } else {
        let configured = repo.find_remote(&remote)?;
        configured
            .refspecs()
            .filter(|spec| {
                spec.direction() == git2::Direction::Fetch && spec.src_matches(&destination)
            })
            .find_map(|spec| {
                spec.transform(&destination).ok().and_then(|name| {
                    name.as_str()
                        .ok()
                        .and_then(|name| repo.refname_to_id(name).ok())
                })
            })
    }
    .ok_or_else(|| {
        git2::Error::from_str(
            "Fetch the selected push remote and branch before pushing with lease.",
        )
    })?
    .to_string();
    Ok(PushLease {
        remote,
        source,
        destination,
        expected,
    })
}

pub fn run_push_with_lease(
    path: &Path,
    lease: &PushLease,
    cancel: &AtomicBool,
) -> Result<(), GitError> {
    let expectation = format!(
        "--force-with-lease={}:{}",
        lease.destination, lease.expected
    );
    let spec = format!("{}:{}", lease.source, lease.destination);
    super::process::git(
        path,
        &["push", &expectation, "--", &lease.remote, &spec],
        cancel,
    )
    .map(|_| ())
}
pub fn run_push(
    path: &Path,
    cancel: &Arc<AtomicBool>,
    _id: JobId,
    _tx: &Sender<JobProgress>,
) -> Result<(), GitError> {
    push(path, cancel)
}
fn push(path: &Path, cancel: &AtomicBool) -> Result<(), GitError> {
    let (remote, source, destination) = push_target(path)?;
    let spec = format!("{source}:{destination}");
    super::process::git(path, &["push", "--", &remote, &spec], cancel).map(|_| ())
}

fn push_target(path: &Path) -> Result<(String, String, String), GitError> {
    let repo = git2::Repository::open(path)?;
    let head = repo.head()?;
    if !head.is_branch() {
        return Err(git2::Error::from_str("Select a branch before pushing.").into());
    }
    let branch = head
        .shorthand()
        .ok()
        .ok_or_else(|| git2::Error::from_str("HEAD has no branch name."))?;
    let config = repo.config()?;
    let upstream_remote = config.get_string(&format!("branch.{branch}.remote")).ok();
    let remote = config
        .get_string(&format!("branch.{branch}.pushRemote"))
        .ok()
        .or_else(|| config.get_string("remote.pushDefault").ok())
        .or_else(|| upstream_remote.clone())
        .map(Ok)
        .unwrap_or_else(|| default_push_remote(&repo))?;
    if remote != "." {
        repo.find_remote(&remote)?;
    }
    let source = format!("refs/heads/{branch}");
    let destination = if upstream_remote.as_deref() == Some(&remote) {
        config
            .get_string(&format!("branch.{branch}.merge"))
            .unwrap_or_else(|_| source.clone())
    } else {
        source.clone()
    };
    if !destination.starts_with("refs/heads/") {
        return Err(git2::Error::from_str("Push destination is not a branch.").into());
    }
    Ok((remote, source, destination))
}

pub fn run_push_set_upstream(
    path: &Path,
    cancel: &Arc<AtomicBool>,
    _id: JobId,
    _tx: &Sender<JobProgress>,
) -> Result<(), GitError> {
    let (remote, _, _) = push_target(path)?;
    super::process::git(
        path,
        &["push", "--set-upstream", "--", &remote, "HEAD"],
        cancel,
    )
    .map(|_| ())
}

pub fn run_fetch_all(
    path: &Path,
    cancel: &Arc<AtomicBool>,
    job_id: JobId,
    tx: &Sender<JobProgress>,
) -> Result<(), GitError> {
    let prune = crate::config::Config::load().fetch_prune;
    let repo = git2::Repository::open(path)?;
    for remote in repo.remotes()?.iter().flatten().flatten() {
        let _ = tx.send(JobProgress::Update {
            id: job_id,
            message: format!("Fetching {remote}..."),
            pct: None,
        });
        fetch_remote(path, remote, cancel, prune)?;
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(GitError::Cancelled);
    }
    Ok(())
}
fn fetch_remote(
    path: &Path,
    remote: &str,
    cancel: &AtomicBool,
    prune: bool,
) -> Result<(), GitError> {
    let prune_arg = if prune { "--prune" } else { "--no-prune" };
    super::process::git(path, &["fetch", prune_arg, "--", remote], cancel).map(|_| ())
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
        let Some(local_oid) = branch.get().target() else {
            continue;
        };
        let Ok(upstream) = branch.upstream() else {
            continue;
        };
        let Some(upstream_oid) = upstream.get().target() else {
            continue;
        };
        if let Ok((a, b)) = repo.graph_ahead_behind(local_oid, upstream_oid) {
            map.insert(name, (a, b));
        }
    }
    map
}

pub fn compute_ahead_behind(repo: &git2::Repository) -> Option<(usize, usize)> {
    let head = repo.head().ok()?;
    let local_oid = head.target()?;
    let branch_name = head.shorthand().ok()?;
    let branch = repo
        .find_branch(branch_name, git2::BranchType::Local)
        .ok()?;
    let upstream = branch.upstream().ok()?;
    let upstream_oid = upstream.get().target()?;
    repo.graph_ahead_behind(local_oid, upstream_oid).ok()
}

/// Never guess between multiple remotes: repository settings make the choice explicit.
pub fn default_push_remote(repo: &git2::Repository) -> Result<String, GitError> {
    if let Ok(name) = repo.config()?.get_string("remote.pushDefault") {
        repo.find_remote(&name)?;
        return Ok(name);
    }
    let remotes = repo.remotes()?;
    if remotes.len() == 1 {
        return Ok(remotes
            .get(0)?
            .ok_or_else(|| git2::Error::from_str("Remote name is unavailable."))?
            .to_string());
    }
    Err(git2::Error::from_str(
        "Choose a default push remote in Repository Settings before publishing.",
    )
    .into())
}

pub fn provider_remote(repo: &git2::Repository) -> Result<String, String> {
    if let Ok(name) = repo
        .config()
        .and_then(|c| c.get_string("recorde.providerRemote"))
    {
        repo.find_remote(&name).map_err(|e| e.to_string())?;
        return Ok(name);
    }
    if repo.find_remote("origin").is_ok() {
        return Ok("origin".into());
    }
    let remotes = repo.remotes().map_err(|e| e.to_string())?;
    if remotes.len() == 1 {
        return Ok(remotes
            .get(0)
            .map_err(|e| e.to_string())?
            .ok_or("Remote name is unavailable.")?
            .into());
    }
    Err("Select a provider remote in Repository Settings.".into())
}

pub fn save_remote(
    path: &Path,
    name: &str,
    url: &str,
    rename_from: Option<&str>,
) -> Result<(), String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    if name.is_empty() || url.is_empty() {
        return Err("Remote name and URL are required.".into());
    }
    let run = |args: &[&str]| {
        super::process::git(path, args, &AtomicBool::new(false))
            .map(|_| ())
            .map_err(|e| e.user_message())
    };
    if let Some(old) = rename_from.filter(|old| *old != name) {
        run(&["remote", "rename", "--", old, name])?;
        let mut config = repo.config().map_err(|e| e.to_string())?;
        for key in remote_selection_keys(&config, old)? {
            if config.get_string(&key).ok().as_deref() == Some(old) {
                config.set_str(&key, name).map_err(|e| e.to_string())?;
            }
        }
    }
    if repo.find_remote(name).is_ok() {
        run(&["remote", "set-url", "--", name, url])
    } else {
        run(&["remote", "add", "--", name, url])
    }
}

pub fn remove_remote(path: &Path, name: &str) -> Result<(), String> {
    super::process::git(
        path,
        &["remote", "remove", "--", name],
        &AtomicBool::new(false),
    )
    .map_err(|e| e.user_message())?;
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let mut config = repo.config().map_err(|e| e.to_string())?;
    for key in remote_selection_keys(&config, name)? {
        if config.get_string(&key).ok().as_deref() == Some(name) {
            config.remove(&key).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn remote_selection_keys(config: &git2::Config, remote: &str) -> Result<Vec<String>, String> {
    let mut keys = vec![
        "remote.pushDefault".to_string(),
        "recorde.providerRemote".to_string(),
    ];
    let mut entries = config.entries(None).map_err(|e| e.to_string())?;
    while let Some(entry) = entries.next() {
        let entry = entry.map_err(|e| e.to_string())?;
        if let Some(name) = entry.name().ok() {
            if name.starts_with("branch.")
                && name.ends_with(".pushremote")
                && entry.value().ok() == Some(remote)
            {
                keys.push(name.into());
            }
        }
    }
    Ok(keys)
}
