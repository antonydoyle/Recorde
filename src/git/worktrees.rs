use std::path::Path;

#[derive(Debug, Clone)]
pub struct WorktreeInfo {
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
        .args(["worktree", "list", "--porcelain"])
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
    let mut result = Vec::new();

    // Blocks are separated by blank lines
    let mut wt_path: Option<String> = None;
    let mut head_oid: Option<String> = None;
    let mut branch: Option<String> = None;

    let flush = |wt_path: &mut Option<String>,
                 head_oid: &mut Option<String>,
                 branch: &mut Option<String>,
                 result: &mut Vec<WorktreeInfo>| {
        if let (Some(p), Some(h)) = (wt_path.take(), head_oid.take()) {
            let is_main = result.is_empty();
            result.push(WorktreeInfo {
                path: p,
                head_oid: h,
                branch: branch.take(),
                is_main,
            });
        } else {
            wt_path.take();
            head_oid.take();
            branch.take();
        }
    };

    for line in text.lines() {
        if line.is_empty() {
            flush(&mut wt_path, &mut head_oid, &mut branch, &mut result);
            continue;
        }

        if let Some(val) = line.strip_prefix("worktree ") {
            wt_path = Some(val.to_string());
        } else if let Some(val) = line.strip_prefix("HEAD ") {
            head_oid = Some(val.to_string());
        } else if let Some(val) = line.strip_prefix("branch ") {
            // Strip "refs/heads/" prefix if present
            let name = val.strip_prefix("refs/heads/").unwrap_or(val);
            branch = Some(name.to_string());
        } else if line == "detached" {
            branch = None;
        }
    }

    // Flush any trailing block (output may not end with a blank line)
    flush(&mut wt_path, &mut head_oid, &mut branch, &mut result);

    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
