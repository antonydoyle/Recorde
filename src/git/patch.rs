use std::io::Write as IoWrite;
use std::path::Path;

use crate::git::error::GitError;
use crate::state::DiffHunk;

#[cfg(test)]
#[path = "patch_tests.rs"]
mod regression_tests;

// ── Patch generation ──────────────────────────────────────────────────────────

/// Build a full unified diff patch for one hunk (all lines selected).
pub fn hunk_patch(file_path: &str, hunk: &DiffHunk) -> String {
    let all: Vec<usize> = (0..hunk.lines.len()).collect();
    build_patch(file_path, hunk, &all, false)
}

/// Build a partial patch from selected line indices within a hunk.
///
/// Unselected additions do not exist in the index and must be omitted.
/// Unselected deletions remain in the index and become context.
pub fn lines_patch(file_path: &str, hunk: &DiffHunk, selected: &[usize]) -> String {
    build_patch(file_path, hunk, selected, false)
}

/// Build a patch to reverse-apply from the index. Its source is the new side:
/// retain unselected additions as context and omit unselected deletions.
pub fn unstage_lines_patch(file_path: &str, hunk: &DiffHunk, selected: &[usize]) -> String {
    build_patch(file_path, hunk, selected, true)
}

fn build_patch(file_path: &str, hunk: &DiffHunk, selected: &[usize], reverse: bool) -> String {
    build_patch_with_modes(file_path, hunk, selected, reverse, None)
}

/// File creation/deletion headers depend on the selected patch's effective sides.
pub fn selection_patch(
    diff: &crate::state::FileDiff,
    hunk: &DiffHunk,
    selected: &[usize],
    reverse: bool,
) -> String {
    build_patch_with_modes(
        &diff.file_path,
        hunk,
        selected,
        reverse,
        Some((diff.old_mode, diff.new_mode)),
    )
}

/// One atomic file patch spanning any number of selected changes/hunks.
pub fn multi_selection_patch(
    diff: &crate::state::FileDiff,
    selected: &[(usize, usize)],
    reverse: bool,
) -> Result<String, String> {
    if diff.is_binary || diff.is_rename {
        return Err("Use a whole-file operation for binary or renamed files.".into());
    }
    let mut groups = std::collections::BTreeMap::<usize, std::collections::BTreeSet<usize>>::new();
    for &(hunk, line) in selected {
        if !diff
            .hunks
            .get(hunk)
            .and_then(|h| h.lines.get(line))
            .is_some_and(|l| matches!(l.origin, '+' | '-'))
        {
            return Err(
                "Selection contains an invalid or unchanged line. Refresh the diff.".into(),
            );
        }
        groups.entry(hunk).or_default().insert(line);
    }
    if groups.is_empty() {
        return Err("Select at least one changed line.".into());
    }
    let mut result = String::new();
    let mut shift = 0i64;
    for (hunk, lines) in groups {
        let patch = selection_patch(
            diff,
            &diff.hunks[hunk],
            &lines.into_iter().collect::<Vec<_>>(),
            reverse,
        );
        let header = patch.find("\n@@ ").ok_or("Missing hunk header.")? + 1;
        let body = patch[header..].find('\n').ok_or("Invalid hunk header.")? + header;
        let parts: Vec<_> = patch[header..body].split_whitespace().collect();
        let range = |part: &str| -> Result<(i64, i64), String> {
            let (start, count) = part[1..].split_once(',').ok_or("Invalid hunk range.")?;
            Ok((
                start.parse().map_err(|_| "Invalid hunk start.")?,
                count.parse().map_err(|_| "Invalid hunk length.")?,
            ))
        };
        let (mut old, old_count) = range(parts[1])?;
        let (mut new, new_count) = range(parts[2])?;
        if reverse {
            old = new - i64::from(new_count > 0) + shift + i64::from(old_count > 0);
            shift += old_count - new_count;
        } else {
            new = old - i64::from(old_count > 0) + shift + i64::from(new_count > 0);
            shift += new_count - old_count;
        }
        if old < 0 || new < 0 {
            return Err("Invalid selected patch coordinates.".into());
        }
        if result.is_empty() {
            result.push_str(&patch[..header]);
        }
        result.push_str(&format!("@@ -{old},{old_count} +{new},{new_count} @@"));
        result.push_str(&patch[body..]);
    }
    Ok(result)
}

