use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::cell::RefCell;

use std::collections::HashSet;

use crate::git::{branches, capabilities, graph::CommitGraph, remote};
use crate::state::{BlameLine, ChangedFile, CommitDetail, ConflictedFile, DiffHunk, DiffLine, FileDiff, WorkingDirFile, StashEntry, SharedState};

/// Load a repository: build the commit graph, working-dir status, and stashes.
pub fn load_repo(path: PathBuf, state: SharedState) {
    std::thread::spawn(move || {
        let result = load_repo_inner(&path);
        let mut s = state.lock().unwrap();
        match result {
            Ok((graph, head_branch, ahead_behind, branch_ahead_behind, dirty_count, files, stashes, conflicts, merged, caps, submodules, worktrees)) => {
                let n = graph.nodes.len();
                s.console_log.push(format!("Opened {} ({} commits, branch: {})", path.display(), n, head_branch));
                let bisect_active = crate::git::reflog::bisect_is_active(&path);
                let bisect_oid = if bisect_active { crate::git::reflog::bisect_current(&path) } else { None };
                s.repo_path = Some(path);
                s.commit_graph = Some(graph);
                s.head_branch = Some(head_branch);
                s.ahead_behind = ahead_behind;
                s.branch_ahead_behind = branch_ahead_behind;
                s.dirty_count = dirty_count;
                s.working_dir_files = files;
                s.stashes = stashes;
                s.conflicted_files = conflicts;
                s.merged_into_head = merged;
                s.capabilities = Some(caps);
                s.bisect_in_progress = bisect_active;
                s.bisect_current_oid = bisect_oid;
                s.submodules = submodules;
                s.worktrees = worktrees;
                s.error = None;
                s.remote_op = None;
            }
            Err(e) => {
                s.console_log.push(format!("error: {}", e.message()));
                s.error = Some(e.message().to_string());
            }
        }
    });
}

type RepoInnerResult = (CommitGraph, String, Option<(usize, usize)>, std::collections::HashMap<String,(usize,usize)>, usize, Vec<WorkingDirFile>, Vec<StashEntry>, Vec<ConflictedFile>, HashSet<String>, crate::git::capabilities::RepoCapabilities, Vec<crate::state::SubmoduleInfo>, Vec<crate::state::WorktreeInfo>);

fn load_repo_inner(path: &Path) -> Result<RepoInnerResult, git2::Error> {
    // Immutable pass: graph + head + status + ahead/behind
    let head_branch;
    let graph;
    let dirty_count;
    let status;
    let ahead_behind;
    let branch_ahead_behind;
    {
        let repo = git2::Repository::open(path)?;
        graph = CommitGraph::from_repo(&repo)?;
        head_branch = repo
            .head()
            .ok()
            .and_then(|h| h.shorthand().map(str::to_owned))
            .unwrap_or_else(|| "HEAD".to_string());
        (dirty_count, status) = collect_status(&repo)?;
        ahead_behind = remote::compute_ahead_behind(&repo);
        branch_ahead_behind = remote::compute_all_ahead_behind(&repo);
    }

    // Mutable pass: stashes
    let stashes = {
        let mut repo = git2::Repository::open(path)?;
        branches::collect_stashes(&mut repo)
    };

    let (conflicts, merged) = {
        let repo = git2::Repository::open(path)?;
        (collect_conflicts(&repo), collect_merged_into_head(&repo))
    };

    let caps = capabilities::detect(path);

    // Convert git::submodules::SubmoduleInfo → state::SubmoduleInfo
    let submodules: Vec<crate::state::SubmoduleInfo> = crate::git::submodules::list(path)
        .into_iter()
        .map(|s| crate::state::SubmoduleInfo {
            name:        s.name,
            path:        s.path,
            url:         s.url,
            initialized: s.initialized,
        })
        .collect();

    // Convert git::worktrees::WorktreeInfo → state::WorktreeInfo
    let worktrees: Vec<crate::state::WorktreeInfo> = crate::git::worktrees::list(path)
        .into_iter()
        .map(|w| crate::state::WorktreeInfo {
            path:     w.path,
            head_oid: w.head_oid,
            branch:   w.branch,
            is_main:  w.is_main,
        })
        .collect();

    Ok((graph, head_branch, ahead_behind, branch_ahead_behind, dirty_count, status, stashes, conflicts, merged, caps, submodules, worktrees))
}

