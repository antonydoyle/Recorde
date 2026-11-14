use super::*;
use crate::git::{operations::compute_workdir_diff, test_support::TestRepo};

#[test]
fn diff_controls_hidden_whitespace_maps_partial_stage_unstage_without_hidden_changes() {
    let repo = TestRepo::new();
    let base = "same\r\nwhite space\r\nsame\r\nold\r\nsame\r\n";
    let working = "same\r\nwhite   space\r\nsame\r\ninserted\r\nnew\r\nsame\r\n";
    repo.commit("file", base.as_bytes());
    repo.write("file", working.as_bytes());
    let comparison = crate::config::DiffComparison {
        diff_context_lines: 1,
        ignore_whitespace: true,
        full_file: false,
    };
    let changes = |diff: &crate::state::FileDiff| {
        diff.hunks
            .iter()
            .enumerate()
            .flat_map(|(h, hunk)| {
                hunk.lines
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| matches!(l.origin, '+' | '-'))
                    .map(move |(l, _)| (h, l))
            })
            .collect::<Vec<_>>()
    };
    let display = super::super::operations::compute_workdir_comparison(
        repo.path(),
        "file",
        false,
        comparison,
    )
    .unwrap();
    assert!(display.canonical_hunks.is_some());
    let selected = changes(&display);
    let patch = multi_selection_patch(&display, &selected, false).unwrap();
    apply_to_index(repo.path(), &patch).unwrap();
    assert_eq!(
        repo.index_bytes("file"),
        working.replace("white   space", "white space").as_bytes()
    );
    // Staging the entire file makes whitespace visible only in the canonical
    // staged patch; unstaging visible lines must retain that whitespace in index.
    repo.git_ok(&["add", "file"]);
    let display =
        super::super::operations::compute_workdir_comparison(repo.path(), "file", true, comparison)
            .unwrap();
    let patch = multi_selection_patch(&display, &changes(&display), true).unwrap();
    remove_from_index(repo.path(), &patch).unwrap();
    assert_eq!(
        repo.index_bytes("file"),
        base.replace("white space", "white   space").as_bytes()
    );
    assert_eq!(
        std::fs::read(repo.path().join("file")).unwrap(),
        working.as_bytes()
    );
    // Pure whitespace differences have no selectable displayed changes.
    repo.write("file", base.as_bytes());
    let hidden = super::super::operations::compute_workdir_comparison(
        repo.path(),
        "file",
        false,
        comparison,
    )
    .unwrap();
    assert!(hidden.hunks.is_empty());
    assert!(!hidden.raw_patch.is_empty());
    assert!(multi_selection_patch(&hidden, &[], false).is_err());
}

#[test]
fn selection_across_hunks_recalculates_offsets_and_round_trips_both_directions() {
    let repo = TestRepo::new();
    let base: String = (0..40).map(|i| format!("line {i}\r\n")).collect();
    let working = base
        .replace("line 1\r\n", "line 1\r\nfirst\r\nignore\r\n")
        .replace("line 30\r\n", "last\r\n");
    repo.commit("file[1].txt", base.as_bytes());
    repo.write("file[1].txt", working.as_bytes());
    let selection = |diff: &crate::state::FileDiff| -> Vec<_> {
        diff.hunks
            .iter()
            .enumerate()
            .flat_map(|(h, hunk)| {
                hunk.lines
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| {
                        matches!(l.origin, '+' | '-')
                            && ["first\r\n", "line 30\r\n", "last\r\n"]
                                .contains(&l.content.as_str())
                    })
                    .map(move |(l, _)| (h, l))
            })
            .collect()
    };
    let diff = compute_workdir_diff(repo.path(), "file[1].txt", false).unwrap();
    assert_eq!(diff.hunks.len(), 2);
    let patch = multi_selection_patch(&diff, &selection(&diff), false).unwrap();
    assert_eq!(patch.matches("diff --git").count(), 1);
    apply_to_index(repo.path(), &patch).unwrap();
    assert_eq!(
        repo.index_bytes("file[1].txt"),
        working.replace("ignore\r\n", "").as_bytes()
    );
    repo.git_ok(&["--literal-pathspecs", "add", "--", "file[1].txt"]);
    let diff = compute_workdir_diff(repo.path(), "file[1].txt", true).unwrap();
    let patch = multi_selection_patch(&diff, &selection(&diff), true).unwrap();
    remove_from_index(repo.path(), &patch).unwrap();
    assert_eq!(
        repo.index_bytes("file[1].txt"),
        base.replace("line 1\r\n", "line 1\r\nignore\r\n")
            .as_bytes()
    );
    assert_eq!(
        std::fs::read(repo.path().join("file[1].txt")).unwrap(),
        working.as_bytes()
    );
}

