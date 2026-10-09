use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::cell::RefCell;

use std::collections::HashSet;

#[cfg(test)]
#[path = "operations_tests.rs"]
mod tests;

use crate::git::{branches, capabilities, graph::CommitGraph, remote};
use crate::state::{BlameLine, ChangedFile, CommitDetail, ConflictedFile, DiffHunk, DiffLine, FileDiff, WorkingDirFile, StashEntry, SharedState};

/// Load a repository: build the commit graph, working-dir status, and stashes.
pub fn load_repo(path: PathBuf, state: SharedState) {
    let path = path.canonicalize().unwrap_or(path);
    let id = begin_repo_load(&path, &mut state.lock().unwrap());
    spawn_repo_load(path, id, state);
}

/// Refreshes may not supersede an explicit request to open another repository.
pub fn refresh_repo(state: SharedState) {
    let request = {
        let mut s = state.lock().unwrap();
        begin_repo_refresh(&mut s)
    };
    if let Some((path, id)) = request { spawn_repo_load(path, id, state); }
}

/// Extend the visible history without clearing the current graph, diff or forms.
pub fn load_more_history(state: SharedState) {
    let request = {
        let mut s = state.lock().unwrap();
        if s.repo_load_path.is_some() { return; }
        let Some(graph) = s.commit_graph.as_ref().filter(|g| g.has_more) else { return };
        let limit = graph.limit.saturating_add(2000);
        let previous = graph.clone();
        s.history_limit = limit;
        begin_repo_refresh(&mut s).map(|(path, id)| (path, id, limit, s.graph_filter.clone(), !s.show_all_branches, previous))
    };
    let Some((path, id, limit, filter, head_only, previous)) = request else { return };
    std::thread::spawn(move || {
        let result = git2::Repository::open(&path).and_then(|repo| CommitGraph::from_repo_page(&repo, limit, &filter, head_only, Some(&previous)));
        let mut s = state.lock().unwrap();
        if s.repo_load_id != id || s.repo_path.as_ref() != Some(&path) { return; }
        s.repo_load_path = None;
        match result {
            Ok(graph) => s.commit_graph = Some(std::sync::Arc::new(graph)),
            Err(error) => s.error = Some(error.to_string()),
        }
    });
}

fn begin_repo_load(path: &Path, state: &mut crate::state::AppState) -> u64 {
    state.repo_load_id += 1;
    state.repo_load_path = Some(path.to_path_buf());
    state.repo_load_id
}

/// Worktree notifications do not require rebuilding commit history.
pub fn refresh_worktree(state: SharedState) {
    let request = { let mut s = state.lock().unwrap(); begin_repo_refresh(&mut s) };
    let Some((path, id)) = request else { return };
    std::thread::spawn(move || {
        let result = (|| -> Result<_, git2::Error> {
            let repo = git2::Repository::open(&path)?;
            let (dirty, files) = collect_status(&repo)?;
            Ok((dirty, files, collect_conflicts(&repo), capabilities::detect(&path)))
        })();
        let selection = {
            let mut s = state.lock().unwrap();
            if s.repo_load_id != id || s.repo_path.as_ref() != Some(&path) { return; }
            s.repo_load_path = None;
            match result {
                Ok((dirty, files, conflicts, caps)) => {
                    s.dirty_count = dirty; s.working_dir_files = files;
                    s.conflicted_files = conflicts; apply_capabilities(&mut s, caps);
                }
                Err(e) => { s.error = Some(e.message().into()); return; }
            }
            (s.selected_file.clone(), s.diff_context)
        };
        if let (Some(file), context) = selection {
            match context {
                crate::state::DiffContext::StagedFile => load_workdir_diff(&file, true, state),
                crate::state::DiffContext::UnstagedFile => load_workdir_diff(&file, false, state),
                _ => {},
            }
        }
    });
}

fn begin_repo_refresh(state: &mut crate::state::AppState) -> Option<(PathBuf, u64)> {
    let path = state.repo_path.clone()?;
    if state.repo_load_path.as_ref().is_some_and(|pending| pending != &path) {
        return None;
    }
    let id = begin_repo_load(&path, state);
    Some((path, id))
}

