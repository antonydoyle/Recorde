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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EffectivePull {
    #[default]
    Merge,
    MergeNoFf,
    Rebase,
    FastForwardOnly,
}

impl EffectivePull {
    pub fn label(self) -> &'static str {
        match self {
            Self::Merge => "Pull (merge)",
            Self::MergeNoFf => "Pull (merge, no fast-forward)",
            Self::Rebase => "Pull (rebase)",
            Self::FastForwardOnly => "Pull (fast-forward only)",
        }
    }

    fn strategy(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::MergeNoFf => "merge-no-ff",
            Self::Rebase => "rebase",
            Self::FastForwardOnly => "ff-only",
        }
    }
}

/// Branch configuration takes precedence even when it explicitly disables rebase.
fn effective_pull(
    branch_rebase: Option<&str>,
    rebase: Option<&str>,
    ff: Option<&str>,
) -> EffectivePull {
    match branch_rebase.or(rebase) {
        Some("merges" | "interactive" | "i") => return EffectivePull::Rebase,
        Some(value) => match git2::Config::parse_bool(value) {
            Ok(true) => return EffectivePull::Rebase,
            Ok(false) => {}
            Err(_) => return EffectivePull::Merge,
        },
        None => {}
    }
    match ff {
        Some("only") => EffectivePull::FastForwardOnly,
        Some(value) if git2::Config::parse_bool(value).ok() == Some(false) => {
            EffectivePull::MergeNoFf
        }
        _ => EffectivePull::Merge,
    }
}

/// Called by background capability refreshes and the pull worker, never during rendering.
pub fn configured_pull(
    repo: &git2::Repository,
    setting: &crate::config::PullStrategy,
) -> EffectivePull {
    use crate::config::PullStrategy;
    match setting {
        PullStrategy::Merge => EffectivePull::Merge,
        PullStrategy::Rebase => EffectivePull::Rebase,
        PullStrategy::FfOnly => EffectivePull::FastForwardOnly,
        PullStrategy::GitConfig => {
            let Ok(config) = repo.config() else {
                return EffectivePull::Merge;
            };
            let branch_rebase = repo
                .head()
                .ok()
                .filter(|head| head.is_branch())
                .and_then(|head| {
                    pull_config_value(
                        &config,
                        &format!("branch.{}.rebase", head.shorthand().ok()?),
                    )
                });
            let rebase = pull_config_value(&config, "pull.rebase");
            let ff = pull_config_value(&config, "pull.ff");
            effective_pull(branch_rebase.as_deref(), rebase.as_deref(), ff.as_deref())
        }
    }
}

fn pull_config_value(config: &git2::Config, key: &str) -> Option<String> {
    // get_bool also handles an implicit true (a key without an '=' value).
    config
        .get_bool(key)
        .map(|value| value.to_string())
        .or_else(|_| config.get_string(key))
        .ok()
}

// ── Public API (called from app.rs) ──────────────────────────────────────────

pub fn fetch_all(runner: &mut GitRunner, state: &SharedState) {
    let Some(path) = repo_path(state) else { return };
    runner.submit(JobKind::FetchAll, path);
}

pub fn pull_branch(runner: &mut GitRunner, state: &SharedState) {
    let Some(path) = repo_path(state) else { return };
    // Resolve configuration on the worker so repository reads stay off the UI thread.
    runner.submit(JobKind::PullBranch, path);
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

/// One throttle per job, shared across all of its remotes and subprocesses.
fn progress_callback(id: JobId, tx: &Sender<JobProgress>) -> impl FnMut(&str, Option<u8>) + '_ {
    let mut last = None;
    move |message, pct| {
        if last.is_none_or(|time: std::time::Instant| {
            time.elapsed() >= std::time::Duration::from_millis(100)
        }) {
            let _ = tx.send(JobProgress::Update {
                id,
                message: crate::auth::redact::text(message),
                pct,
            });
            last = Some(std::time::Instant::now());
        }
    }
}