#[test]
fn combined_selection_rejects_invalid_positions_and_preserves_new_file_eof() {
    let repo = TestRepo::new();
    repo.write("new.txt", b"first\nsecond");
    let diff = compute_workdir_diff(repo.path(), "new.txt", false).unwrap();
    assert!(multi_selection_patch(&diff, &[], false).is_err());
    assert!(multi_selection_patch(&diff, &[(99, 0)], false).is_err());
    let index = diff.hunks[0]
        .lines
        .iter()
        .position(|line| line.content == "second")
        .unwrap();
    let patch = multi_selection_patch(&diff, &[(0, index), (0, index)], false).unwrap();
    apply_to_index(repo.path(), &patch).unwrap();
    assert_eq!(repo.index_bytes("new.txt"), b"second");
}

fn exported_round_trip(repo: &TestRepo, file: &str) {
    let diff = compute_workdir_diff(repo.path(), file, true).unwrap();
    let expected = repo.open().index().unwrap().write_tree().unwrap();
    let output = tempfile::tempdir().unwrap();
    let patch = output.path().join("selected.patch");
    export_patch(&diff, &patch).unwrap();
    repo.git_ok(&["reset", "--hard", "HEAD"]);
    import_patch_file(repo.path(), &patch).unwrap();
    assert_eq!(repo.open().index().unwrap().write_tree().unwrap(), expected);
}

#[test]
fn complete_export_round_trips_multiple_hunks_without_repeated_headers() {
    let repo = TestRepo::new();
    let before = (0..60).map(|i| format!("line {i}\n")).collect::<String>();
    repo.commit("file.txt", before.as_bytes());
    let after = before
        .replace("line 2\n", "changed two\n")
        .replace("line 50\n", "changed fifty\n");
    repo.write("file.txt", after.as_bytes());
    repo.git_ok(&["add", "file.txt"]);
    let diff = compute_workdir_diff(repo.path(), "file.txt", true).unwrap();
    assert_eq!(diff.hunks.len(), 2);
    assert_eq!(
        String::from_utf8_lossy(&diff.raw_patch)
            .matches("diff --git ")
            .count(),
        1
    );
    exported_round_trip(&repo, "file.txt");
}

#[test]
fn complete_export_round_trips_rename_and_binary_payload() {
    let repo = TestRepo::new();
    repo.commit("old name.txt", b"renamed content\n");
    repo.git_ok(&["mv", "old name.txt", "new name.txt"]);
    let diff = compute_workdir_diff(repo.path(), "new name.txt", true).unwrap();
    assert!(diff.is_rename);
    assert_eq!(diff.old_path.as_deref(), Some("old name.txt"));
    exported_round_trip(&repo, "new name.txt");
    repo.git_ok(&["reset", "--hard", "HEAD"]);
    repo.commit("file.bin", b"old\0binary\n");
    repo.write("file.bin", b"new\0binary\r\nwith bytes\xff");
    repo.git_ok(&["add", "file.bin"]);
    let diff = compute_workdir_diff(repo.path(), "file.bin", true).unwrap();
    assert!(diff.is_binary);
    assert!(String::from_utf8_lossy(&diff.raw_patch).contains("GIT binary patch"));
    exported_round_trip(&repo, "file.bin");
}