fn collect_merged_into_head(repo: &git2::Repository) -> HashSet<String> {
    let Ok(head) = repo.head() else { return HashSet::new() };
    let Some(head_oid) = head.target() else { return HashSet::new() };
    let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) else { return HashSet::new() };
    branches.flatten()
        .filter_map(|(b, _)| {
            let name = b.name().ok().flatten()?.to_string();
            let tip = b.get().target()?;
            if tip == head_oid { return None; } // skip HEAD branch itself
            let (ahead, _) = repo.graph_ahead_behind(tip, head_oid).ok()?;
            if ahead == 0 { Some(name) } else { None }
        })
        .collect()
}

fn collect_conflicts(repo: &git2::Repository) -> Vec<ConflictedFile> {
    let Ok(index) = repo.index() else { return Vec::new() };
    let Ok(conflicts) = index.conflicts() else { return Vec::new() };
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for conflict in conflicts.flatten() {
        // Prefer ours path, fall back to theirs, then ancestor
        let entry = conflict.our.as_ref()
            .or(conflict.their.as_ref())
            .or(conflict.ancestor.as_ref());
        if let Some(e) = entry {
            if let Ok(path) = std::str::from_utf8(&e.path) {
                if seen.insert(path.to_string()) {
                    result.push(ConflictedFile { path: path.to_string() });
                }
            }
        }
    }
    result
}

fn collect_status(repo: &git2::Repository) -> Result<(usize, Vec<WorkingDirFile>), git2::Error> {
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .exclude_submodules(true);

    let statuses = repo.statuses(Some(&mut opts))?;
    let mut files: Vec<WorkingDirFile> = Vec::new();

    for entry in statuses.iter() {
        let path = entry.path().unwrap_or("").to_string();
        let s = entry.status();

        let staged = if s.contains(git2::Status::INDEX_NEW) { Some('A') }
            else if s.contains(git2::Status::INDEX_MODIFIED) { Some('M') }
            else if s.contains(git2::Status::INDEX_DELETED) { Some('D') }
            else if s.contains(git2::Status::INDEX_RENAMED) { Some('R') }
            else { None };

        let unstaged = if s.contains(git2::Status::WT_NEW) { Some('?') }
            else if s.contains(git2::Status::WT_MODIFIED) { Some('M') }
            else if s.contains(git2::Status::WT_DELETED) { Some('D') }
            else { None };

        if staged.is_some() || unstaged.is_some() {
            files.push(WorkingDirFile { path, staged_status: staged, unstaged_status: unstaged });
        }
    }

    let dirty = files.len();
    Ok((dirty, files))
}

/// Load the diff for a working-directory file (not a commit diff).
/// `staged = true`  → diff between index (staged) and HEAD.
/// `staged = false` → diff between working tree and index.
pub fn load_workdir_diff(file_path: &str, staged: bool, state: SharedState) {
    let fp = file_path.to_string();
    let rp = state.lock().unwrap().repo_path.clone();
    let Some(path) = rp else { return };

    state.lock().unwrap().current_diff = None;

    std::thread::spawn(move || {
        let result = compute_workdir_diff(&path, &fp, staged);
        let mut s = state.lock().unwrap();
        if s.selected_file.as_deref() == Some(&fp) {
            match result {
                Ok(diff) => s.current_diff = Some(diff),
                Err(e) => s.error = Some(e.message().to_string()),
            }
        }
    });
}

fn compute_workdir_diff(repo_path: &Path, file_path: &str, staged: bool) -> Result<FileDiff, git2::Error> {
    let repo = git2::Repository::open(repo_path)?;
    let mut opts = git2::DiffOptions::new();
    opts.pathspec(file_path);

    let diff = if staged {
        // index vs HEAD (what would be committed)
        let head_tree = repo.head().ok()
            .and_then(|h| h.peel_to_commit().ok())
            .and_then(|c| c.tree().ok());
        repo.diff_tree_to_index(head_tree.as_ref(), None, Some(&mut opts))?
    } else {
        // workdir vs index (unstaged changes)
        repo.diff_index_to_workdir(None, Some(&mut opts))?
    };

    parse_diff(diff, "", file_path)
}

