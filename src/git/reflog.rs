use std::path::Path;

use crate::state::ReflogEntry;

/// Load reflog for HEAD (up to 200 entries).
pub fn load_reflog(repo_path: &Path) -> Vec<ReflogEntry> {
    let Ok(output) = std::process::Command::new("git")
        .arg("-C").arg(repo_path)
        .args(["reflog", "--format=%H\t%h\t%gd\t%gs\t%ct", "-n", "200"])
        .output()
    else {
        return Vec::new();
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(5, '\t');
            let oid       = parts.next()?.trim().to_string();
            let short_oid = parts.next()?.trim().to_string();
            let refname   = parts.next()?.trim().to_string();
            let message   = parts.next()?.trim().to_string();
            let timestamp = parts.next()?.trim().parse::<i64>().unwrap_or(0);
            Some(ReflogEntry { oid, short_oid, refname, message, timestamp })
        })
        .collect()
}

/// Recover a reflog entry by creating a branch at that OID.
pub fn recover(repo_path: &Path, oid: &str, branch_name: &str) -> Result<(), String> {
    let status = std::process::Command::new("git")
        .arg("-C").arg(repo_path)
        .args(["branch", branch_name, oid])
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("git branch {} {} failed", branch_name, oid)) }
}

/// Return true if a bisect is currently active in this repo.
pub fn bisect_is_active(repo_path: &Path) -> bool {
    git2::Repository::open(repo_path)
        .map(|r| r.path().join("BISECT_LOG").exists())
        .unwrap_or(false)
}

/// Read the current bisect step commit OID (from BISECT_HEAD), if any.
pub fn bisect_current(repo_path: &Path) -> Option<String> {
    let repo = git2::Repository::open(repo_path).ok()?;
    let oid = std::fs::read_to_string(repo.path().join("BISECT_HEAD")).ok()?;
    Some(oid.trim().to_string())
}