fn spawn_repo_load(path: PathBuf, id: u64, state: SharedState) {
    let (limit, filter, head_only) = { let s = state.lock().unwrap();
        if s.repo_path.as_ref() == Some(&path) { (s.history_limit, s.graph_filter.clone(), !s.show_all_branches) }
        else { s.repo_views.get(&path).map(|v| (v.history_limit, v.graph_filter.clone(), !v.show_all_branches)).unwrap_or((2000, Default::default(), false)) }
    };
    std::thread::spawn(move || {
        let result = load_repo_inner_limit(&path, limit.max(2000), &filter, head_only);
        let bisect_active = crate::git::reflog::bisect_is_active(&path);
        let bisect_oid = if bisect_active { crate::git::reflog::bisect_current(&path) } else { None };
        let success = result.is_ok();
        let mut s = state.lock().unwrap();
        let switching = s.repo_path.as_ref() != Some(&path);
        apply_repo_result(&mut s, path.clone(), id, result, bisect_active, bisect_oid);
        let selection = if success && s.repo_load_id == id && s.repo_path.as_ref() == Some(&path)
            && (switching || s.diff_context != crate::state::DiffContext::CommitDiff) {
            s.selected_file.clone().map(|file| {
                s.diff_load_id += 1;
                s.current_diff = None;
                (RepoScope { path, epoch: s.repo_epoch }, s.diff_load_id, file, s.selected_commit.clone(), s.diff_context)
            })
        } else { None };
        let detail = if success && s.repo_load_id == id && s.selected_commit_detail.is_none() {
            s.selected_commit.clone()
        } else { None };
        drop(s);
        if let Some(oid) = detail { load_commit_detail(&oid, state.clone()); }
        if let Some((scope, request, file, commit, context)) = selection {
            let result = match (context, commit.as_deref()) {
                (crate::state::DiffContext::CommitDiff, Some(oid)) => compute_diff(&scope.path, oid, &file),
                (crate::state::DiffContext::StagedFile, _) => compute_workdir_diff(&scope.path, &file, true),
                (crate::state::DiffContext::UnstagedFile, _) => compute_workdir_diff(&scope.path, &file, false),
                _ => return,
            };
            let mut s = state.lock().unwrap();
            if scope.is_current(&s) && s.diff_load_id == request && s.selected_file.as_deref() == Some(&file)
                && s.diff_context == context && s.selected_commit == commit {
                match result { Ok(diff) => s.current_diff = Some(std::sync::Arc::new(diff)), Err(error) => s.error = Some(error.to_string()) }
            }
        }
    });
}

fn apply_repo_result(
    s: &mut crate::state::AppState, path: PathBuf, id: u64,
    result: Result<RepoInnerResult, git2::Error>, bisect_active: bool, bisect_oid: Option<String>,
) {
    if id != s.repo_load_id { return; }
    s.repo_load_path = None;
    match result {
            Ok((graph, head_branch, ahead_behind, branch_ahead_behind, dirty_count, files, stashes, conflicts, merged, caps, submodules, worktrees)) => {
                let switching = s.repo_path.as_ref() != Some(&path);
                if switching {
                    if let Some(previous) = s.repo_path.clone() {
                        s.repo_views.insert(previous, crate::state::RepoView::capture(s));
                    }
                }
                if s.repo_path.as_ref() != Some(&path) {
                    // Repository-owned selections, providers and recovery state
                    // cannot be carried over to an unrelated repository.
                    *s = crate::state::AppState {
                        repo_load_id: s.repo_load_id,
                        repo_views: std::mem::take(&mut s.repo_views),
                        repo_epoch: s.repo_epoch + 1,
                        console_log: std::mem::take(&mut s.console_log),
                        active_ops: std::mem::take(&mut s.active_ops),
                        local_jobs: std::mem::take(&mut s.local_jobs),
                        git_user_initials: std::mem::take(&mut s.git_user_initials),
                        show_all_branches: true,
                        focused_pane: s.focused_pane,
                        ..Default::default()
                    };
                }
                let n = graph.nodes.len();
                s.history_limit = graph.limit;
                s.console_log.push(format!("Opened {} ({} commits, branch: {})", path.display(), n, head_branch));
                s.repo_path = Some(path.clone());
                s.commit_graph = Some(std::sync::Arc::new(graph));
                if switching {
                    if let Some(view) = s.repo_views.get(&path).cloned() { view.restore(s); }
                }
                s.head_branch = Some(head_branch);
                s.ahead_behind = ahead_behind;
                s.branch_ahead_behind = branch_ahead_behind;
                s.dirty_count = dirty_count;
                s.working_dir_files = files;
                s.stashes = stashes;
                s.deleted_branches = super::branches::load_deleted(&path);
                s.conflicted_files = conflicts;
                s.merged_into_head = merged;
                apply_capabilities(s, caps);
                s.bisect_in_progress = bisect_active;
                s.bisect_current_oid = bisect_oid;
                s.submodules = submodules;
                s.worktrees = worktrees;
                // A refresh must not erase an operation failure before the user
                // has seen it (including failures which leave conflicts).
                s.remote_op = None;
            }
            Err(e) => {
                s.console_log.push(format!("error: {}", e.message()));
                s.error = Some(e.message().to_string());
            }
    }
}