fn build_patch_with_modes(
    file_path: &str,
    hunk: &DiffHunk,
    selected: &[usize],
    reverse: bool,
    modes: Option<(u32, u32)>,
) -> String {
    let (mut old_start, mut new_start) = parse_hunk_header(&hunk.header);
    let selected_set: std::collections::HashSet<usize> = selected.iter().copied().collect();

    let mut body = String::new();
    let mut old_count: u32 = 0;
    let mut new_count: u32 = 0;

    for (i, line) in hunk.lines.iter().enumerate() {
        let origin = match line.origin {
            ' ' => ' ',
            '+' | '-' if selected_set.contains(&i) => line.origin,
            '+' if reverse => ' ',
            '-' if !reverse => ' ',
            // EOF marker callbacks are represented by the preceding raw line's
            // missing newline. They must not count toward either hunk length.
            _ => continue,
        };
        body.push(origin);
        body.push_str(&line.content);
        if !line.content.ends_with('\n') {
            body.push_str("\n\\ No newline at end of file\n");
        }
        if origin != '+' {
            old_count += 1;
        }
        if origin != '-' {
            new_count += 1;
        }
    }

    let old_path = quote_path(&format!("a/{}", file_path));
    let new_path = quote_path(&format!("b/{}", file_path));
    let mut metadata = String::new();
    let mut old_label = old_path.clone();
    let mut new_label = new_path.clone();
    if let Some((old_mode, new_mode)) = modes {
        if old_mode == 0 && old_count == 0 {
            metadata = format!("new file mode {new_mode:06o}\n");
            old_label = "/dev/null".into();
            old_start = 0;
        } else if old_mode == 0 {
            old_start = old_start.max(1);
        }
        if new_mode == 0 && new_count == 0 {
            metadata = format!("deleted file mode {old_mode:06o}\n");
            new_label = "/dev/null".into();
            new_start = 0;
        } else if new_mode == 0 {
            new_start = new_start.max(1);
        }
    }
    format!(
        "diff --git {old_path} {new_path}\n{metadata}--- {old_label}\n+++ {new_label}\n@@ -{old_start},{old_count} +{new_start},{new_count} @@\n{body}",
    )
}

// Git's C-style quoting keeps tabs, newlines and backslashes out of patch headers.
fn quote_path(path: &str) -> String {
    if !path
        .bytes()
        .any(|b| b <= b' ' || b == b'"' || b == b'\\' || b >= 0x7f)
    {
        return path.to_string();
    }
    let mut quoted = String::from("\"");
    for byte in path.bytes() {
        match byte {
            b'"' => quoted.push_str("\\\""),
            b'\\' => quoted.push_str("\\\\"),
            0x20..=0x7e => quoted.push(byte as char),
            _ => quoted.push_str(&format!("\\{:03o}", byte)),
        }
    }
    quoted.push('"');
    quoted
}