fn parse_diff(diff: git2::Diff, commit_oid: &str, file_path: &str) -> Result<FileDiff, git2::Error> {
    let is_binary = Rc::new(RefCell::new(false));
    let is_binary_cb = Rc::clone(&is_binary);
    let hunks_rc: Rc<RefCell<Vec<(String, Vec<DiffLine>)>>> = Rc::new(RefCell::new(Vec::new()));
    let hunks_hunk = Rc::clone(&hunks_rc);
    let hunks_line = Rc::clone(&hunks_rc);

    diff.foreach(
        &mut |_, _| true,
        Some(&mut |_, _| { *is_binary_cb.borrow_mut() = true; true }),
        Some(&mut |_, hunk| {
            let header = std::str::from_utf8(hunk.header()).unwrap_or("").trim_end().to_string();
            hunks_hunk.borrow_mut().push((header, Vec::new()));
            true
        }),
        Some(&mut |_, _, line| {
            let mut h = hunks_line.borrow_mut();
            if let Some(last) = h.last_mut() {
                let content = std::str::from_utf8(line.content())
                    .unwrap_or("")
                    .trim_end_matches(['\n', '\r'])
                    .to_string();
                last.1.push(DiffLine {
                    origin: line.origin(),
                    content,
                    old_lineno: line.old_lineno(),
                    new_lineno: line.new_lineno(),
                });
            }
            true
        }),
    )?;

    drop(hunks_hunk);
    drop(hunks_line);
    drop(is_binary_cb);
    let raw = Rc::try_unwrap(hunks_rc).unwrap().into_inner();
    let hunks: Vec<DiffHunk> = raw.into_iter()
        .map(|(header, lines)| DiffHunk { header, lines })
        .collect();

    let additions = hunks.iter().flat_map(|h| h.lines.iter()).filter(|l| l.origin == '+').count();
    let deletions = hunks.iter().flat_map(|h| h.lines.iter()).filter(|l| l.origin == '-').count();
    let binary = *is_binary.borrow();

    Ok(FileDiff {
        commit_oid: commit_oid.to_string(),
        file_path: file_path.to_string(),
        hunks,
        is_binary: binary,
        additions,
        deletions,
    })
}

/// Load the full detail for a single commit (changed files + message body).
/// Runs on a background thread and writes into `state.selected_commit_detail`.
pub fn load_commit_detail(oid_str: &str, state: SharedState) {
    let oid_str = oid_str.to_string();
    let repo_path = state.lock().unwrap().repo_path.clone();
    let Some(path) = repo_path else { return };

    std::thread::spawn(move || {
        let result = compute_commit_detail(&path, &oid_str);
        let mut s = state.lock().unwrap();
        // Only apply if this is still the selected commit.
        if s.selected_commit.as_deref() == Some(&oid_str) {
            match result {
                Ok(detail) => s.selected_commit_detail = Some(detail),
                Err(e) => s.error = Some(e.message().to_string()),
            }
        }
    });
}

fn compute_commit_detail(path: &Path, oid_str: &str) -> Result<CommitDetail, git2::Error> {
    let repo = git2::Repository::open(path)?;
    let oid = git2::Oid::from_str(oid_str)?;
    let commit = repo.find_commit(oid)?;

    let tree = commit.tree()?;
    let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());

    let diff = repo.diff_tree_to_tree(
        parent_tree.as_ref(),
        Some(&tree),
        Some(git2::DiffOptions::new().ignore_whitespace_change(false)),
    )?;

    let mut changed_files: Vec<ChangedFile> = Vec::new();

    diff.foreach(
        &mut |delta, _| {
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let status = match delta.status() {
                git2::Delta::Added => 'A',
                git2::Delta::Deleted => 'D',
                git2::Delta::Modified => 'M',
                git2::Delta::Renamed => 'R',
                git2::Delta::Copied => 'C',
                _ => '?',
            };
            changed_files.push(ChangedFile { path, status });
            true
        },
        None,
        None,
        None,
    )?;

    let full_msg = commit.message().unwrap_or("").to_string();
    let author = commit.author();
    let author_name = author.name().unwrap_or("Unknown").to_string();
    let author_email = author.email().unwrap_or("").to_string();
    let timestamp = commit.time().seconds();

    Ok(CommitDetail {
        oid: oid_str.to_string(),
        author: author_name,
        author_email,
        timestamp,
        message: full_msg,
        changed_files,
    })
}