#[test]
fn selected_file_export_does_not_include_other_staged_changes() {
    let repo = TestRepo::new();
    repo.commit("base", b"base");
    repo.write("literal[1].txt", b"selected\r\n");
    repo.write("literal1.txt", b"other\n");
    repo.git_ok(&["add", "--all"]);
    let diff = compute_workdir_diff(repo.path(), "literal[1].txt", true).unwrap();
    let raw = String::from_utf8_lossy(&diff.raw_patch);
    assert!(raw.contains("selected"));
    assert!(!raw.contains("other"));
    assert!(raw.contains("new file mode"));
    let output = tempfile::tempdir().unwrap();
    let patch = output.path().join("new.patch");
    export_patch(&diff, &patch).unwrap();
    repo.git_ok(&["reset", "--hard", "HEAD"]);
    import_patch_file(repo.path(), &patch).unwrap();
    assert_eq!(repo.index_bytes("literal[1].txt"), b"selected\r\n");
    assert!(repo
        .open()
        .index()
        .unwrap()
        .get_path(Path::new("literal1.txt"), 0)
        .is_none());
}

#[test]
fn new_file_partial_stage_and_unstage_preserve_bytes_and_mode() {
    let repo = TestRepo::new();
    repo.commit("base", b"base");
    let name = "new file.sh";
    repo.write(name, b"first\nsecond\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            repo.path().join(name),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    let diff = compute_workdir_diff(repo.path(), name, false).unwrap();
    let hunk = &diff.hunks[0];
    let first = hunk
        .lines
        .iter()
        .position(|l| l.content == "first\n")
        .unwrap();
    apply_to_index(repo.path(), &selection_patch(&diff, hunk, &[first], false)).unwrap();
    assert_eq!(repo.index_bytes(name), b"first\n");
    #[cfg(unix)]
    assert_eq!(
        repo.open()
            .index()
            .unwrap()
            .get_path(Path::new(name), 0)
            .unwrap()
            .mode,
        0o100755
    );
    repo.git_ok(&["add", "--", name]);
    let diff = compute_workdir_diff(repo.path(), name, true).unwrap();
    let hunk = &diff.hunks[0];
    let first = hunk
        .lines
        .iter()
        .position(|l| l.content == "first\n")
        .unwrap();
    remove_from_index(repo.path(), &selection_patch(&diff, hunk, &[first], true)).unwrap();
    assert_eq!(repo.index_bytes(name), b"second\n");
    let diff = compute_workdir_diff(repo.path(), name, true).unwrap();
    let hunk = &diff.hunks[0];
    let all: Vec<_> = (0..hunk.lines.len()).collect();
    remove_from_index(repo.path(), &selection_patch(&diff, hunk, &all, true)).unwrap();
    assert!(repo
        .open()
        .index()
        .unwrap()
        .get_path(Path::new(name), 0)
        .is_none());
    assert_eq!(
        std::fs::read(repo.path().join(name)).unwrap(),
        b"first\nsecond\n"
    );
}

#[test]
fn deleted_file_partial_stage_and_unstage_keep_remaining_content() {
    let repo = TestRepo::new();
    let name = "deleted.txt";
    repo.commit(name, b"first\nsecond\n");
    std::fs::remove_file(repo.path().join(name)).unwrap();
    let diff = compute_workdir_diff(repo.path(), name, false).unwrap();
    let hunk = &diff.hunks[0];
    let first = hunk
        .lines
        .iter()
        .position(|l| l.content == "first\n")
        .unwrap();
    apply_to_index(repo.path(), &selection_patch(&diff, hunk, &[first], false)).unwrap();
    assert_eq!(repo.index_bytes(name), b"second\n");
    repo.git_ok(&["add", "--", name]);
    let diff = compute_workdir_diff(repo.path(), name, true).unwrap();
    let hunk = &diff.hunks[0];
    let first = hunk
        .lines
        .iter()
        .position(|l| l.content == "first\n")
        .unwrap();
    remove_from_index(repo.path(), &selection_patch(&diff, hunk, &[first], true)).unwrap();
    assert_eq!(repo.index_bytes(name), b"first\n");
    assert!(!repo.path().join(name).exists());
}

fn round_trip(
    base: &[u8],
    working: &[u8],
    wanted: &[u8],
    remaining: &[u8],
    selected_text: &[&str],
    name: &str,
) {
    let repo = TestRepo::new();
    repo.commit(name, base);
    repo.write(name, working);
    let diff = compute_workdir_diff(repo.path(), name, false).unwrap();
    assert_eq!(diff.hunks.len(), 1);
    let hunk = &diff.hunks[0];
    let selected: Vec<_> = hunk
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            matches!(l.origin, '+' | '-') && selected_text.contains(&l.content.as_str())
        })
        .map(|(i, _)| i)
        .collect();
    assert!(!selected.is_empty());
    apply_to_index(repo.path(), &lines_patch(name, hunk, &selected)).unwrap();
    assert_eq!(repo.index_bytes(name), wanted);
    assert_eq!(std::fs::read(repo.path().join(name)).unwrap(), working);

    // Reverse selection from the FULL staged diff, including unselected changes.
    repo.git_ok(&["add", "--", name]);
    let diff = compute_workdir_diff(repo.path(), name, true).unwrap();
    let hunk = &diff.hunks[0];
    let selected: Vec<_> = hunk
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            matches!(l.origin, '+' | '-') && selected_text.contains(&l.content.as_str())
        })
        .map(|(i, _)| i)
        .collect();
    let patch = unstage_lines_patch(name, hunk, &selected);
    remove_from_index(repo.path(), &patch).unwrap();
    assert_eq!(repo.index_bytes(name), remaining);
    assert_eq!(std::fs::read(repo.path().join(name)).unwrap(), working);
    // Reapplying the exact inverse restores the full staged version.
    apply_to_index(repo.path(), &patch).unwrap();
    assert_eq!(repo.index_bytes(name), working);
}