pub fn run_pull_with_rebase(
    path: &Path,
    cancel: &Arc<AtomicBool>,
    id: JobId,
    tx: &Sender<JobProgress>,
) -> Result<(), GitError> {
    let mut progress = progress_callback(id, tx);
    super::process::with_cancellation(cancel.clone(), || {
        pull_progress(path, cancel, "rebase", &mut progress)
    })
}
pub fn run_pull_ff_only(
    path: &Path,
    cancel: &Arc<AtomicBool>,
    id: JobId,
    tx: &Sender<JobProgress>,
) -> Result<(), GitError> {
    pull_progress(path, cancel, "ff-only", &mut progress_callback(id, tx))
}
pub fn run_pull(
    path: &Path,
    cancel: &Arc<AtomicBool>,
    id: JobId,
    tx: &Sender<JobProgress>,
) -> Result<(), GitError> {
    run_configured_pull(
        path,
        cancel,
        &crate::config::Config::load().pull_strategy,
        &mut progress_callback(id, tx),
    )
}

fn run_configured_pull(
    path: &Path,
    cancel: &Arc<AtomicBool>,
    setting: &crate::config::PullStrategy,
    on_progress: &mut dyn FnMut(&str, Option<u8>),
) -> Result<(), GitError> {
    let repo = git2::Repository::open(path)?;
    let strategy = configured_pull(&repo, setting);
    super::process::with_cancellation(cancel.clone(), || {
        pull_progress(path, cancel, strategy.strategy(), on_progress)
    })
}
#[cfg(test)]
fn pull(path: &Path, cancel: &AtomicBool, strategy: &str) -> Result<(), GitError> {
    pull_progress(path, cancel, strategy, &mut |_, _| {})
}

