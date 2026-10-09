use std::path::Path;

use crate::state::BlameLine;

/// Return per-line blame annotations for `file_path` in the working tree.
/// Lines are sorted by line number and 1-indexed.
pub fn blame_file(repo_path: &Path, file_path: &str) -> Vec<BlameLine> {
    let repo = match git2::Repository::open(repo_path) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let blame = match repo.blame_file(Path::new(file_path), None) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };

    let workdir = match repo.workdir() {
        Some(w) => w,
        None => return Vec::new(),
    };

    let Ok(path) = super::safe_path::worktree_file(workdir, file_path) else {
        return Vec::new();
    };
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let file_lines: Vec<&str> = content.lines().collect();

    let mut result = Vec::new();
    for hunk in blame.iter() {
        let sig = hunk.final_signature();
        let full_oid = hunk.final_commit_id().to_string();
        let short_oid = full_oid[..7.min(full_oid.len())].to_string();
        let author = sig
            .as_ref()
            .and_then(|sig| sig.name().ok())
            .unwrap_or("?")
            .to_string();
        let timestamp = sig.map(|sig| sig.when().seconds()).unwrap_or(0);
        let start_line = hunk.final_start_line(); // 1-indexed
        let lines_count = hunk.lines_in_hunk();

        for i in 0..lines_count {
            let lineno = start_line + i;
            let content = file_lines
                .get(lineno.saturating_sub(1))
                .copied()
                .unwrap_or("")
                .to_string();
            result.push(BlameLine {
                lineno,
                short_oid: short_oid.clone(),
                author: author.clone(),
                timestamp,
                content,
            });
        }
    }

    result.sort_by_key(|l| l.lineno);
    result
}
