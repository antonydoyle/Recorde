use super::*;

fn fixture(bytes: &[u8]) -> Manifest {
    let (size, sha256) = protocol::digest(bytes).unwrap();
    Manifest {
        schema: 1,
        key_id: "fixture".into(),
        channel: "stable".into(),
        version: "99.0.0".into(),
        issued_at: 100,
        expires_at: 200,
        platform: "linux".into(),
        arch: std::env::consts::ARCH.into(),
        package_kind: "deb".into(),
        artifact_name: "recorde_99.0.0-1.deb".into(),
        url: protocol::release_url("99.0.0", "recorde_99.0.0-1.deb").unwrap(),
        size,
        sha256,
    }
}

#[test]
fn strict_arguments_and_origin_have_no_trust_or_command_overrides() {
    assert!(args(vec![
        "--envelope".into(),
        "/tmp/offer".into(),
        "--artifact".into(),
        "/tmp/artifact".into()
    ])
    .is_ok());
    for values in [
        vec!["--help"],
        vec!["--envelope", "relative", "--artifact", "/tmp/a"],
        vec![
            "--envelope",
            "/tmp/e",
            "--artifact",
            "/tmp/a",
            "--keys",
            "/tmp/keys",
        ],
    ] {
        assert!(args(values.into_iter().map(Into::into).collect()).is_err());
    }
    assert_eq!(origin(0, 0, Some("1000".into())).unwrap(), 1000);
    for (real, effective, text) in [
        (1000, 1000, "1000"),
        (1000, 0, "1000"),
        (0, 0, "0"),
        (0, 0, "01000"),
        (0, 0, "-1"),
    ] {
        assert!(origin(real, effective, Some(text.into())).is_err());
    }
    assert!(origin(0, 0, None).is_err());
}

#[test]
fn production_verifier_rejects_test_keys_and_bad_envelopes() {
    let raw = serde_json::to_vec(&fixture(b"deb fixture")).unwrap();
    let signed = protocol::sign(&raw, &[17; 32]).unwrap();
    assert!(verify(&signed, 150, None).is_err());
    assert!(verify(b"{}", 150, None).is_err());
    assert!(verify(&vec![b' '; protocol::MAX_ENVELOPE + 1], 150, None).is_err());
}

#[test]
fn package_identity_architecture_and_upstream_are_exact() {
    assert_eq!(
        upstream("0.4.0-1").unwrap(),
        expected_upstream("0.4.0").unwrap()
    );
    assert_eq!(
        upstream("0.4.0~rc.1-2").unwrap(),
        expected_upstream("0.4.0-rc.1").unwrap()
    );
    assert_eq!(
        upstream("0.4.0-alpha-beta-1").unwrap(),
        expected_upstream("0.4.0-alpha-beta").unwrap()
    );
    assert_ne!(
        upstream("0.3.9-999").unwrap(),
        expected_upstream("0.4.0").unwrap()
    );
    for invalid in ["", "0.4.0-", "1:0.4.0-1", "--option", "0.4.0\nanything"] {
        assert!(upstream(invalid).is_err());
    }
    let arch = native_arch().unwrap();
    assert!(package_record(
        format!("recorde\t0.4.0-1\t{arch}\tinstalled\n").as_bytes(),
        true
    )
    .is_ok());
    for text in [
        format!("other\t0.4.0-1\t{arch}\n"),
        "recorde\t0.4.0-1\tall\n".into(),
        format!("recorde\t0.4.0-1\t{arch}\textra\n"),
    ] {
        assert!(package_record(text.as_bytes(), false).is_err());
    }
    assert!(package_record(
        format!("recorde\t0.4.0-1\t{arch}\tunpacked\n").as_bytes(),
        true
    )
    .is_err());
}