fn pull_progress(
    path: &Path,
    cancel: &AtomicBool,
    strategy: &str,
    on_progress: &mut dyn FnMut(&str, Option<u8>),
) -> Result<(), GitError> {
    let (remote, _, _) = upstream(path)?;
    fetch_remote_progress(
        path,
        &remote,
        cancel,
        crate::config::Config::load().fetch_prune,
        on_progress,
    )?;
    if strategy == "rebase" {
        super::rebase::verify_merge_resolutions(path, "@{upstream}")
            .map_err(|e| git2::Error::from_str(&e))?;
    }
    let args: &[&str] = match strategy {
        "rebase" => &[
            "-c",
            "rebase.updateRefs=false",
            "rebase",
            "--rebase-merges",
            "--no-autostash",
            "@{upstream}",
        ],
        "ff-only" => &["merge", "--ff-only", "@{upstream}"],
        "merge-no-ff" => &["merge", "--no-ff", "--no-edit", "@{upstream}"],
        _ => &["merge", "--ff", "--no-edit", "@{upstream}"],
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
    push_with_lease_progress(path, lease, cancel, &mut |_, _| {})
}

pub(crate) fn run_push_with_lease_progress(
    path: &Path,
    lease: &PushLease,
    cancel: &AtomicBool,
    id: JobId,
    tx: &Sender<JobProgress>,
) -> Result<(), GitError> {
    push_with_lease_progress(path, lease, cancel, &mut progress_callback(id, tx))
}

fn push_with_lease_progress(
    path: &Path,
    lease: &PushLease,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(&str, Option<u8>),
) -> Result<(), GitError> {
    let expectation = format!(
        "--force-with-lease={}:{}",
        lease.destination, lease.expected
    );
    let spec = format!("{}:{}", lease.source, lease.destination);
    super::process::git_network(
        path,
        &["push", &expectation, "--", &lease.remote, &spec],
        cancel,
        on_progress,
    )
    .map(|_| ())
}
pub fn run_push(
    path: &Path,
    cancel: &Arc<AtomicBool>,
    id: JobId,
    tx: &Sender<JobProgress>,
) -> Result<(), GitError> {
    push_progress(path, cancel, &mut progress_callback(id, tx))
}
#[cfg(test)]
fn push(path: &Path, cancel: &AtomicBool) -> Result<(), GitError> {
    push_progress(path, cancel, &mut |_, _| {})
}

fn push_progress(
    path: &Path,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(&str, Option<u8>),
) -> Result<(), GitError> {
    let (remote, source, destination) = push_target(path)?;
    let spec = format!("{source}:{destination}");
    super::process::git_network(path, &["push", "--", &remote, &spec], cancel, on_progress)
        .map(|_| ())
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
    id: JobId,
    tx: &Sender<JobProgress>,
) -> Result<(), GitError> {
    let (remote, _, _) = push_target(path)?;
    super::process::git_network(
        path,
        &["push", "--set-upstream", "--", &remote, "HEAD"],
        cancel,
        &mut progress_callback(id, tx),
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
    let mut progress = progress_callback(job_id, tx);
    for remote in repo.remotes()?.iter().flatten().flatten() {
        progress(&format!("Fetching {remote}..."), None);
        fetch_remote_progress(path, remote, cancel, prune, &mut progress)?;
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(GitError::Cancelled);
    }
    Ok(())
}
#[cfg(test)]
fn fetch_remote(
    path: &Path,
    remote: &str,
    cancel: &AtomicBool,
    prune: bool,
) -> Result<(), GitError> {
    fetch_remote_progress(path, remote, cancel, prune, &mut |_, _| {})
}

fn fetch_remote_progress(
    path: &Path,
    remote: &str,
    cancel: &AtomicBool,
    prune: bool,
    on_progress: &mut dyn FnMut(&str, Option<u8>),
) -> Result<(), GitError> {
    let prune_arg = if prune { "--prune" } else { "--no-prune" };
    super::process::git_network(
        path,
        &["fetch", prune_arg, "--", remote],
        cancel,
        on_progress,
    )
    .map(|_| ())
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

#[cfg(test)]
mod pull_config_tests {
    use super::*;
    use crate::config::PullStrategy;
    use crate::git::test_support::TestRepo;

    #[test]
    fn effective_pull_obeys_rebase_precedence_and_ff_rules() {
        use EffectivePull::{FastForwardOnly, Merge, MergeNoFf, Rebase};
        let cases = [
            (None, None, None, Merge),
            (None, None, Some("only"), FastForwardOnly),
            (None, None, Some("false"), MergeNoFf),
            (None, None, Some("true"), Merge),
            (None, None, Some("unknown"), Merge),
            (None, Some("false"), Some("only"), FastForwardOnly),
            (None, Some("false"), Some("false"), MergeNoFf),
            (None, Some("false"), None, Merge),
            (None, Some("true"), Some("only"), Rebase),
            (None, Some("merges"), Some("false"), Rebase),
            (None, Some("interactive"), None, Rebase),
            (None, Some("i"), None, Rebase),
            (None, Some("unknown"), Some("only"), Merge),
            (Some("false"), Some("true"), Some("only"), FastForwardOnly),
            (Some("true"), Some("false"), Some("only"), Rebase),
            (Some("merges"), Some("false"), None, Rebase),
            (Some("interactive"), Some("false"), None, Rebase),
            (Some("i"), Some("false"), None, Rebase),
            (Some("unknown"), Some("true"), Some("only"), Merge),
        ];
        for (branch, rebase, ff, expected) in cases {
            assert_eq!(
                effective_pull(branch, rebase, ff),
                expected,
                "branch.rebase={branch:?}, pull.rebase={rebase:?}, pull.ff={ff:?}"
            );
        }
        for value in ["true", "yes", "on", "1", "2", "-1", "TRUE", "Yes", "ON"] {
            assert_eq!(effective_pull(None, Some(value), None), Rebase, "{value}");
            assert_eq!(effective_pull(None, None, Some(value)), Merge, "{value}");
        }
        for value in ["false", "no", "off", "0", "FALSE", "No", "OFF", ""] {
            assert_eq!(effective_pull(None, Some(value), None), Merge, "{value}");
            assert_eq!(
                effective_pull(None, None, Some(value)),
                MergeNoFf,
                "{value}"
            );
        }
    }

    #[test]
    fn configured_pull_reads_branch_override_and_respects_explicit_app_choices() {
        let fixture = TestRepo::new();
        fixture.commit("base.txt", b"base\n");
        let repo = fixture.open();
        let mut config = repo.config().unwrap();
        config.set_bool("pull.rebase", true).unwrap();
        config.set_bool("branch.main.rebase", false).unwrap();
        config.set_str("pull.ff", "only").unwrap();
        assert_eq!(
            configured_pull(&repo, &PullStrategy::GitConfig),
            EffectivePull::FastForwardOnly
        );
        assert_eq!(
            configured_pull(&repo, &PullStrategy::Merge),
            EffectivePull::Merge
        );
        assert_eq!(
            configured_pull(&repo, &PullStrategy::Rebase),
            EffectivePull::Rebase
        );
        assert_eq!(
            configured_pull(&repo, &PullStrategy::FfOnly),
            EffectivePull::FastForwardOnly
        );
        config.remove("branch.main.rebase").unwrap();
        assert_eq!(
            configured_pull(&repo, &PullStrategy::GitConfig),
            EffectivePull::Rebase
        );
    }

    #[test]
    fn git_config_pull_refuses_divergence_when_ff_only_is_configured() {
        let local = TestRepo::new();
        local.commit("base.txt", b"base\n");
        let remote_dir = tempfile::tempdir().unwrap();
        git2::Repository::init_bare(remote_dir.path()).unwrap();
        local.git_ok(&[
            "remote",
            "add",
            "upstream",
            remote_dir.path().to_str().unwrap(),
        ]);
        local.git_ok(&["push", "-u", "upstream", "main:trunk"]);
        let peer = TestRepo::new();
        peer.git_ok(&[
            "remote",
            "add",
            "upstream",
            remote_dir.path().to_str().unwrap(),
        ]);
        peer.git_ok(&["fetch", "upstream"]);
        peer.git_ok(&["checkout", "-B", "main", "upstream/trunk"]);
        peer.commit("peer.txt", b"peer\n");
        peer.git_ok(&["push", "upstream", "main:trunk"]);
        let before = local.commit("local.txt", b"local\n");
        let repo = local.open();
        let mut config = repo.config().unwrap();
        config.set_bool("branch.main.rebase", false).unwrap();
        config.set_str("pull.ff", "only").unwrap();
        // Exercise the same configured worker as pull_branch without changing app config.
        let result = run_configured_pull(
            local.path(),
            &Arc::new(AtomicBool::new(false)),
            &PullStrategy::GitConfig,
            &mut |_, _| {},
        );
        assert!(result.is_err());
        assert_eq!(local.head(), before);
        assert_eq!(
            repo.head()
                .unwrap()
                .peel_to_commit()
                .unwrap()
                .parent_count(),
            1
        );
        assert!(!repo.path().join("MERGE_HEAD").exists());
        assert!(!local.path().join("peer.txt").exists());
        assert_eq!(local.index_bytes("local.txt"), b"local\n");
    }

    #[test]
    fn git_config_no_ff_creates_a_merge_even_for_a_fast_forward() {
        let local = TestRepo::new();
        let base = local.commit("base.txt", b"base\n");
        let remote_dir = tempfile::tempdir().unwrap();
        git2::Repository::init_bare(remote_dir.path()).unwrap();
        local.git_ok(&[
            "remote",
            "add",
            "upstream",
            remote_dir.path().to_str().unwrap(),
        ]);
        local.git_ok(&["push", "-u", "upstream", "main:trunk"]);
        let peer = TestRepo::new();
        peer.git_ok(&[
            "remote",
            "add",
            "upstream",
            remote_dir.path().to_str().unwrap(),
        ]);
        peer.git_ok(&["fetch", "upstream"]);
        peer.git_ok(&["checkout", "-B", "main", "upstream/trunk"]);
        let upstream = peer.commit("peer.txt", b"peer\n");
        peer.git_ok(&["push", "upstream", "main:trunk"]);
        let repo = local.open();
        let mut config = repo.config().unwrap();
        config.set_bool("branch.main.rebase", false).unwrap();
        config.set_bool("pull.ff", false).unwrap();
        run_configured_pull(
            local.path(),
            &Arc::new(AtomicBool::new(false)),
            &PullStrategy::GitConfig,
            &mut |_, _| {},
        )
        .unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.parent_count(), 2);
        assert_eq!(head.parent_id(0).unwrap(), base);
        assert_eq!(head.parent_id(1).unwrap(), upstream);
    }
}