/// Parse old_start and new_start from a hunk header like `@@ -10,5 +15,8 @@`.
pub fn parse_hunk_header(header: &str) -> (u32, u32) {
    let parts: Vec<&str> = header.split_whitespace().collect();
    let old_start = parts
        .get(1)
        .and_then(|s| s.strip_prefix('-'))
        .and_then(|s| s.split(',').next())
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(1);
    let new_start = parts
        .get(2)
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
    let mut file = tempfile::NamedTempFile::new().map_err(|e| GitError::Command {
        cmd: "Create patch".into(),
        stderr: e.to_string(),
    })?;
    file.write_all(patch.as_bytes())
        .map_err(|e| GitError::Command {
            cmd: "Write patch".into(),
            stderr: e.to_string(),
        })?;
    let mut command = super::process::command(repo_path);
    command
        .arg("apply")
        .args(extra_args)
        .arg("--")
        .arg(file.path());
    let output = super::process::run(
        &mut command,
        &std::sync::atomic::AtomicBool::new(false),
        std::time::Duration::from_secs(600),
    )?;
    if output.status.success() {
        Ok(())
    } else {
        Err(GitError::Command {
            cmd: "git apply".into(),
            stderr: crate::auth::redact::text(String::from_utf8_lossy(&output.stderr).trim()),
        })
    }
}

// ── File I/O ──────────────────────────────────────────────────────────────────

/// Write the complete unified diff for all hunks in `diff` to a `.patch` file.
pub fn export_patch(diff: &crate::state::FileDiff, dest: &Path) -> Result<(), String> {
    if diff.raw_patch.is_empty() {
        return Err("No patch data is available for this file.".into());
    }
    std::fs::write(dest, &diff.raw_patch).map_err(|e| e.to_string())
}

/// Apply a patch file to the git index: `git apply --cached <path>`.
pub fn import_patch_file(repo_path: &Path, patch_path: &Path) -> Result<(), GitError> {
    let mut command = super::process::command(repo_path);
    command.args(["apply", "--cached", "--"]).arg(patch_path);
    let out = super::process::run(
        &mut command,
        &std::sync::atomic::AtomicBool::new(false),
        std::time::Duration::from_secs(600),
    )?;
    if out.status.success() {
        Ok(())
    } else {
        Err(GitError::Command {
            cmd: "git apply --cached".into(),
            stderr: crate::auth::redact::text(String::from_utf8_lossy(&out.stderr).trim()),
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{DiffHunk, DiffLine};

    fn make_hunk(header: &str, lines: &[(char, &str, Option<u32>, Option<u32>)]) -> DiffHunk {
        DiffHunk {
            header: header.to_string(),
            lines: lines
                .iter()
                .map(|(origin, content, old_no, new_no)| DiffLine {
                    origin: *origin,
                    content: content.to_string(),
                    old_lineno: *old_no,
                    new_lineno: *new_no,
                })
                .collect(),
        }
    }

    #[test]
    fn hunk_patch_contains_header_and_lines() {
        let hunk = make_hunk(
            "@@ -1,3 +1,4 @@",
            &[
                (' ', "context\n", Some(1), Some(1)),
                ('+', "added\n", None, Some(2)),
                (' ', "more\n", Some(2), Some(3)),
            ],
        );
        let patch = hunk_patch("src/foo.rs", &hunk);
        assert!(patch.contains("diff --git a/src/foo.rs b/src/foo.rs"));
        assert!(patch.contains("--- a/src/foo.rs"));
        assert!(patch.contains("+added"));
        assert!(patch.contains(" context"));
    }

    #[test]
    fn lines_patch_omits_unselected_additions() {
        let hunk = make_hunk(
            "@@ -1,2 +1,3 @@",
            &[
                (' ', "ctx\n", Some(1), Some(1)),
                ('+', "lineA\n", None, Some(2)),
                ('+', "lineB\n", None, Some(3)),
            ],
        );
        // Only select lineA (index 1), not lineB (index 2)
        let patch = lines_patch("file.txt", &hunk, &[1]);
        assert!(patch.contains("+lineA"));
        assert!(!patch.contains("lineB"));
    }

    #[test]
    fn parse_hunk_header_extracts_starts() {
        let (old, new) = parse_hunk_header("@@ -10,5 +20,8 @@ fn foo()");
        assert_eq!(old, 10);
        assert_eq!(new, 20);
    }
}
