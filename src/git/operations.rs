use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::cell::RefCell;

use crate::git::graph::CommitGraph;
use crate::state::{ChangedFile, CommitDetail, DiffHunk, DiffLine, FileDiff, FileStatus, SharedState};

/// Load a repository: build the commit graph and collect working-dir status.
pub fn load_repo(path: PathBuf, state: SharedState) {
    std::thread::spawn(move || {
        let result = load_repo_inner(&path);
        let mut s = state.lock().unwrap();
        match result {
            Ok((graph, head_branch, dirty_count, status)) => {
                s.repo_path = Some(path);
                s.commit_graph = Some(graph);
                s.head_branch = Some(head_branch);
                s.dirty_count = dirty_count;
                s.working_dir_status = status;
                s.error = None;
                // Clear stale selection when a new repo is loaded.
                s.selected_commit = None;
                s.selected_commit_detail = None;
            }
            Err(e) => s.error = Some(e.message().to_string()),
        }
    });
}

fn load_repo_inner(
    path: &Path,
) -> Result<(CommitGraph, String, usize, Vec<FileStatus>), git2::Error> {
    let repo = git2::Repository::open(path)?;

    let graph = CommitGraph::from_repo(&repo)?;

    let head_branch = repo
        .head()
        .ok()
        .and_then(|h| h.shorthand().map(str::to_owned))
        .unwrap_or_else(|| "HEAD".to_string());

    let (dirty_count, status) = collect_status(&repo)?;

    Ok((graph, head_branch, dirty_count, status))
}

fn collect_status(repo: &git2::Repository) -> Result<(usize, Vec<FileStatus>), git2::Error> {
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(false)
        .exclude_submodules(true);

    let statuses = repo.statuses(Some(&mut opts))?;
    let mut files = Vec::new();

    for entry in statuses.iter() {
        let path = entry.path().unwrap_or("").to_string();
        let s = entry.status();
        let ch = if s.contains(git2::Status::INDEX_NEW) { 'A' }
            else if s.contains(git2::Status::INDEX_MODIFIED) { 'M' }
            else if s.contains(git2::Status::INDEX_DELETED) { 'D' }
            else if s.contains(git2::Status::WT_MODIFIED) { 'm' }
            else if s.contains(git2::Status::WT_DELETED) { 'd' }
            else if s.contains(git2::Status::WT_NEW) { '?' }
            else { ' ' };
        files.push(FileStatus { path, index_status: ch });
    }

    let dirty = files.len();
    Ok((dirty, files))
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

    // Use Rc<RefCell<>> to share data across multiple closures in foreach.
    let is_binary = Rc::new(RefCell::new(false));
    let is_binary_bin = Rc::clone(&is_binary);

    // Collect (hunk_header, lines) pairs.
    let hunks_rc: Rc<RefCell<Vec<(String, Vec<DiffLine>)>>> = Rc::new(RefCell::new(Vec::new()));
    let hunks_hunk = Rc::clone(&hunks_rc);
    let hunks_line = Rc::clone(&hunks_rc);

    diff.foreach(
        &mut |_, _| true,
        Some(&mut |_, _| {
            *is_binary_bin.borrow_mut() = true;
            true
        }),
        Some(&mut |_, hunk| {
            let header = std::str::from_utf8(hunk.header())
                .unwrap_or("")
                .trim_end()
                .to_string();
            hunks_hunk.borrow_mut().push((header, Vec::new()));
            true
        }),
        Some(&mut |_, _, line| {
            let mut hunks = hunks_line.borrow_mut();
            if let Some(last) = hunks.last_mut() {
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

    let raw_hunks = Rc::try_unwrap(hunks_rc).unwrap().into_inner();
    let hunks: Vec<DiffHunk> = raw_hunks
        .into_iter()
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