#[test]
fn selected_addition_omits_other_additions() {
    round_trip(
        b"ctx\n",
        b"ctx\nlineA\nlineB\n",
        b"ctx\nlineA\n",
        b"ctx\nlineB\n",
        &["lineA\n"],
        "file.txt",
    );
}

#[test]
fn selected_deletion_preserves_other_deletions() {
    round_trip(
        b"ctx\nlineA\nlineB\n",
        b"ctx\n",
        b"ctx\nlineB\n",
        b"ctx\nlineA\n",
        &["lineA\n"],
        "file.txt",
    );
}

#[test]
fn selected_replacement_preserves_other_replacements() {
    round_trip(
        b"a\nb\nc\n",
        b"A\nb\nC\n",
        b"A\nb\nc\n",
        b"a\nb\nC\n",
        &["a\n", "A\n"],
        "file.txt",
    );
}

#[test]
fn crlf_and_unicode_paths_round_trip() {
    round_trip(
        b"ctx\r\n",
        b"ctx\r\nA\r\nB\r\n",
        b"ctx\r\nA\r\n",
        b"ctx\r\nB\r\n",
        &["A\r\n"],
        "spaced é file.txt",
    );
}

#[test]
fn missing_final_newline_is_preserved() {
    round_trip(
        b"ctx\nold",
        b"ctx\nnew",
        b"ctx\nnew",
        b"ctx\nold",
        &["old", "new"],
        "file.txt",
    );
}

#[test]
fn later_hunk_with_different_old_and_new_offsets_round_trips() {
    let repo = TestRepo::new();
    let base = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n";
    let working = format!("first\n{}A\nB\n", base);
    repo.commit("file.txt", base.as_bytes());
    repo.write("file.txt", working.as_bytes());
    let diff = compute_workdir_diff(repo.path(), "file.txt", false).unwrap();
    assert_eq!(diff.hunks.len(), 2);
    let hunk = &diff.hunks[1];
    let selected = hunk.lines.iter().position(|l| l.content == "A\n").unwrap();
    apply_to_index(repo.path(), &lines_patch("file.txt", hunk, &[selected])).unwrap();
    assert_eq!(
        repo.index_bytes("file.txt"),
        format!("{}A\n", base).as_bytes()
    );
}
