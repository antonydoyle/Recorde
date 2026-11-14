use super::*;
use crate::git::test_support::{snapshot, TestRepo};

fn dirty_fixture() -> TestRepo {
    let repo = TestRepo::new();
    let hooks = repo.open().path().join("empty-hooks");
    std::fs::create_dir(&hooks).unwrap();
    repo.open()
        .config()
        .unwrap()
        .set_str("core.hooksPath", hooks.to_str().unwrap())
        .unwrap();
    std::fs::remove_dir(repo.path().join("empty-hooks")).unwrap();
    repo.commit(".gitignore", b"/ignored.env\n");
    repo.commit("file.txt", b"base\n");
    repo.git_ok(&["branch", "destination"]);
    repo.write("file.txt", b"staged\n");
    repo.git_ok(&["add", "file.txt"]);
    repo.write("file.txt", b"unstaged\n");
    repo.write("untracked.bin", b"new bytes\r\n\xff\x00");
    repo.write("ignored.env", b"ignored bytes\xff\x00");
    repo
}

fn saved(repo: &TestRepo) -> SavedWork {
    let (records, warnings) = load(repo.path());
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(records.len(), 1);
    records[0].clone()
}

#[test]
fn exact_staging_and_untracked_restore_survives_restart_stash_reorder_and_gc() {
    let repo = dirty_fixture();
    let before = snapshot(repo.path());
    let preview = Preview::capture(repo.path(), 0, Target::Local("destination".into())).unwrap();
    assert_eq!(
        (preview.tracked, preview.untracked, preview.ignored),
        (1, 1, 1)
    );
    preview.execute(Choice::Stash, || Ok(())).unwrap();
    let record = saved(&repo);
    assert_eq!(record.phase, Phase::Switched);
    let saved_oid = record.stash_oid.clone().unwrap();
    assert_eq!(
        repo.open().head().unwrap().shorthand().unwrap(),
        "destination"
    );
    assert_eq!(repo.index_bytes("file.txt"), b"base\n");
    assert!(!repo.path().join("untracked.bin").exists());
    assert_eq!(
        std::fs::read(repo.path().join("ignored.env")).unwrap(),
        b"ignored bytes\xff\x00"
    );
    repo.write("another.txt", b"later stash\n");
    repo.git_ok(&[
        "stash",
        "push",
        "--include-untracked",
        "-m",
        "unrelated stash",
    ]);
    let unrelated = repo.open().refname_to_id("refs/stash").unwrap();
    repo.git_ok(&["stash", "drop", "stash@{1}"]);
    repo.git_ok(&["reflog", "expire", "--expire=now", "--all"]);
    repo.git_ok(&["gc", "--prune=now"]);
    let restore = saved(&repo).prepare_restore(repo.path(), 0).unwrap();
    assert_eq!(
        (restore.tracked, restore.untracked, restore.ignored),
        (0, 0, 1)
    );
    restore.execute(|| Ok(())).unwrap();
    let restored = saved(&repo);
    assert_eq!(restored.phase, Phase::Restored);
    assert!(!restored.can_restore());
    assert!(restored.prepare_restore(repo.path(), 0).is_err());
    assert_eq!(repo.open().refname_to_id("refs/stash").unwrap(), unrelated);
    assert_eq!(
        repo.open()
            .refname_to_id(&restored.pin())
            .unwrap()
            .to_string(),
        saved_oid
    );
    assert_eq!(repo.index_bytes("file.txt"), b"staged\n");
    assert_eq!(
        std::fs::read(repo.path().join("file.txt")).unwrap(),
        b"unstaged\n"
    );
    repo.git_ok(&["checkout", "main"]);
    assert_eq!(snapshot(repo.path()), before);
}

