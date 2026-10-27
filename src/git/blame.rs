use std::path::Path;

use crate::state::BlameLine;

/// Return per-line blame annotations for `file_path` in the working tree.
/// Lines are sorted by line number and 1-indexed.
pub fn blame_file(repo_path: &Path, file_path: &str) -> Vec<BlameLine> {
    let repo = match git2::Repository::open(repo_path) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let workdir = match repo.workdir() {
        Some(w) => w,
        None => return Vec::new(),
    };

    let Ok(file) = super::safe_path::ScopedFile::new(workdir, file_path) else {
        return Vec::new();
    };
    let Some(content) = file
        .read(16 * 1024 * 1024)
        .ok()
        .flatten()
        .and_then(|bytes| String::from_utf8(bytes).ok())
    else {
        return Vec::new();
    };
    let file_lines: Vec<&str> = content.lines().collect();
    // Bound both versions before asking libgit2 to compute annotations.
    let head_blob = repo
        .head()
        .and_then(|head| head.peel_to_tree())
        .and_then(|tree| tree.get_path(Path::new(file_path)).map(|entry| entry.id()));
    if !head_blob
        .and_then(|oid| repo.odb()?.read_header(oid))
        .is_ok_and(|(size, kind)| kind == git2::ObjectType::Blob && size <= 16 * 1024 * 1024)
    {
        return Vec::new();
    }
    let committed = match repo.blame_file(Path::new(file_path), None) {
        Ok(blame) => blame,
        Err(_) => return Vec::new(),
    };
    let blame = match committed.blame_buffer(content.as_bytes()) {
        Ok(blame) => blame,
        Err(_) => return Vec::new(),
    };

    let mut result = Vec::new();
    for hunk in blame.iter() {
        let sig = hunk.final_signature();
        let full_oid = hunk.final_commit_id().to_string();
        let short_oid = full_oid[..7.min(full_oid.len())].to_string();
        let author = if hunk.final_commit_id().is_zero() {
            "Uncommitted".into()
        } else {
            sig.as_ref()
                .and_then(|sig| sig.name().ok())
                .unwrap_or("?")
                .to_string()
        };
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

#[cfg(test)]
mod tests {
    #[test]
    fn working_lines_keep_their_committed_annotations_after_insertion() {
        let fixture = crate::git::test_support::TestRepo::new();
        let first = fixture.commit("file", b"first\nsecond\n");
        fixture.write("file", b"new line\nfirst\nsecond\n");
        let lines = super::blame_file(fixture.path(), "file");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].content, "new line");
        assert_eq!(lines[0].author, "Uncommitted");
        assert_eq!(lines[1].content, "first");
        assert_eq!(lines[1].short_oid, first.to_string()[..7]);
        assert_eq!(lines[2].content, "second");
        assert_eq!(lines[2].lineno, 3);
    }
}