/// Load the unified diff for a single file in a commit.
/// Runs on a background thread and writes into `state.current_diff`.
pub fn load_diff(commit_oid: &str, file_path: &str, state: SharedState) {
    let commit_oid = commit_oid.to_string();
    let file_path = file_path.to_string();
    let repo_path = state.lock().unwrap().repo_path.clone();
    let Some(path) = repo_path else { return };

    // Clear the previous diff immediately so the UI shows "Loading…"
    state.lock().unwrap().current_diff = None;

    std::thread::spawn(move || {
        let result = compute_diff(&path, &commit_oid, &file_path);
        let mut s = state.lock().unwrap();
        // Only apply if this file is still selected
        if s.selected_file.as_deref() == Some(&file_path) {
            match result {
                Ok(diff) => s.current_diff = Some(diff),
                Err(e) => s.error = Some(e.message().to_string()),
            }
        }
    });
}

fn compute_diff(repo_path: &Path, commit_oid: &str, file_path: &str) -> Result<FileDiff, git2::Error> {
    let repo = git2::Repository::open(repo_path)?;
    let oid = git2::Oid::from_str(commit_oid)?;
    let commit = repo.find_commit(oid)?;
    let tree = commit.tree()?;
    let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());

    let mut opts = git2::DiffOptions::new();
    opts.pathspec(file_path);

    let diff = repo.diff_tree_to_tree(
        parent_tree.as_ref(),
        Some(&tree),
        Some(&mut opts),
    )?;

    parse_diff(diff, commit_oid, file_path)
}

// ── Phase 7 operations ────────────────────────────────────────────────────────

/// Load the OIDs of commits that touched `file_path` and store them in state.
pub fn load_file_history(file_path: &str, state: SharedState) {
    let fp = file_path.to_string();
    let repo_path = state.lock().unwrap().repo_path.clone();
    let Some(path) = repo_path else { return };
    std::thread::spawn(move || {
        let oids = crate::git::history::file_history_oids(&path, &fp);
        let mut s = state.lock().unwrap();
        s.file_history_path = Some(fp);
        s.file_history_oids = oids;
    });
}

/// Load blame annotations for `file_path` into state.
pub fn load_blame(file_path: &str, state: SharedState) {
    let fp = file_path.to_string();
    let repo_path = state.lock().unwrap().repo_path.clone();
    let Some(path) = repo_path else { return };
    std::thread::spawn(move || {
        let lines: Vec<BlameLine> = crate::git::blame::blame_file(&path, &fp);
        let mut s = state.lock().unwrap();
        s.blame_lines = lines;
        s.show_blame = true;
    });
}

/// Load the list of files changed between two commits into state for the compare modal.
pub fn load_compare_files(base_oid: &str, compare_oid: &str, state: SharedState) {
    let base = base_oid.to_string();
    let cmp  = compare_oid.to_string();
    let repo_path = state.lock().unwrap().repo_path.clone();
    let Some(path) = repo_path else { return };
    std::thread::spawn(move || {
        let files = compute_compare_files(&path, &base, &cmp).unwrap_or_default();
        let mut s = state.lock().unwrap();
        s.compare_files = files;
        s.compare_diff = None;
        s.compare_selected_file = None;
    });
}

fn compute_compare_files(repo_path: &Path, base_oid: &str, cmp_oid: &str) -> Result<Vec<ChangedFile>, git2::Error> {
    let repo = git2::Repository::open(repo_path)?;
    let base_tree = repo.find_commit(git2::Oid::from_str(base_oid)?)?.tree()?;
    let cmp_tree  = repo.find_commit(git2::Oid::from_str(cmp_oid)?)?.tree()?;
    let diff = repo.diff_tree_to_tree(Some(&base_tree), Some(&cmp_tree), None)?;
    let mut files = Vec::new();
    diff.foreach(
        &mut |delta, _| {
            let fp = delta.new_file().path().or_else(|| delta.old_file().path())
                .map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            let status = match delta.status() {
                git2::Delta::Added => 'A', git2::Delta::Deleted => 'D',
                git2::Delta::Modified => 'M', git2::Delta::Renamed => 'R',
                git2::Delta::Copied => 'C', _ => '?',
            };
            files.push(ChangedFile { path: fp, status });
            true
        },
        None, None, None,
    )?;
    Ok(files)
}