#[test]
fn stale_source_target_index_worktree_untracked_and_session_refuse_before_mutation() {
    for kind in 0..7 {
        let repo = dirty_fixture();
        let preview =
            Preview::capture(repo.path(), 0, Target::Local("destination".into())).unwrap();
        match kind {
            0 => repo.git_ok(&["checkout", "-b", "same-tip"]),
            1 => {
                repo.commit("moved.txt", b"moved head\n");
            }
            2 => {
                repo.write("index.txt", b"staged later");
                repo.git_ok(&["add", "index.txt"]);
            }
            3 => repo.write("file.txt", b"external edit"),
            4 => repo.write("untracked.bin", b"different untracked bytes"),
            5 => {
                let other = repo.commit("target.txt", b"moved target");
                repo.git_ok(&["reset", "--hard", &preview.source.head_oid]);
                repo.open()
                    .reference("refs/heads/destination", other, true, "fixture")
                    .unwrap();
            }
            _ => {}
        }
        let before = snapshot(repo.path());
        let result = preview.execute(Choice::Stash, || {
            if kind == 6 {
                Err("Repository session changed".into())
            } else {
                Ok(())
            }
        });
        assert!(result.is_err(), "stale kind{kind}");
        assert_eq!(snapshot(repo.path()), before, "stale kind{kind}");
        assert!(load(repo.path()).0.is_empty());
    }
}

#[test]
fn clean_and_dirty_bring_changes_cover_shared_destination_types() {
    for (kind, dirty) in (0..6).flat_map(|kind| [(kind, false), (kind, true)]) {
        let repo = TestRepo::new();
        let base = repo.commit("file.txt", b"base\n");
        let tip = repo.commit("target.txt", b"destination\n");
        repo.git_ok(&["branch", "destination"]);
        repo.git_ok(&["reset", "--hard", &base.to_string()]);
        repo.open()
            .remote("origin", "https://example.invalid/fixture")
            .unwrap();
        repo.open()
            .reference("refs/remotes/origin/topic", tip, false, "fixture")
            .unwrap();
        repo.open()
            .reference("refs/recorde/reviews/7", tip, false, "fixture")
            .unwrap();
        let target = match kind {
            0 => Target::Local("destination".into()),
            1 => Target::New {
                name: "created".into(),
                oid: Some(tip.to_string()),
            },
            2 => Target::Remote("origin/topic".into()),
            3 => {
                repo.git_ok(&["branch", "--track", "topic", "origin/topic"]);
                Target::Remote("origin/topic".into())
            }
            4 => Target::Detached(tip.to_string()),
            _ => Target::Review {
                branch: "review/7".into(),
                oid: tip.to_string(),
                remote: "origin".into(),
                remote_url_digest: url_digest("https://example.invalid/fixture"),
                reference: "refs/recorde/reviews/7".into(),
            },
        };
        if dirty {
            repo.write("file.txt", b"staged\n");
            repo.git_ok(&["add", "file.txt"]);
            repo.write("file.txt", b"working\n");
            repo.write("draft.txt", b"untracked\n");
        }
        Preview::capture(repo.path(), 0, target)
            .unwrap()
            .execute(Choice::Bring, || Ok(()))
            .unwrap();
        assert_eq!(repo.head(), tip, "target{kind}");
        assert_eq!(
            repo.index_bytes("file.txt"),
            if dirty {
                b"staged\n".as_slice()
            } else {
                b"base\n".as_slice()
            }
        );
        assert_eq!(
            std::fs::read(repo.path().join("file.txt")).unwrap(),
            if dirty {
                b"working\n".as_slice()
            } else {
                b"base\n".as_slice()
            }
        );
        if dirty {
            assert_eq!(
                std::fs::read(repo.path().join("draft.txt")).unwrap(),
                b"untracked\n"
            );
        }
    }
}

#[test]
fn ignored_collision_after_stash_keeps_exact_saved_work_and_original_branch() {
    let (repo, _) = crate::git::test_support::ignored_collision();
    repo.write("draft.txt", b"untracked work");
    Preview::capture(repo.path(), 0, Target::Local("destination".into()))
        .unwrap()
        .execute(Choice::Stash, || Ok(()))
        .unwrap_err();
    let record = saved(&repo);
    assert_eq!(record.phase, Phase::CheckoutUnknown);
    assert_eq!(repo.open().head().unwrap().shorthand().unwrap(), "main");
    assert_eq!(
        std::fs::read(repo.path().join("ignored.env")).unwrap(),
        b"local ignored bytes\r\n\xff\x00"
    );
    record
        .prepare_restore(repo.path(), 0)
        .unwrap()
        .execute(|| Ok(()))
        .unwrap();
    assert_eq!(
        std::fs::read(repo.path().join("draft.txt")).unwrap(),
        b"untracked work"
    );
}