fn apply_capabilities(s: &mut crate::state::AppState, caps: capabilities::RepoCapabilities) {
    if s.capabilities.as_ref().is_some_and(|old| old.provider_target != caps.provider_target || old.provider != caps.provider)
        || s.pr_source.as_ref().is_some_and(|(name, provider, url)| caps.provider_target.as_ref() != Some(&(name.clone(), url.clone())) || provider != &caps.provider) {
        s.invalidate_provider_data();
    }
    s.capabilities = Some(caps);
}

#[derive(Clone)]
struct RepoScope {
    path: PathBuf,
    epoch: u64,
}

impl RepoScope {
    fn capture(state: &SharedState) -> Option<Self> {
        let s = state.lock().unwrap();
        Some(Self { path: s.repo_path.clone()?, epoch: s.repo_epoch })
    }

    fn is_current(&self, s: &crate::state::AppState) -> bool {
        s.repo_path.as_ref() == Some(&self.path) && s.repo_epoch == self.epoch
    }
}

#[derive(Clone, Copy)]
enum ProviderRead { PullRequests, Details, Issues, Ci, Releases }

struct ProviderScope {
    path: PathBuf,
    epoch: u64,
    kind: ProviderRead,
    request: u64,
}

impl ProviderScope {
    fn begin(s: &mut crate::state::AppState, kind: ProviderRead) -> Option<Self> {
        let path = s.repo_path.clone()?;
        let index = kind as usize;
        s.provider_requests[index] = s.provider_requests[index].wrapping_add(1);
        match kind {
            ProviderRead::PullRequests => {
                s.pr_loading = true;
                s.provider_requests[ProviderRead::Details as usize] = s.provider_requests[ProviderRead::Details as usize].wrapping_add(1);
                s.pr_detail_loading = false;
            }
            ProviderRead::Details => s.pr_detail_loading = true,
            ProviderRead::Issues => s.issue_loading = true,
            ProviderRead::Ci => s.ci_loading = true,
            ProviderRead::Releases => s.releases_loading = true,
        }
        Some(Self { path, epoch: s.repo_epoch, kind, request: s.provider_requests[index] })
    }

    fn is_current(&self, s: &crate::state::AppState) -> bool {
        s.repo_path.as_ref() == Some(&self.path) && s.repo_epoch == self.epoch
            && s.provider_requests[self.kind as usize] == self.request
    }
}

type RepoInnerResult = (CommitGraph, String, Option<(usize, usize)>, std::collections::HashMap<String,(usize,usize)>, usize, Vec<WorkingDirFile>, Vec<StashEntry>, Vec<ConflictedFile>, HashSet<String>, crate::git::capabilities::RepoCapabilities, Vec<crate::state::SubmoduleInfo>, Vec<crate::state::WorktreeInfo>);

#[cfg(test)]
fn load_repo_inner(path: &Path) -> Result<RepoInnerResult, git2::Error> {
    load_repo_inner_limit(path, 2000, &Default::default(), false)
}

fn load_repo_inner_limit(path: &Path, limit: usize, filter: &crate::git::graph::GraphFilter, head_only: bool) -> Result<RepoInnerResult, git2::Error> {
    // Immutable pass: graph + head + status + ahead/behind
    let head_branch;
    let graph;
    let dirty_count;
    let status;
    let ahead_behind;
    let branch_ahead_behind;
    {
        let repo = git2::Repository::open(path)?;
        graph = CommitGraph::from_repo_filtered(&repo, limit, filter, head_only)?;
        head_branch = repo
            .head()
            .ok()
            .and_then(|h| h.shorthand().ok().map(str::to_owned))
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
            locked: w.locked, prunable: w.prunable,
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
        for entry in [conflict.our.as_ref(), conflict.their.as_ref(), conflict.ancestor.as_ref()].into_iter().flatten() {
            if let Ok(path) = std::str::from_utf8(&entry.path) {
                if seen.insert(path.to_string()) {
                    result.push(ConflictedFile {
                        path: path.to_string(),
                        version: super::conflicts::version(repo.workdir().unwrap_or(repo.path()), path).ok(),
                        ours_deleted: index.get_path(Path::new(path), 2).is_none(),
                        theirs_deleted: index.get_path(Path::new(path), 3).is_none(),
                    });
                }
            }
        }
    }
    result
}

