use std::io::Write as IoWrite;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::git::error::GitError;
use crate::state::DiffHunk;

// ── Patch generation ──────────────────────────────────────────────────────────

/// Build a full unified diff patch for one hunk (all lines selected).
pub fn hunk_patch(file_path: &str, hunk: &DiffHunk) -> String {
    let all: Vec<usize> = (0..hunk.lines.len()).collect();
    build_patch(file_path, hunk, &all)
}

/// Build a partial patch from selected line indices within a hunk.
///
/// Unselected change lines (+/-) are converted to context lines so that
/// `git apply` accepts the patch without touching those lines.
pub fn lines_patch(file_path: &str, hunk: &DiffHunk, selected: &[usize]) -> String {
    build_patch(file_path, hunk, selected)
}

fn build_patch(file_path: &str, hunk: &DiffHunk, selected: &[usize]) -> String {
    let (old_start, new_start) = parse_hunk_header(&hunk.header);
    let selected_set: std::collections::HashSet<usize> = selected.iter().copied().collect();

    let mut body = String::new();
    let mut old_count: u32 = 0;
    let mut new_count: u32 = 0;

    for (i, line) in hunk.lines.iter().enumerate() {
        match line.origin {
            ' ' | '\\' => {
                body.push(line.origin);
                body.push_str(&line.content);
                if !line.content.ends_with('\n') { body.push('\n'); }
                old_count += 1;
                new_count += 1;
            }
            '+' => {
                if selected_set.contains(&i) {
                    body.push('+');
                    body.push_str(&line.content);
                    if !line.content.ends_with('\n') { body.push('\n'); }
                    new_count += 1;
                } else {
                    // Not staging this addition — keep it as context on both sides
                    body.push(' ');
                    body.push_str(&line.content);
                    if !line.content.ends_with('\n') { body.push('\n'); }
                    old_count += 1;
                    new_count += 1;
                }
            }
            '-' => {
                if selected_set.contains(&i) {
                    body.push('-');
                    body.push_str(&line.content);
                    if !line.content.ends_with('\n') { body.push('\n'); }
                    old_count += 1;
                } else {
                    // Not staging this deletion — keep it as context on both sides
                    body.push(' ');
                    body.push_str(&line.content);
                    if !line.content.ends_with('\n') { body.push('\n'); }
                    old_count += 1;
                    new_count += 1;
                }
            }
            _ => {}
        }
    }

    format!(
        "diff --git a/{p} b/{p}\n--- a/{p}\n+++ b/{p}\n@@ -{old_start},{old_count} +{new_start},{new_count} @@\n{body}",
        p = file_path,
    )
}

/// Parse old_start and new_start from a hunk header like `@@ -10,5 +15,8 @@`.
pub fn parse_hunk_header(header: &str) -> (u32, u32) {
    let parts: Vec<&str> = header.split_whitespace().collect();
    let old_start = parts.get(1)
        .and_then(|s| s.strip_prefix('-'))
        .and_then(|s| s.split(',').next())
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(1);
    let new_start = parts.get(2)
        .and_then(|s| s.strip_prefix('+'))
        .and_then(|s| s.split(',').next())
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(1);
    (old_start, new_start)
}

// ── Patch application ─────────────────────────────────────────────────────────

/// Apply a patch to the git index (`git apply --cached`).
pub fn apply_to_index(repo_path: &Path, patch: &str) -> Result<(), GitError> {
    run_git_apply(repo_path, patch, &["--cached"])
}

/// Reverse-apply a patch from the git index (`git apply -R --cached`).
pub fn remove_from_index(repo_path: &Path, patch: &str) -> Result<(), GitError> {
    run_git_apply(repo_path, patch, &["-R", "--cached"])
}

/// Discard working-tree changes by reverse-applying a patch (`git apply -R`).
pub fn discard_from_workdir(repo_path: &Path, patch: &str) -> Result<(), GitError> {
    run_git_apply(repo_path, patch, &["-R"])
}

fn run_git_apply(repo_path: &Path, patch: &str, extra_args: &[&str]) -> Result<(), GitError> {
    let cmd_desc = format!("git apply {}", extra_args.join(" "));

    let mut child = Command::new("git")
        .arg("-C").arg(repo_path)
        .arg("apply")
        .args(extra_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| GitError::Command { cmd: cmd_desc.clone(), stderr: e.to_string() })?;

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(patch.as_bytes());
    }

    let out = child.wait_with_output()
        .map_err(|e| GitError::Command { cmd: cmd_desc.clone(), stderr: e.to_string() })?;

    if out.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(GitError::Command { cmd: cmd_desc, stderr })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{DiffLine, DiffHunk};

    fn make_hunk(header: &str, lines: &[(char, &str, Option<u32>, Option<u32>)]) -> DiffHunk {
        DiffHunk {
            header: header.to_string(),
            lines: lines.iter().map(|(origin, content, old_no, new_no)| DiffLine {
                origin: *origin,
                content: content.to_string(),
                old_lineno: *old_no,
                new_lineno: *new_no,
            }).collect(),
        }
    }

    #[test]
    fn hunk_patch_contains_header_and_lines() {
        let hunk = make_hunk("@@ -1,3 +1,4 @@", &[
            (' ', "context\n", Some(1), Some(1)),
            ('+', "added\n",   None,    Some(2)),
            (' ', "more\n",    Some(2), Some(3)),
        ]);
        let patch = hunk_patch("src/foo.rs", &hunk);
        assert!(patch.contains("diff --git a/src/foo.rs b/src/foo.rs"));
        assert!(patch.contains("--- a/src/foo.rs"));
        assert!(patch.contains("+added"));
        assert!(patch.contains(" context"));
    }

    #[test]
    fn lines_patch_converts_unselected_to_context() {
        let hunk = make_hunk("@@ -1,2 +1,3 @@", &[
            (' ', "ctx\n",    Some(1), Some(1)),
            ('+', "lineA\n",  None,    Some(2)),
            ('+', "lineB\n",  None,    Some(3)),
        ]);
        // Only select lineA (index 1), not lineB (index 2)
        let patch = lines_patch("file.txt", &hunk, &[1]);
        assert!(patch.contains("+lineA"));
        // lineB should appear as context
        assert!(patch.contains(" lineB"));
    }

    #[test]
    fn parse_hunk_header_extracts_starts() {
        let (old, new) = parse_hunk_header("@@ -10,5 +20,8 @@ fn foo()");
        assert_eq!(old, 10);
        assert_eq!(new, 20);
    }
}