#[test]
fn conflicted_restore_keeps_stash_pin_partial_files_and_consumes_retry() {
    let repo = dirty_fixture();
    // An unstaged edit reaches the merge-conflict path; a staged patch may
    // instead be refused by --index before Git starts the content merge.
    repo.git_ok(&["reset", "--", "file.txt"]);
    Preview::capture(repo.path(), 0, Target::Local("destination".into()))
        .unwrap()
        .execute(Choice::Stash, || Ok(()))
        .unwrap();
    repo.commit("file.txt", b"destination divergent\n");
    let record = saved(&repo);
    let stash = record.stash_oid.clone();
    assert!(record
        .prepare_restore(repo.path(), 0)
        .unwrap()
        .execute(|| Ok(()))
        .is_err());
    let after = saved(&repo);
    assert_eq!(after.phase, Phase::RestoreFailed);
    assert_eq!(after.stash_oid, stash);
    assert!(!after.can_restore());
    assert!(after.prepare_restore(repo.path(), 0).is_err());
    assert_eq!(
        repo.open()
            .refname_to_id(&record.pin())
            .unwrap()
            .to_string(),
        stash.unwrap()
    );
    assert!(
        std::fs::read(repo.path().join("file.txt"))
            .unwrap()
            .windows(7)
            .any(|p| p == b"<<<<<<<")
            || repo.open().index().unwrap().has_conflicts()
    );
}

#[test]
fn restore_refuses_changed_pin_files_and_consumed_journal() {
    for kind in 0..3 {
        let repo = dirty_fixture();
        Preview::capture(repo.path(), 0, Target::Local("destination".into()))
            .unwrap()
            .execute(Choice::Stash, || Ok(()))
            .unwrap();
        let record = saved(&repo);
        let restore = record.prepare_restore(repo.path(), 0).unwrap();
        match kind {
            0 => {
                repo.open()
                    .reference(&record.pin(), repo.head(), true, "external edit")
                    .unwrap();
            }
            1 => repo.write("external.txt", b"changed after preview"),
            _ => {
                let mut changed = record.clone();
                changed.phase = Phase::RestoreSubmitted;
                changed.write(Some(&record.bytes().unwrap())).unwrap();
            }
        }
        let before = snapshot(repo.path());
        assert!(restore.execute(|| Ok(())).is_err());
        assert_eq!(snapshot(repo.path()), before);
    }
}

#[test]
fn phase_process_fixture() {
    let Some(path) = std::env::var_os("RECORDE_SWITCH_FIXTURE") else {
        return;
    };
    let path = PathBuf::from(path);
    if std::env::var_os("RECORDE_SWITCH_RESTORE").is_some() {
        let (records, warnings) = load(&path);
        assert!(warnings.is_empty(), "{warnings:?}");
        records[0]
            .prepare_restore(&path, 0)
            .unwrap()
            .execute(|| Ok(()))
            .unwrap();
    } else {
        Preview::capture(&path, 0, Target::Local("destination".into()))
            .unwrap()
            .execute(Choice::Stash, || Ok(()))
            .unwrap();
    }
    panic!("Expected fixture process to stop at durable phase");
}