#[test]
fn authenticated_upstream_matches_pinned_cargo_deb_conversion() {
    // Literal outputs of cargo-deb 3.8.0's manifest_version_string, without
    // its default -1 Debian revision. The build-metadata cases deliberately
    // retain the packager's first-hyphen behavior rather than normalizing it.
    for (manifest, debian_upstream) in [
        ("0.4.0", "0.4.0"),
        ("0.4.0-rc.1", "0.4.0~rc.1"),
        ("0.4.0-alpha", "0.4.0-alpha"),
        ("0.4.0-1", "0.4.0-1"),
        ("0.4.0-alpha-beta", "0.4.0-alpha-beta"),
        ("0.4.0-alpha-beta.2", "0.4.0~alpha-beta.2"),
        ("0.4.0-1.2", "0.4.0~1.2"),
        ("0.4.0-alpha+build.2", "0.4.0~alpha+build.2"),
        ("0.4.0+build-rc.1", "0.4.0+build~rc.1"),
        ("0.4.0+metadata", "0.4.0+metadata"),
    ] {
        assert_eq!(expected_upstream(manifest).unwrap(), debian_upstream);
        assert_eq!(
            upstream(&format!("{debian_upstream}-1")).unwrap(),
            debian_upstream
        );
    }
    assert!(expected_upstream("0.4.0-01").is_err());
    assert!(expected_upstream("1:0.4.0").is_err());
}

#[test]
fn opened_source_survives_path_replacement_and_private_copy_is_verified() {
    let source = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let path = source.path().join("input.deb");
    std::fs::write(&path, b"signed original").unwrap();
    let input = open_path(&path, unsafe { libc::geteuid() }, false).unwrap();
    std::fs::rename(&path, source.path().join("old.deb")).unwrap();
    std::fs::write(&path, b"attacker replacement").unwrap();
    let staged = staged_copy(input, dest.path(), &fixture(b"signed original")).unwrap();
    assert_eq!(std::fs::read(&staged).unwrap(), b"signed original");
    assert_eq!(std::fs::metadata(staged).unwrap().mode() & 0o777, 0o600);
    let changed = open_path(&path, unsafe { libc::geteuid() }, false).unwrap();
    assert!(staged_copy(
        changed,
        tempfile::tempdir().unwrap().path(),
        &fixture(b"signed original")
    )
    .is_err());
}

#[test]
fn inputs_reject_symlink_ancestors_hardlinks_wrong_owner_and_special_files() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let path = outside.path().join("input");
    std::fs::write(&path, b"data").unwrap();
    let uid = unsafe { libc::geteuid() };
    std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
    assert!(open_path(&dir.path().join("link/input"), uid, false).is_err());
    assert!(open_path(&path, uid.wrapping_add(1), false).is_err());
    std::fs::hard_link(&path, dir.path().join("hard")).unwrap();
    assert!(open_path(&path, uid, false).is_err());
    let fifo =
        std::ffi::CString::new(dir.path().join("fifo").as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert!(open_path(&dir.path().join("fifo"), uid, false).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"data");
}

#[test]
fn caller_identity_restores_and_requires_owner_read_permission() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input");
    std::fs::write(&path, b"caller bytes").unwrap();
    let uid = unsafe { libc::geteuid() };
    let before = (unsafe { libc::setfsuid(!0) }, unsafe { libc::setfsgid(!0) });
    assert_eq!(
        bounded(caller_file(&path, uid).unwrap(), 100).unwrap(),
        b"caller bytes"
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    assert!(caller_file(&path, uid).is_err());
    assert_eq!(
        before,
        (unsafe { libc::setfsuid(!0) }, unsafe { libc::setfsgid(!0) })
    );
}

#[test]
fn no_downgrade_check_uses_debian_revision_ordering_read_only() {
    crate::process::enable_subreaper().unwrap();
    for (new, old, expected) in [
        ("0.4.0-2", "0.4.0-1", true),
        ("0.4.0-1", "0.4.0-1", false),
        ("0.4.0~rc.1-1", "0.4.0-1", false),
    ] {
        let result = crate::process::run(
            crate::process::Tool::Dpkg,
            &[
                OsStr::new("--compare-versions"),
                OsStr::new(new),
                OsStr::new("gt"),
                OsStr::new(old),
            ],
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(result.success, expected);
    }
}