/// Load diff for a single file between two commits into state.compare_diff.
pub fn load_compare_diff(base_oid: &str, compare_oid: &str, file_path: &str, state: SharedState) {
    let base = base_oid.to_string();
    let cmp  = compare_oid.to_string();
    let fp   = file_path.to_string();
    let repo_path = state.lock().unwrap().repo_path.clone();
    let Some(path) = repo_path else { return };
    std::thread::spawn(move || {
        match compute_compare_diff(&path, &base, &cmp, &fp) {
            Ok(diff) => {
                let mut s = state.lock().unwrap();
                s.compare_diff = Some(diff);
                s.compare_selected_file = Some(fp);
            }
            Err(e) => state.lock().unwrap().error = Some(e.message().to_string()),
        }
    });
}

fn compute_compare_diff(repo_path: &Path, base_oid: &str, cmp_oid: &str, file_path: &str) -> Result<FileDiff, git2::Error> {
    let repo = git2::Repository::open(repo_path)?;
    let base_tree = repo.find_commit(git2::Oid::from_str(base_oid)?)?.tree()?;
    let cmp_tree  = repo.find_commit(git2::Oid::from_str(cmp_oid)?)?.tree()?;
    let mut opts = git2::DiffOptions::new();
    opts.pathspec(file_path);
    let diff = repo.diff_tree_to_tree(Some(&base_tree), Some(&cmp_tree), Some(&mut opts))?;
    parse_diff(diff, cmp_oid, file_path)
}

// ── Phase 9: PR / MR loading ──────────────────────────────────────────────────

/// Load the PR/MR list for the current repo's provider in a background thread.
pub fn load_prs(state: crate::state::SharedState) {
    use crate::providers::{PrFilter, parse_remote};
    use crate::git::capabilities::ProviderType;

    let (repo_path, caps, filter) = {
        let s = state.lock().unwrap();
        (s.repo_path.clone(), s.capabilities.clone(), s.pr_filter)
    };

    let Some(path) = repo_path else { return };
    let Some(caps) = caps else { return };

    // Find the origin remote URL (explicit lets required for git2 lifetimes)
    let url = (|| -> Option<String> {
        let repo = git2::Repository::open(&path).ok()?;
        let remote = repo.find_remote("origin").ok()?;
        remote.url().map(str::to_string)
    })();
    let url = match url {
        Some(u) => u,
        None => {
            state.lock().unwrap().pr_error = Some("No 'origin' remote found.".into());
            return;
        }
    };

    let (owner, repo, host) = match parse_remote(&url) {
        Some(t) => t,
        None => {
            state.lock().unwrap().pr_error = Some("Could not parse remote URL.".into());
            return;
        }
    };

    let token = match crate::auth::store::lookup_for_url(&url).map(|(_, t)| t) {
        Some(t) => t,
        None => {
            state.lock().unwrap().pr_error = Some(
                format!("No token stored for {}. Add an account in Settings > Accounts & SSH.", host)
            );
            return;
        }
    };

    let api_state = match filter {
        PrFilter::Closed => "closed",
        PrFilter::All    => "all",
        _                => "open",
    };

    state.lock().unwrap().pr_loading = true;

    std::thread::spawn(move || {
        let result = match caps.provider {
            ProviderType::GitHub => crate::providers::github::list_prs(&host, &owner, &repo, &token, api_state),
            ProviderType::GitLab => crate::providers::gitlab::list_mrs(&host, &owner, &repo, &token, api_state),
            _ => Err(format!("Provider at {} is not yet supported for in-app PR listing.", host)),
        };
        let mut s = state.lock().unwrap();
        s.pr_loading = false;
        match result {
            Ok(prs) => { s.pr_list = prs; s.pr_error = None; s.pr_selected = None; }
            Err(e)  => { s.pr_error = Some(e); }
        }
    });
}