fn collect_status(repo: &git2::Repository) -> Result<(usize, Vec<WorkingDirFile>), git2::Error> {
    if repo.config()?.get_bool("core.sparseCheckout").unwrap_or(false) {
        let output = super::process::git(repo.workdir().unwrap_or(repo.path()), &["status", "--porcelain=v1", "-z", "--untracked-files=all", "--ignore-submodules=all"], &std::sync::atomic::AtomicBool::new(false))
            .map_err(|e| git2::Error::from_str(&e.user_message()))?;
        if output.stdout.len() >= 1024 * 1024 { return Err(git2::Error::from_str("Sparse status exceeds the display limit. Narrow the checkout or commit changes.")); }
        let mut records = output.stdout.split(|byte| *byte == 0).filter(|r| !r.is_empty());
        let mut files = Vec::new();
        while let Some(record) = records.next() {
            if record.len() < 4 { return Err(git2::Error::from_str("Invalid Git status output.")); }
            let path = std::str::from_utf8(&record[3..]).map_err(|_| git2::Error::from_str("Non-UTF8 paths are unsupported."))?.to_string();
            let staged_status = match record[0] { b'A' | b'M' | b'D' | b'R' | b'C' | b'T' => Some(record[0] as char), _ => None };
            let unstaged_status = match record[1] { b'M' | b'D' | b'R' | b'T' | b'?' => Some(record[1] as char), _ => None };
            if record[..2].iter().any(|c| *c == b'R' || *c == b'C') { records.next(); }
            if staged_status.is_some() || unstaged_status.is_some() { files.push(WorkingDirFile { path, staged_status, unstaged_status }); }
        }
        return Ok((files.len(), files));
    }
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
    let context = if staged { crate::state::DiffContext::StagedFile } else { crate::state::DiffContext::UnstagedFile };
    let (scope, id) = {
        let mut s = state.lock().unwrap();
        let Some(path) = s.repo_path.clone() else { return };
        s.diff_load_id += 1;
        s.current_diff = None;
        s.diff_context = context;
        (RepoScope { path, epoch: s.repo_epoch }, s.diff_load_id)
    };

    std::thread::spawn(move || {
        let result = compute_workdir_diff(&scope.path, &fp, staged);
        let mut s = state.lock().unwrap();
        if scope.is_current(&s) && s.diff_load_id == id && s.diff_context == context
            && s.selected_file.as_deref() == Some(&fp) {
            match result {
                Ok(diff) => s.current_diff = Some(std::sync::Arc::new(diff)),
                Err(e) => s.error = Some(e.message().to_string()),
            }
        }
    });
}

pub(crate) fn compute_workdir_diff(repo_path: &Path, file_path: &str, staged: bool) -> Result<FileDiff, git2::Error> {
    let repo = git2::Repository::open(repo_path)?;
    let mut opts = git2::DiffOptions::new();
    opts.show_binary(true).disable_pathspec_match(true);
    if !staged { opts.pathspec(file_path).include_untracked(true).recurse_untracked_dirs(true).show_untracked_content(true); }

    let mut diff = if staged {
        // index vs HEAD (what would be committed)
        let head_tree = repo.head().ok()
            .and_then(|h| h.peel_to_commit().ok())
            .and_then(|c| c.tree().ok());
        repo.diff_tree_to_index(head_tree.as_ref(), None, Some(&mut opts))?
    } else {
        // workdir vs index (unstaged changes)
        repo.diff_index_to_workdir(None, Some(&mut opts))?
    };

    if staged { diff.find_similar(Some(git2::DiffFindOptions::new().renames(true)))?; }
    parse_diff(&repo, diff, "", file_path, !staged)
}

