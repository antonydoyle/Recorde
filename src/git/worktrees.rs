use std::path::Path;

pub fn add(repo: &Path, destination: &Path, branch: &str) -> Result<(), String> {
    git2::Branch::name_is_valid(branch).map_err(|e| e.to_string()).and_then(|valid| if valid { Ok(()) } else { Err("Invalid branch name.".into()) })?;
    let destination = destination.to_str().ok_or("Worktree path is not valid UTF-8.")?;
    crate::git::merge::run_git_seq(repo, &["worktree", "add", "-b", branch, "--", destination])
        .map_err(|e| e.user_message())
}
pub fn set_locked(repo: &Path, destination: &Path, locked: bool) -> Result<(), String> {
    let destination = destination.to_str().ok_or("Worktree path is not valid UTF-8.")?;
    crate::git::merge::run_git_seq(repo, &["worktree", if locked { "lock" } else { "unlock" }, "--", destination])
        .map_err(|e| e.user_message())
}
pub fn remove(repo: &Path, destination: &Path) -> Result<(), String> {
    // No --force: native Git refuses dirty, locked and main worktrees.
    let destination = destination.to_str().ok_or("Worktree path is not valid UTF-8.")?;
    crate::git::merge::run_git_seq(repo, &["worktree", "remove", "--", destination])
        .map_err(|e| e.user_message())
}

#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    pub locked: bool,
    pub prunable: bool,
    pub path: String,
    pub head_oid: String,
    pub branch: Option<String>,
    pub is_main: bool,
}

/// Run `git worktree list --porcelain` and parse the output.
/// Returns an empty Vec on any error.
pub fn list(repo_path: &Path) -> Vec<WorktreeInfo> {
    let output = match std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["worktree", "list", "--porcelain", "-z"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };

    if !output.status.success() {
        return Vec::new();
    }

    let text = match String::from_utf8(output.stdout) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    parse_porcelain(&text)
}

fn parse_porcelain(text: &str) -> Vec<WorktreeInfo> {
    let fields: Vec<_> = if text.contains('\0') { text.split('\0').collect() } else { text.lines().collect() };
    let mut result = Vec::new();
    let mut current: Option<WorktreeInfo> = None;
    for field in fields {
        if let Some(path) = field.strip_prefix("worktree ") {
            if let Some(info) = current.take() { result.push(info); }
            current = Some(WorktreeInfo { path: path.into(), head_oid: String::new(), branch: None, is_main: result.is_empty(), locked: false, prunable: false });
        } else if let Some(info) = &mut current {
            if let Some(oid) = field.strip_prefix("HEAD ") { info.head_oid = oid.into(); }
            else if let Some(branch) = field.strip_prefix("branch ") { info.branch = Some(branch.strip_prefix("refs/heads/").unwrap_or(branch).into()); }
            else if field == "locked" || field.starts_with("locked ") { info.locked = true; }
            else if field == "prunable" || field.starts_with("prunable ") { info.prunable = true; }
        }
    }
    if let Some(info) = current { result.push(info); }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_lifecycle_refuses_locked_and_dirty_removal() {
        let repo = crate::git::test_support::TestRepo::new();
        repo.commit("file.txt", b"base\n");
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("linked space");
        add(repo.path(), &destination, "linked").unwrap();
        set_locked(repo.path(), &destination, true).unwrap();
        let entries = list(repo.path());
        assert!(entries.iter().any(|w| !w.is_main && w.locked && Path::new(&w.path).canonicalize().ok() == destination.canonicalize().ok()));
        assert!(remove(repo.path(), &destination).is_err());
        set_locked(repo.path(), &destination, false).unwrap();
        std::fs::write(destination.join("file.txt"), b"keep local edits\n").unwrap();
        assert!(remove(repo.path(), &destination).is_err());
        assert_eq!(std::fs::read(destination.join("file.txt")).unwrap(), b"keep local edits\n");
        std::fs::write(destination.join("file.txt"), b"base\n").unwrap();
        remove(repo.path(), &destination).unwrap();
        assert!(!destination.exists());
    }

    #[test]
    fn test_parse_porcelain_basic() {
        let input = "\
worktree /home/user/project
HEAD abc123def456abc123def456abc123def456abc123
branch refs/heads/main

worktree /home/user/project-feature
HEAD 111222333444abc123def456abc123def456abc1
branch refs/heads/feature/foo

";
        let worktrees = parse_porcelain(input);
        assert_eq!(worktrees.len(), 2);
        assert!(worktrees[0].is_main);
        assert!(!worktrees[1].is_main);
        assert_eq!(worktrees[0].branch.as_deref(), Some("main"));
        assert_eq!(worktrees[1].branch.as_deref(), Some("feature/foo"));
    }

    #[test]
    fn test_parse_porcelain_detached() {
        let input = "\
worktree /home/user/project
HEAD abc123def456abc123def456abc123def456abc123
branch refs/heads/main

worktree /home/user/project-detached
HEAD deadbeef1234deadbeef1234deadbeef12345678
detached

";
        let worktrees = parse_porcelain(input);
        assert_eq!(worktrees.len(), 2);
        assert!(worktrees[1].branch.is_none());
    }
    #[test]
    fn nul_porcelain_preserves_newlines_and_bare_main_identity() {
        let entries = parse_porcelain("worktree /main\0bare\0\0worktree /linked\nspace\0HEAD abcd\0detached\0locked reason\0prunable missing\0\0");
        assert_eq!(entries.len(), 2);
        assert!(entries[0].is_main);
        assert!(!entries[1].is_main);
        assert_eq!(entries[1].path, "/linked\nspace");
        assert!(entries[1].locked && entries[1].prunable);
    }

}