fn crash_at(repo: &TestRepo, phase: &str, restore: bool) {
    let mut command = crate::git::process::background_command(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "git::switch::tests::phase_process_fixture",
            "--nocapture",
        ])
        .env("RECORDE_SWITCH_FIXTURE", repo.path())
        .env("RECORDE_SWITCH_EXIT_AT", phase);
    if restore {
        command.env("RECORDE_SWITCH_RESTORE", "1");
    }
    let output = crate::git::process::run(
        &mut command,
        &AtomicBool::new(false),
        std::time::Duration::from_secs(45),
    )
    .unwrap();
    assert_eq!(
        output.status.code(),
        Some(55),
        "phase{phase}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn real_process_restart_never_repeats_submitted_stash_or_restore() {
    for (point, phase, has_stash) in [
        ("prepared", Phase::Prepared, false),
        ("stash-submitted", Phase::StashSubmitted, false),
        ("stash-created", Phase::StashSubmitted, true),
        ("stash-pinned", Phase::StashSubmitted, true),
        ("stashed", Phase::Stashed, true),
        ("switching", Phase::Switching, true),
        ("checkout-completed", Phase::Switching, true),
    ] {
        let repo = dirty_fixture();
        crash_at(&repo, point, false);
        let record = saved(&repo);
        assert_eq!(record.phase, phase, "{point}");
        assert_eq!(
            repo.open().refname_to_id("refs/stash").is_ok(),
            has_stash,
            "{point}"
        );
        let before = snapshot(repo.path());
        let _ = load(repo.path());
        assert_eq!(snapshot(repo.path()), before, "reload{point}");
        if phase == Phase::StashSubmitted || phase == Phase::Prepared {
            assert!(!record.can_restore());
            assert!(record.prepare_restore(repo.path(), 0).is_err());
            if has_stash {
                let before = snapshot(repo.path());
                record.reconcile(repo.path()).unwrap();
                assert_eq!(
                    snapshot(repo.path()),
                    before,
                    "reconcile must not switch/apply"
                );
                saved(&repo)
                    .prepare_restore(repo.path(), 0)
                    .unwrap()
                    .execute(|| Ok(()))
                    .unwrap();
                assert_eq!(repo.index_bytes("file.txt"), b"staged\n");
            } else {
                assert!(record.reconcile(repo.path()).is_err());
            }
        } else {
            record
                .prepare_restore(repo.path(), 0)
                .unwrap()
                .execute(|| Ok(()))
                .unwrap();
            assert_eq!(repo.index_bytes("file.txt"), b"staged\n");
        }
    }
    for point in ["restore-submitted", "restore-applied"] {
        let repo = dirty_fixture();
        Preview::capture(repo.path(), 0, Target::Local("destination".into()))
            .unwrap()
            .execute(Choice::Stash, || Ok(()))
            .unwrap();
        crash_at(&repo, point, true);
        let record = saved(&repo);
        assert_eq!(record.phase, Phase::RestoreSubmitted);
        assert!(!record.can_restore());
        assert!(record.prepare_restore(repo.path(), 0).is_err());
        if point == "restore-applied" {
            assert_eq!(repo.index_bytes("file.txt"), b"staged\n");
        }
        let before = snapshot(repo.path());
        let _ = load(repo.path());
        assert_eq!(snapshot(repo.path()), before);
    }
}

