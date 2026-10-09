use std::path::Path;

/// Returns true if `git lfs` is installed and callable (exit code 0).
pub fn is_lfs_installed() -> bool {
    let mut command = super::process::background_command("git");
    command.args(["lfs", "version"]);
    super::process::run(
        &mut command,
        &std::sync::atomic::AtomicBool::new(false),
        std::time::Duration::from_secs(10),
    )
    .is_ok_and(|out| out.status.success())
}

/// Returns true if the repo has a `.git/lfs` directory OR `.gitattributes`
/// contains at least one `filter=lfs` line.
pub fn repo_uses_lfs(repo_path: &Path) -> bool {
    // Check for .git/lfs directory
    let git_lfs_dir = repo_path.join(".git").join("lfs");
    if git_lfs_dir.exists() {
        return true;
    }

    // Check .gitattributes for filter=lfs
    let attrs = repo_path.join(".gitattributes");
    std::fs::read_to_string(&attrs)
        .map(|content| content.lines().any(|l| l.contains("filter=lfs")))
        .unwrap_or(false)
}

/// Parses `.gitattributes` and returns the glob pattern from each line that
/// contains `filter=lfs`.  For example:
///   `*.psd filter=lfs diff=lfs merge=lfs -text`  →  `"*.psd"`
pub fn tracked_patterns(repo_path: &Path) -> Vec<String> {
    let attrs = repo_path.join(".gitattributes");
    let Ok(content) = std::fs::read_to_string(&attrs) else {
        return Vec::new();
    };

    content
        .lines()
        .filter(|l| l.contains("filter=lfs"))
        .filter_map(|l| {
            // The pattern is the first whitespace-separated token on the line.
            // Skip comment lines (starting with '#').
            let trimmed = l.trim();
            if trimmed.starts_with('#') {
                return None;
            }
            trimmed.split_whitespace().next().map(|s| s.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_temp_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        dir
    }

    #[test]
    fn test_repo_uses_lfs_via_git_lfs_dir() {
        let dir = make_temp_repo();
        fs::create_dir_all(dir.path().join(".git").join("lfs")).unwrap();
        assert!(repo_uses_lfs(dir.path()));
    }

    #[test]
    fn test_repo_uses_lfs_via_gitattributes() {
        let dir = make_temp_repo();
        fs::write(
            dir.path().join(".gitattributes"),
            "*.psd filter=lfs diff=lfs merge=lfs -text\n",
        )
        .unwrap();
        assert!(repo_uses_lfs(dir.path()));
    }

    #[test]
    fn test_repo_uses_lfs_false_when_absent() {
        let dir = make_temp_repo();
        assert!(!repo_uses_lfs(dir.path()));
    }

    #[test]
    fn test_tracked_patterns_parses_correctly() {
        let dir = make_temp_repo();
        fs::write(
            dir.path().join(".gitattributes"),
            "*.psd filter=lfs diff=lfs merge=lfs -text\n\
             *.zip filter=lfs diff=lfs merge=lfs -text\n\
             # comment line filter=lfs\n\
             *.txt text=auto\n",
        )
        .unwrap();
        let patterns = tracked_patterns(dir.path());
        assert_eq!(patterns, vec!["*.psd", "*.zip"]);
    }

    #[test]
    fn test_tracked_patterns_empty_when_no_gitattributes() {
        let dir = make_temp_repo();
        let patterns = tracked_patterns(dir.path());
        assert!(patterns.is_empty());
    }
}