// ── Phase 13 loaders ─────────────────────────────────────────────────────────

/// Shared helper: resolve (owner, repo, host, token, provider) for the open repo.
fn resolve_repo_info(state: &crate::state::SharedState)
    -> Option<(String, String, String, String, crate::git::capabilities::ProviderType)>
{
    use crate::providers::parse_remote;
    let (path, caps) = {
        let s = state.lock().unwrap();
        (s.repo_path.clone()?, s.capabilities.clone()?)
    };
    let url = (|| -> Option<String> {
        let repo = git2::Repository::open(&path).ok()?;
        let remote = repo.find_remote("origin").ok()?;
        remote.url().map(str::to_string)
    })()?;
    let (owner, repo, host) = parse_remote(&url)?;
    let token = crate::auth::store::lookup_for_url(&url).map(|(_, t)| t)?;
    Some((owner, repo, host, token, caps.provider))
}

pub fn load_issues(state: crate::state::SharedState) {
    use crate::git::capabilities::ProviderType;
    use crate::providers::IssueFilter;

    let filter = state.lock().unwrap().issue_filter;
    let Some((owner, repo, host, token, provider)) = resolve_repo_info(&state) else {
        state.lock().unwrap().issue_error = Some("No token stored. Add an account in Settings > Accounts & SSH.".into());
        return;
    };

    state.lock().unwrap().issue_loading = true;
    std::thread::spawn(move || {
        let api_state = filter.api_state();
        let result = match provider {
            ProviderType::GitHub => crate::providers::github::list_issues(&host, &owner, &repo, &token, api_state),
            ProviderType::GitLab => crate::providers::gitlab::list_issues(&host, &owner, &repo, &token, api_state),
            _ => Err(format!("Provider at {} not yet supported for issues.", host)),
        };
        let mut s = state.lock().unwrap();
        s.issue_loading = false;
        match result {
            Ok(issues) => { s.issue_list = issues; s.issue_error = None; s.issue_selected = None; }
            Err(e)     => { s.issue_error = Some(e); }
        }
    });
}

pub fn load_ci_runs(state: crate::state::SharedState) {
    use crate::git::capabilities::ProviderType;

    let branch = state.lock().unwrap().head_branch.clone().unwrap_or_default();
    let Some((owner, repo, host, token, provider)) = resolve_repo_info(&state) else {
        state.lock().unwrap().ci_error = Some("No token stored. Add an account in Settings > Accounts & SSH.".into());
        return;
    };

    state.lock().unwrap().ci_loading = true;
    std::thread::spawn(move || {
        let result = match provider {
            ProviderType::GitHub => crate::providers::github::list_ci_runs(&host, &owner, &repo, &branch, &token),
            ProviderType::GitLab => crate::providers::gitlab::list_pipelines(&host, &owner, &repo, &branch, &token),
            _ => Err(format!("Provider at {} not yet supported for CI.", host)),
        };
        let mut s = state.lock().unwrap();
        s.ci_loading = false;
        match result {
            Ok(runs) => { s.ci_runs = runs; s.ci_error = None; }
            Err(e)   => { s.ci_error = Some(e); }
        }
    });
}

pub fn load_releases(state: crate::state::SharedState) {
    use crate::git::capabilities::ProviderType;

    let Some((owner, repo, host, token, provider)) = resolve_repo_info(&state) else {
        state.lock().unwrap().releases_error = Some("No token stored. Add an account in Settings > Accounts & SSH.".into());
        return;
    };

    state.lock().unwrap().releases_loading = true;
    std::thread::spawn(move || {
        let result = match provider {
            ProviderType::GitHub => crate::providers::github::list_releases(&host, &owner, &repo, &token),
            ProviderType::GitLab => crate::providers::gitlab::list_releases(&host, &owner, &repo, &token),
            _ => Err(format!("Provider at {} not yet supported for releases.", host)),
        };
        let mut s = state.lock().unwrap();
        s.releases_loading = false;
        match result {
            Ok(rel) => { s.releases = rel; s.releases_error = None; }
            Err(e)  => { s.releases_error = Some(e); }
        }
    });
}
