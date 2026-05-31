use std::collections::HashSet;
use std::path::Path;

/// Returns the full commit OIDs of every commit that touched `file_path`,
/// following renames (`--follow`).
pub fn file_history_oids(repo_path: &Path, file_path: &str) -> HashSet<String> {
    let Ok(output) = std::process::Command::new("git")
        .arg("-C").arg(repo_path)
        .args(["log", "--follow", "--format=%H", "--", file_path])
        .output()
    else {
        return HashSet::new();
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}