fn parse_diff(repo: &git2::Repository, diff: git2::Diff, commit_oid: &str, file_path: &str, workdir: bool) -> Result<FileDiff, git2::Error> {
    let requested = Path::new(file_path);
    let selected = diff.deltas().find(|d| d.new_file().path() == Some(requested))
        .or_else(|| diff.deltas().find(|d| d.old_file().path() == Some(requested)));
    let (old_path, new_path, old_mode, new_mode, is_rename) = selected.map(|d| {
        let old = if matches!(d.status(), git2::Delta::Added | git2::Delta::Untracked) { 0 } else { u32::from(d.old_file().mode()) };
        let new = if d.status() == git2::Delta::Deleted { 0 } else { u32::from(d.new_file().mode()) };
        (d.old_file().path().map(Path::to_path_buf), d.new_file().path().map(Path::to_path_buf), old, new, matches!(d.status(), git2::Delta::Renamed | git2::Delta::Copied))
    }).unwrap_or((None, None, 0, 0, false));
    let matches = |d: &git2::DiffDelta<'_>| d.old_file().path() == old_path.as_deref() && d.new_file().path() == new_path.as_deref();
    let mut raw_patch = Vec::new();
    diff.print(git2::DiffFormat::Patch, |delta, _, line| {
        if matches(&delta) {
            if matches!(line.origin(), '+' | '-' | ' ') { raw_patch.push(line.origin() as u8); }
            raw_patch.extend_from_slice(line.content());
        }
        true
    })?;
    let is_binary = Rc::new(RefCell::new(false));
    let is_binary_cb = Rc::clone(&is_binary);
    let hunks_rc: Rc<RefCell<Vec<(String, Vec<DiffLine>)>>> = Rc::new(RefCell::new(Vec::new()));
    let hunks_hunk = Rc::clone(&hunks_rc);
    let hunks_line = Rc::clone(&hunks_rc);

    diff.foreach(
        &mut |_, _| true,
        Some(&mut |delta, _| { if matches(&delta) { *is_binary_cb.borrow_mut() = true; } true }),
        Some(&mut |delta, hunk| {
            if !matches(&delta) { return true; }
            let header = std::str::from_utf8(hunk.header()).unwrap_or("").trim_end().to_string();
            hunks_hunk.borrow_mut().push((header, Vec::new()));
            true
        }),
        Some(&mut |delta, _, line| {
            if !matches(&delta) { return true; }
            let mut h = hunks_line.borrow_mut();
            if let Some(last) = h.last_mut() {
                // Never reconstruct an actionable patch from silently replaced
                // bytes. Unsupported text encodings must fail without mutation.
                let Ok(content) = std::str::from_utf8(line.content()) else { return false; };
                let content = content.to_string();
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

    let binary_preview = if binary {
        diff.deltas().find(|delta| matches(delta)).map(|delta| crate::git::binary_preview::snapshot(repo, &delta, workdir))
    } else { None };
    Ok(FileDiff {
        binary_preview,
        raw_patch, old_path: old_path.map(|p| p.to_string_lossy().into_owned()), is_rename,
        old_mode, new_mode,
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
    let Some(scope) = RepoScope::capture(&state) else { return };

    std::thread::spawn(move || {
        let result = compute_commit_detail(&scope.path, &oid_str);
        let mut s = state.lock().unwrap();
        // Only apply if this is still the selected commit.
        if scope.is_current(&s) && s.selected_commit.as_deref() == Some(&oid_str) {
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

    let mut diff = repo.diff_tree_to_tree(
        parent_tree.as_ref(),
        Some(&tree),
        Some(git2::DiffOptions::new().ignore_whitespace_change(false)),
    )?;

    diff.find_similar(Some(git2::DiffFindOptions::new().renames(true)))?;
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
    let (scope, id) = {
        let mut s = state.lock().unwrap();
        let Some(path) = s.repo_path.clone() else { return };
        s.diff_load_id += 1;
        s.current_diff = None;
        s.diff_context = crate::state::DiffContext::CommitDiff;
        (RepoScope { path, epoch: s.repo_epoch }, s.diff_load_id)
    };

    std::thread::spawn(move || {
        let result = compute_diff(&scope.path, &commit_oid, &file_path);
        let mut s = state.lock().unwrap();
        // Only apply if this file is still selected
        if scope.is_current(&s) && s.diff_load_id == id
            && s.diff_context == crate::state::DiffContext::CommitDiff
            && s.selected_commit.as_deref() == Some(&commit_oid)
            && s.selected_file.as_deref() == Some(&file_path) {
            match result {
                Ok(diff) => s.current_diff = Some(std::sync::Arc::new(diff)),
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
    opts.show_binary(true);

    let mut diff = repo.diff_tree_to_tree(
        parent_tree.as_ref(),
        Some(&tree),
        Some(&mut opts),
    )?;

    diff.find_similar(Some(git2::DiffFindOptions::new().renames(true)))?;
    parse_diff(&repo, diff, commit_oid, file_path, false)
}

// ── Phase 7 operations ────────────────────────────────────────────────────────

/// Load the OIDs of commits that touched `file_path` and store them in state.
pub fn load_file_history(file_path: &str, state: SharedState) {
    let fp = file_path.to_string();
    let Some(scope) = RepoScope::capture(&state) else { return };
    {
        let mut s = state.lock().unwrap();
        s.file_history_path = Some(fp.clone());
        s.file_history_oids.clear();
    }
    std::thread::spawn(move || {
        let oids = crate::git::history::file_history_oids(&scope.path, &fp);
        let mut s = state.lock().unwrap();
        if !scope.is_current(&s) || s.file_history_path.as_deref() != Some(&fp) { return; }
        s.file_history_oids = oids;
    });
}

/// Load blame annotations for `file_path` into state.
pub fn load_blame(file_path: &str, state: SharedState) {
    let fp = file_path.to_string();
    let Some(scope) = RepoScope::capture(&state) else { return };
    std::thread::spawn(move || {
        let lines: Vec<BlameLine> = crate::git::blame::blame_file(&scope.path, &fp);
        let mut s = state.lock().unwrap();
        if !scope.is_current(&s) || s.selected_file.as_deref() != Some(&fp) || !s.show_blame { return; }
        s.blame_lines = lines;
        s.show_blame = true;
    });
}

/// Load the list of files changed between two commits into state for the compare modal.
pub fn load_compare_files(base_oid: &str, compare_oid: &str, state: SharedState) {
    let base = base_oid.to_string();
    let cmp  = compare_oid.to_string();
    let Some(scope) = RepoScope::capture(&state) else { return };
    std::thread::spawn(move || {
        let files = compute_compare_files(&scope.path, &base, &cmp).unwrap_or_default();
        let mut s = state.lock().unwrap();
        if !scope.is_current(&s) { return; }
        if !matches!(&s.modal, crate::state::ModalKind::Compare { base_oid, compare_oid }
            if base_oid == &base && compare_oid == &cmp) { return; }
        s.compare_files = files;
        s.compare_diff = None;
        s.compare_selected_file = None;
    });
}

fn compute_compare_files(repo_path: &Path, base_oid: &str, cmp_oid: &str) -> Result<Vec<ChangedFile>, git2::Error> {
    let repo = git2::Repository::open(repo_path)?;
    let base_tree = repo.find_commit(git2::Oid::from_str(base_oid)?)?.tree()?;
    let cmp_tree  = repo.find_commit(git2::Oid::from_str(cmp_oid)?)?.tree()?;
    let mut diff = repo.diff_tree_to_tree(Some(&base_tree), Some(&cmp_tree), None)?;
    diff.find_similar(Some(git2::DiffFindOptions::new().renames(true)))?;
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
    let Some(scope) = RepoScope::capture(&state) else { return };
    {
        let mut s = state.lock().unwrap();
        s.compare_selected_file = Some(fp.clone());
        s.compare_diff = None;
    }
    std::thread::spawn(move || {
        let result = compute_compare_diff(&scope.path, &base, &cmp, &fp);
        let mut s = state.lock().unwrap();
        if !scope.is_current(&s) || s.compare_selected_file.as_deref() != Some(&fp) { return; }
        if !matches!(&s.modal, crate::state::ModalKind::Compare { base_oid, compare_oid }
            if base_oid == &base && compare_oid == &cmp) { return; }
        match result {
            Ok(diff) => {
                s.compare_diff = Some(diff);
            }
            Err(e) => s.error = Some(e.message().to_string()),
        }
    });
}

fn compute_compare_diff(repo_path: &Path, base_oid: &str, cmp_oid: &str, file_path: &str) -> Result<FileDiff, git2::Error> {
    let repo = git2::Repository::open(repo_path)?;
    let base_tree = repo.find_commit(git2::Oid::from_str(base_oid)?)?.tree()?;
    let cmp_tree  = repo.find_commit(git2::Oid::from_str(cmp_oid)?)?.tree()?;
    let mut opts = git2::DiffOptions::new();
    opts.show_binary(true);
    let mut diff = repo.diff_tree_to_tree(Some(&base_tree), Some(&cmp_tree), Some(&mut opts))?;
    diff.find_similar(Some(git2::DiffFindOptions::new().renames(true)))?;
    parse_diff(&repo, diff, cmp_oid, file_path, false)
}

// ── Phase 9: PR / MR loading ──────────────────────────────────────────────────

/// Load the PR/MR list for the current repo's provider in a background thread.
pub fn load_prs(state: crate::state::SharedState) {
    let Some(scope) = ProviderScope::begin(&mut state.lock().unwrap(), ProviderRead::PullRequests) else { return };
    use crate::providers::PrFilter;
    use crate::git::capabilities::ProviderType;
    let filter = state.lock().unwrap().pr_filter;

    let api_state = match filter {
        PrFilter::Closed => "closed",
        PrFilter::All    => "all",
        _                => "open",
    };


    std::thread::spawn(move || {
        let result = (|| {
            let opened = git2::Repository::open(&scope.path).map_err(|e| e.to_string())?;
            let remote = crate::git::remote::provider_remote(&opened)?;
            let remote_url = opened.find_remote(&remote).map_err(|e| e.to_string())?.url().ok().ok_or("Remote has no URL.")?.to_string();
            let (owner, repo, host, token, provider) = resolve_repo_info_for_remote(&scope.path, &remote)?;
            let prs = match provider {
                ProviderType::GitHub => crate::providers::github::list_prs(&host, &owner, &repo, &token, api_state),
                ProviderType::GitLab => crate::providers::gitlab::list_mrs(&host, &owner, &repo, &token, api_state),
                _ => Err("Provider is not supported for in-app PR listing.".into()),
            }?;
            Ok::<_, String>((prs, remote, provider, remote_url))
        })();
        let mut s = state.lock().unwrap();
        if !scope.is_current(&s) || s.pr_filter != filter { return; }
        s.pr_loading = false;
        match result {
            Ok((prs, remote, provider, remote_url)) => {
                let selected_number = s.pr_selected.and_then(|index| s.pr_list.get(index)).map(|pr| pr.number);
                s.pr_loaded = true; s.pr_source = Some((remote, provider, remote_url)); s.pr_list = prs; s.pr_error = None;
                s.pr_selected = selected_number.and_then(|number| s.pr_list.iter().position(|pr| pr.number == number));
            }
            Err(e)  => { s.pr_error = Some(e); }
        }
    });
}

// ── Phase 13 loaders ─────────────────────────────────────────────────────────

pub fn load_pr_details(state: SharedState) {
    let (number, remote, remote_url, scope, queue) = {
        let mut s = state.lock().unwrap();
        if s.pr_loading { return; }
        let Some(item) = s.pr_selected.and_then(|index| s.pr_list.get(index)) else { return };
        let number = item.number;
        let queue = item.queue_status.is_some();
        let Some((remote, _, remote_url)) = s.pr_source.clone() else { return };
        let Some(scope) = ProviderScope::begin(&mut s, ProviderRead::Details) else { return };
        (number, remote, remote_url, scope, queue)
    };
    std::thread::spawn(move || {
        let result = (|| {
            crate::git::reviews::verify_remote(&scope.path, &remote, &remote_url)?;
            let (owner, repo, host, token, provider) = resolve_repo_info_for_remote(&scope.path, &remote)?;
            let (mut item, mut warnings) = match provider {
                capabilities::ProviderType::GitHub => crate::providers::github::details(&host, &owner, &repo, number, &token),
                capabilities::ProviderType::GitLab => crate::providers::gitlab::details(&host, &owner, &repo, number, &token),
                _ => Err("Review details are not supported for this provider.".into()),
            }?;
            if queue {
                let status = match provider {
                    capabilities::ProviderType::GitHub => crate::providers::queue::github(&host, &owner, &repo, number, &item.head_oid, &token),
                    capabilities::ProviderType::GitLab => crate::providers::queue::gitlab(&host, &owner, &repo, number, &item.base_ref, &token),
                    _ => unreachable!(),
                };
                item.queue_status = Some(match status {
                    Ok(status) => status,
                    Err(error) => { warnings.push(format!("Queue status unavailable: {error}")); "Queue status unavailable".into() }
                });
            }
            Ok((item, warnings))
        })();
        let mut s = state.lock().unwrap();
        if !scope.is_current(&s) { return; }
        s.pr_detail_loading = false;
        let Some(index) = s.pr_selected.filter(|&index| s.pr_list.get(index).is_some_and(|item| item.number == number)) else { return };
        match result {
            Ok((item, warnings)) => { s.pr_list[index] = item; s.pr_error = if warnings.is_empty() { None } else { Some(warnings.join("\n")) }; }
            Err(error) => s.pr_error = Some(error),
        }
    });
}

/// Shared helper: resolve (owner, repo, host, token, provider) for the open repo.
fn resolve_repo_info(path: &Path)
    -> Result<(String, String, String, String, crate::git::capabilities::ProviderType), String>
{
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let remote = crate::git::remote::provider_remote(&repo)?;
    resolve_repo_info_for_remote(path, &remote)
}

pub(crate) fn resolve_repo_info_for_remote(path: &Path, remote_name: &str)
    -> Result<(String, String, String, String, crate::git::capabilities::ProviderType), String>
{
    use crate::auth::ProviderKind;
    use crate::git::capabilities::ProviderType;
    let repo = git2::Repository::open(path).map_err(|_| "Cannot open repository.".to_string())?;
    let remote = repo.find_remote(remote_name).map_err(|_| "Provider remote no longer exists.".to_string())?;
    let url = remote.url().ok().ok_or("Remote has no URL.")?;
    let endpoint = crate::auth::endpoint::parse(url, true)?;
    let (user, token) = crate::auth::store::lookup_for_api(url, path)?
        .ok_or("No token for the selected host/account. Add or update it in Settings > Accounts & SSH.")?;
    let config = crate::config::Config::try_load()?;
    let account = config.accounts.iter().find(|a| a.username == user
        && crate::auth::endpoint::normalize_host(&a.host).ok().as_deref() == Some(&endpoint.host));
    let provider = match account.map(|a| &a.provider) {
        Some(ProviderKind::GitHubCom | ProviderKind::GitHubEnterprise) => ProviderType::GitHub,
        Some(ProviderKind::GitLabCom | ProviderKind::GitLabSelf) => ProviderType::GitLab,
        None if endpoint.host == "github.com" => ProviderType::GitHub,
        None if endpoint.host == "gitlab.com" => ProviderType::GitLab,
        _ => return Err("Choose the provider type for this host in Settings > Accounts & SSH.".into()),
    };
    Ok((endpoint.namespace, endpoint.project, endpoint.host, token, provider))
}

pub fn load_issues(state: crate::state::SharedState) {
    let Some(scope) = ProviderScope::begin(&mut state.lock().unwrap(), ProviderRead::Issues) else { return };
    use crate::git::capabilities::ProviderType;
    use crate::providers::IssueFilter;

    let filter = state.lock().unwrap().issue_filter;
    std::thread::spawn(move || {
        let result = (|| {
            let (owner, repo, host, token, provider) = resolve_repo_info(&scope.path)?;

        let api_state = filter.api_state();
        match provider {
            ProviderType::GitHub => crate::providers::github::list_issues(&host, &owner, &repo, &token, api_state),
            ProviderType::GitLab => crate::providers::gitlab::list_issues(&host, &owner, &repo, &token, api_state),
            _ => Err(format!("Provider at {} not yet supported for issues.", host)),
        }
        })();
        let mut s = state.lock().unwrap();
        if !scope.is_current(&s) || s.issue_filter != filter { return; }
        s.issue_loading = false;
        match result {
            Ok(issues) => { s.issue_list = issues; s.issue_error = None; s.issue_selected = None; }
            Err(e)     => { s.issue_error = Some(e); }
        }
    });
}

pub fn load_ci_runs(state: crate::state::SharedState) {
    let Some(scope) = ProviderScope::begin(&mut state.lock().unwrap(), ProviderRead::Ci) else { return };
    use crate::git::capabilities::ProviderType;

    let branch = state.lock().unwrap().head_branch.clone().unwrap_or_default();
    std::thread::spawn(move || {
        let result = (|| {
            let (owner, repo, host, token, provider) = resolve_repo_info(&scope.path)?;

        match provider {
            ProviderType::GitHub => crate::providers::github::list_ci_runs(&host, &owner, &repo, &branch, &token),
            ProviderType::GitLab => crate::providers::gitlab::list_pipelines(&host, &owner, &repo, &branch, &token),
            _ => Err(format!("Provider at {} not yet supported for CI.", host)),
        }
        })();
        let mut s = state.lock().unwrap();
        if !scope.is_current(&s) { return; }
        s.ci_loading = false;
        if s.head_branch.as_deref().unwrap_or_default() != branch { return; }
        match result {
            Ok(runs) => { s.ci_runs = runs; s.ci_error = None; }
            Err(e)   => { s.ci_error = Some(e); }
        }
    });
}

pub fn load_releases(state: crate::state::SharedState) {
    let Some(scope) = ProviderScope::begin(&mut state.lock().unwrap(), ProviderRead::Releases) else { return };
    use crate::git::capabilities::ProviderType;

    std::thread::spawn(move || {
        let result = (|| {
            let (owner, repo, host, token, provider) = resolve_repo_info(&scope.path)?;

        match provider {
            ProviderType::GitHub => crate::providers::github::list_releases(&host, &owner, &repo, &token),
            ProviderType::GitLab => crate::providers::gitlab::list_releases(&host, &owner, &repo, &token),
            _ => Err(format!("Provider at {} not yet supported for releases.", host)),
        }
        })();
        let mut s = state.lock().unwrap();
        if !scope.is_current(&s) { return; }
        s.releases_loading = false;
        match result {
            Ok(rel) => { s.releases = rel; s.releases_error = None; }
            Err(e)  => { s.releases_error = Some(e); }
        }
    });
}
