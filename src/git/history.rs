use std::collections::HashSet;
use std::path::Path;

/// Returns the full commit OIDs of every commit that touched `file_path`,
/// following renames (`--follow`).
pub fn file_history_oids(repo_path: &Path, file_path: &str) -> HashSet<String> {
    let Ok(output) = super::process::git(
        repo_path,
        &[
            "--literal-pathspecs",
            "log",
            "--follow",
            "--format=%H",
            "--",
            file_path,
        ],
        &std::sync::atomic::AtomicBool::new(false),
    ) else {
        return HashSet::new();
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}