#[test]
fn corrupt_oversized_and_redirected_journals_are_preserved_with_warnings() {
    let repo = dirty_fixture();
    let git = repo.open();
    let directory = git.path().join("recorde/switch");
    std::fs::create_dir_all(&directory).unwrap();
    let id = new_id();
    let file = directory.join(format!("{id}.json"));
    std::fs::write(&file, b"not json").unwrap();
    let (records, warnings) = load(repo.path());
    assert!(records.is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(std::fs::read(&file).unwrap(), b"not json");
    std::fs::write(&file, vec![b'x'; RECORD_LIMIT as usize + 1]).unwrap();
    assert!(!load(repo.path()).1.is_empty());
    assert!(!valid_id("../escape"));
    assert!(!valid_id("0000000000000000-1-2/other"));
}

#[test]
fn cancellation_before_start_and_during_checkout_keeps_work_and_records_actual_uncertainty() {
    use std::sync::Arc;
    let repo = dirty_fixture();
    let preview = Preview::capture(repo.path(), 0, Target::Local("destination".into())).unwrap();
    let before = snapshot(repo.path());
    assert!(
        crate::git::process::with_cancellation(Arc::new(AtomicBool::new(true)), || preview
            .execute(Choice::Stash, || Ok(())))
        .is_err()
    );
    assert_eq!(snapshot(repo.path()), before);
    assert!(load(repo.path()).0.is_empty());
    let git = repo.open();
    let hook = git.path().join("empty-hooks/post-checkout");
    std::fs::write(
        &hook,
        b"#!/bin/sh\nprintf ready > .git/recorde-switch-marker\nsleep 30\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let work_cancel = cancel.clone();
    let worker = std::thread::spawn(move || {
        crate::git::process::with_cancellation(work_cancel, || {
            preview.execute(Choice::Stash, || Ok(()))
        })
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !git.path().join("recorde-switch-marker").exists() && std::time::Instant::now() < deadline
    {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    let started = git.path().join("recorde-switch-marker").exists();
    cancel.store(true, Ordering::Relaxed);
    let outcome = worker.join().unwrap();
    assert!(outcome.is_err());
    assert!(
        started,
        "post-checkout did not start; worker outcome: {outcome:?}"
    );
    let record = saved(&repo);
    assert_eq!(record.phase, Phase::CheckoutUnknown);
    assert_eq!(
        repo.open().head().unwrap().shorthand().unwrap(),
        "destination"
    );
    std::fs::remove_file(&hook).unwrap();
    record
        .prepare_restore(repo.path(), 0)
        .unwrap()
        .execute(|| Ok(()))
        .unwrap();
    assert_eq!(repo.index_bytes("file.txt"), b"staged\n");
    assert_eq!(
        std::fs::read(repo.path().join("untracked.bin")).unwrap(),
        b"new bytes\r\n\xff\x00"
    );
}

#[test]
fn pin_collision_and_journal_write_refusal_stop_before_checkout_without_overwriting() {
    for pin_collision in [false, true] {
        let repo = dirty_fixture();
        let preview =
            Preview::capture(repo.path(), 0, Target::Local("destination".into())).unwrap();
        let before = snapshot(repo.path());
        let calls = std::cell::Cell::new(0);
        let result = preview.execute(Choice::Stash, || {
            calls.set(calls.get() + 1);
            if calls.get() == 3 {
                let record = saved(&repo);
                if pin_collision {
                    repo.open()
                        .reference(&record.pin(), repo.head(), false, "external pin collision")
                        .unwrap();
                } else {
                    let file = repo
                        .open()
                        .path()
                        .join(format!("recorde/switch/{}.json", record.id));
                    let mut permissions = std::fs::metadata(&file).unwrap().permissions();
                    permissions.set_readonly(true);
                    std::fs::set_permissions(&file, permissions).unwrap();
                }
            }
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(repo.open().head().unwrap().shorthand().unwrap(), "main");
        let record = saved(&repo);
        if pin_collision {
            assert_eq!(record.phase, Phase::StashSubmitted);
            assert_eq!(
                repo.open().refname_to_id(&record.pin()).unwrap(),
                repo.head()
            );
            assert!(record.reconcile(repo.path()).is_err());
            let stash = repo.open().refname_to_id("refs/stash").unwrap();
            record.verify_stash(&repo.open(), stash).unwrap();
        } else {
            assert_eq!(record.phase, Phase::Prepared);
            assert_eq!(snapshot(repo.path()), before);
            let file = repo
                .open()
                .path()
                .join(format!("recorde/switch/{}.json", record.id));
            let mut permissions = std::fs::metadata(&file).unwrap().permissions();
            permissions.set_readonly(false);
            std::fs::set_permissions(file, permissions).unwrap();
        }
        assert!(!record.recovery_id.is_empty());
        assert!(repo
            .open()
            .find_reference(&format!("refs/recorde/recovery/{}", record.recovery_id))
            .is_ok());
    }
}

#[test]
fn journal_ancestor_redirection_is_refused_before_saving_or_switching() {
    let repo = dirty_fixture();
    let outside = tempfile::tempdir().unwrap();
    let preview = Preview::capture(repo.path(), 0, Target::Local("destination".into())).unwrap();
    let link = repo.open().path().join("recorde");
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), &link).unwrap();
    #[cfg(windows)]
    {
        let mut command = crate::git::process::background_command("cmd.exe");
        command
            .args(["/D", "/C", "mklink", "/J"])
            .arg(link.to_string_lossy().replace('/', "\\"))
            .arg(outside.path().to_string_lossy().replace('/', "\\"));
        let output = crate::git::process::run(
            &mut command,
            &AtomicBool::new(false),
            std::time::Duration::from_secs(15),
        )
        .unwrap();
        assert!(
            output.status.success(),
            "junction fixture: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let before = snapshot(repo.path());
    assert!(preview.execute(Choice::Stash, || Ok(())).is_err());
    assert_eq!(snapshot(repo.path()), before);
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    assert!(!load(repo.path()).1.is_empty());
    #[cfg(unix)]
    std::fs::remove_file(link).unwrap();
    #[cfg(windows)]
    std::fs::remove_dir(link).unwrap();
}

#[test]
fn shared_switch_routes_refuse_ignored_collisions_without_changing_any_files() {
    for kind in 0..6 {
        let (repo, tip) = crate::git::test_support::ignored_collision();
        repo.open()
            .remote("origin", "https://example.invalid/fixture")
            .unwrap();
        repo.open()
            .reference("refs/remotes/origin/topic", tip, false, "fixture")
            .unwrap();
        repo.open()
            .reference("refs/recorde/reviews/7", tip, false, "fixture")
            .unwrap();
        let target = match kind {
            0 => Target::Local("destination".into()),
            1 => Target::New {
                name: "created".into(),
                oid: Some(tip.to_string()),
            },
            2 => Target::Remote("origin/topic".into()),
            3 => {
                repo.git_ok(&["branch", "--track", "topic", "origin/topic"]);
                Target::Remote("origin/topic".into())
            }
            4 => Target::Detached(tip.to_string()),
            _ => Target::Review {
                branch: "review/7".into(),
                oid: tip.to_string(),
                remote: "origin".into(),
                remote_url_digest: url_digest("https://example.invalid/fixture"),
                reference: "refs/recorde/reviews/7".into(),
            },
        };
        let preview = Preview::capture(repo.path(), 0, target).unwrap();
        let before = snapshot(repo.path());
        assert!(
            preview.execute(Choice::Bring, || Ok(())).is_err(),
            "switch kind{kind}"
        );
        assert_eq!(snapshot(repo.path()), before, "switch kind{kind}");
    }
}

#[test]
fn replaced_objects_remote_retargeting_and_linked_worktree_ownership_invalidate_review() {
    for kind in 0..3 {
        let repo = TestRepo::new();
        let base = repo.commit("base.txt", b"base\n");
        let other = repo.commit("other.txt", b"other\n");
        repo.git_ok(&["reset", "--hard", &base.to_string()]);
        repo.git_ok(&["branch", "destination"]);
        repo.open()
            .remote("origin", "https://example.invalid/original")
            .unwrap();
        repo.open()
            .reference("refs/recorde/reviews/7", base, false, "fixture")
            .unwrap();
        let target = if kind == 1 {
            Target::Review {
                branch: "review/7".into(),
                oid: base.to_string(),
                remote: "origin".into(),
                remote_url_digest: url_digest("https://example.invalid/original"),
                reference: "refs/recorde/reviews/7".into(),
            }
        } else {
            Target::Local("destination".into())
        };
        let preview = Preview::capture(repo.path(), 0, target).unwrap();
        let external = tempfile::tempdir().unwrap();
        match kind {
            0 => {
                repo.open()
                    .reference(
                        &format!("refs/replace/{base}"),
                        other,
                        false,
                        "external replacement",
                    )
                    .unwrap();
            }
            1 => repo
                .open()
                .remote_set_url("origin", "https://example.invalid/changed")
                .unwrap(),
            _ => repo.git_ok(&[
                "worktree",
                "add",
                external.path().join("linked").to_str().unwrap(),
                "destination",
            ]),
        }
        let before = snapshot(repo.path());
        assert!(preview.execute(Choice::Bring, || Ok(())).is_err());
        assert_eq!(snapshot(repo.path()), before);
    }
}

#[test]
fn no_changes_stash_and_cancelled_or_stale_queued_preview_never_mutate() {
    let repo = TestRepo::new();
    repo.commit("file.txt", b"base\n");
    repo.git_ok(&["branch", "destination"]);
    let preview = Preview::capture(repo.path(), 0, Target::Local("destination".into())).unwrap();
    let before = snapshot(repo.path());
    assert!(!preview.has_changes());
    assert!(preview.execute(Choice::Stash, || Ok(())).is_err());
    assert_eq!(snapshot(repo.path()), before);
    assert!(load(repo.path()).0.is_empty());
    let state = crate::state::new_shared_state();
    state.lock().unwrap().repo_path = Some(repo.path().canonicalize().unwrap());
    request(Target::Local("destination".into()), state.clone());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !state.lock().unwrap().local_jobs.is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    assert!(state.lock().unwrap().pending_switch.take().is_some());
    assert_eq!(snapshot(repo.path()), before);
    let hold = crate::git::mutation::reserve(repo.path());
    request(Target::Local("destination".into()), state.clone());
    state.lock().unwrap().repo_epoch += 1;
    drop(hold);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !state.lock().unwrap().local_jobs.is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    assert!(state.lock().unwrap().pending_switch.is_none());
    assert_eq!(snapshot(repo.path()), before);
}

#[test]
fn delayed_local_review_fetch_cannot_publish_into_another_or_reopened_session() {
    for navigation in 0..3 {
        let repo = TestRepo::new();
        let base = repo.commit("file.txt", b"base\n");
        let tip = repo.commit("file.txt", b"review\n");
        let remote = tempfile::tempdir().unwrap();
        git2::Repository::init_bare(remote.path()).unwrap();
        repo.git_ok(&["remote", "add", "origin", remote.path().to_str().unwrap()]);
        repo.git_ok(&["push", "origin", "HEAD:refs/pull/42/head"]);
        repo.git_ok(&["reset", "--hard", &base.to_string()]);
        let other = TestRepo::new();
        other.commit("other.txt", b"other repository\n");
        let state = crate::state::new_shared_state();
        let source = repo.path().canonicalize().unwrap();
        {
            let mut s = state.lock().unwrap();
            s.repo_path = Some(source.clone());
            s.repo_epoch = 7;
        }
        // A real local fetch completes after its initiating session was captured.
        let target = crate::git::reviews::prepare(
            repo.path(),
            "origin",
            &crate::git::capabilities::ProviderType::GitHub,
            42,
            &tip.to_string(),
        )
        .unwrap();
        if navigation > 0 {
            let mut s = state.lock().unwrap();
            s.repo_path = Some(other.path().canonicalize().unwrap());
            s.repo_epoch = 8;
            if navigation == 2 {
                s.repo_path = Some(source.clone());
                s.repo_epoch = 9;
            }
        }
        let before_a = snapshot(repo.path());
        let before_b = snapshot(other.path());
        request_for(&source, 7, target, state.clone());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            let s = state.lock().unwrap();
            let done = s.local_jobs.is_empty()
                && (navigation > 0 || s.pending_switch.is_some() || s.error.is_some());
            drop(s);
            if done || std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        let s = state.lock().unwrap();
        assert!(s.error.is_none(), "navigation{navigation}: {:?}", s.error);
        if navigation == 0 {
            assert_eq!(s.pending_switch.as_ref().unwrap().path, source);
        } else {
            assert!(s.pending_switch.is_none());
            assert!(s.local_jobs.is_empty());
        }
        assert_eq!(snapshot(repo.path()), before_a);
        assert_eq!(snapshot(other.path()), before_b);
    }
}

#[cfg(unix)]
#[test]
fn unix_directory_sync_uses_a_readable_handle_to_the_held_inode() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("owned")).unwrap();
    std::fs::write(root.path().join("owned/witness"), b"original").unwrap();
    std::fs::write(outside.path().join("witness"), b"outside").unwrap();
    let directory =
        cap_std::fs::Dir::open_ambient_dir(root.path().join("owned"), cap_std::ambient_authority())
            .unwrap();
    sync_directory(&directory).unwrap();
    std::fs::rename(root.path().join("owned"), root.path().join("parked")).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("owned")).unwrap();
    sync_directory(&directory).unwrap();
    assert_eq!(directory.read("witness").unwrap(), b"original");
    assert_eq!(
        std::fs::read(outside.path().join("witness")).unwrap(),
        b"outside"
    );
}
