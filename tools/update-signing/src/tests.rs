use super::*;

fn fixture() -> (Manifest, KeySet, [u8; 32]) {
    let seed = [71; 32];
    let payload = b"fixture executable";
    let (size, sha256) = digest(&payload[..]).unwrap();
    let manifest = Manifest {
        schema: 1,
        key_id: "fixture".into(),
        channel: "stable".into(),
        version: "1.2.3".into(),
        issued_at: 1000,
        expires_at: 2000,
        platform: "linux".into(),
        arch: "x86_64".into(),
        package_kind: "portable".into(),
        artifact_name: "recorde-1.2.3-linux-x86_64".into(),
        url: release_url("1.2.3", "recorde-1.2.3-linux-x86_64").unwrap(),
        size,
        sha256,
    };
    (
        manifest,
        KeySet {
            schema: 1,
            keys: vec![public_key(&seed, "fixture".into())],
        },
        seed,
    )
}
fn target() -> Target {
    Target {
        platform: "linux".into(),
        arch: "x86_64".into(),
        package_kind: "portable".into(),
        channel: "stable".into(),
    }
}

#[test]
fn verifies_only_pinned_domain_separated_raw_bytes_before_parsing() {
    let (manifest, keys, seed) = fixture();
    let raw = serde_json::to_vec(&manifest).unwrap();
    let signed = sign(&raw, &seed).unwrap();
    assert_eq!(verify(&signed, &keys).unwrap().version, "1.2.3");
    let mut envelope: Envelope = serde_json::from_slice(&signed).unwrap();
    envelope.manifest_base64 = STANDARD.encode(b"not even JSON");
    let error = verify(&serde_json::to_vec(&envelope).unwrap(), &keys).unwrap_err();
    assert!(error.contains("signature verification"));
    let wrong_domain = SigningKey::from_bytes(&seed).sign(&raw);
    envelope.manifest_base64 = STANDARD.encode(&raw);
    envelope.signature_base64 = STANDARD.encode(wrong_domain.to_bytes());
    assert!(verify(&serde_json::to_vec(&envelope).unwrap(), &keys).is_err());
    let mut unknown = keys.clone();
    unknown.keys[0] = public_key(&[72; 32], "other".into());
    assert!(verify(&signed, &unknown).is_err());
    assert!(verify(
        &signed,
        &KeySet {
            schema: 1,
            keys: vec![]
        }
    )
    .is_err());
}

#[test]
fn pinned_overlap_rotation_accepts_both_keys_but_not_forged_key_identity() {
    let (mut manifest, mut keys, seed) = fixture();
    let next = [19; 32];
    keys.keys.push(public_key(&next, "next".into()));
    assert!(verify(
        &sign(&serde_json::to_vec(&manifest).unwrap(), &seed).unwrap(),
        &keys
    )
    .is_ok());
    manifest.key_id = "next".into();
    assert!(verify(
        &sign(&serde_json::to_vec(&manifest).unwrap(), &next).unwrap(),
        &keys
    )
    .is_ok());
    assert!(verify(
        &sign(&serde_json::to_vec(&manifest).unwrap(), &seed).unwrap(),
        &keys
    )
    .is_err());
    keys.keys.push(keys.keys[0].clone());
    assert!(verify(
        &sign(&serde_json::to_vec(&manifest).unwrap(), &next).unwrap(),
        &keys
    )
    .is_err());
}

#[test]
fn rejects_manifest_tampering_duplicate_fields_and_oversized_envelopes() {
    let (manifest, keys, seed) = fixture();
    let mut raw = serde_json::to_vec(&manifest).unwrap();
    let mut envelope: Envelope = serde_json::from_slice(&sign(&raw, &seed).unwrap()).unwrap();
    let index = raw.iter().position(|b| *b == b'1').unwrap();
    raw[index] = b'2';
    envelope.manifest_base64 = STANDARD.encode(&raw);
    assert!(verify(&serde_json::to_vec(&envelope).unwrap(), &keys).is_err());
    assert!(verify(&vec![b' '; MAX_ENVELOPE + 1], &keys).is_err());
    let signed = sign(&serde_json::to_vec(&manifest).unwrap(), &seed).unwrap();
    let text = String::from_utf8(signed).unwrap();
    let duplicate = text.replacen('{', "{\"manifest_base64\":\"garbage\",", 1);
    assert!(verify(duplicate.as_bytes(), &keys).is_err());
}

#[test]
fn guards_semver_channel_platform_expiry_and_persistent_replay_floor() {
    let (mut manifest, _, _) = fixture();
    validate_offer(&manifest, &target(), "1.2.2", 1500, None).unwrap();
    assert!(validate_offer(&manifest, &target(), "1.2.3", 1500, None).is_err());
    assert!(validate_offer(&manifest, &target(), "1.2.3+other", 1500, None).is_err());
    assert!(validate_offer(&manifest, &target(), "1.2.2", 2000, None).is_err());
    assert!(validate_offer(&manifest, &target(), "1.2.2", 399, None).is_err());
    let seen = Checkpoint {
        version: "1.2.4".into(),
        issued_at: 1000,
    };
    assert!(validate_offer(&manifest, &target(), "1.2.2", 1500, Some(&seen)).is_err());
    let seen = Checkpoint {
        version: "1.2.3".into(),
        issued_at: 1001,
    };
    assert!(validate_offer(&manifest, &target(), "1.2.2", 1500, Some(&seen)).is_err());
    let mut wrong = target();
    wrong.arch = "aarch64".into();
    assert!(validate_offer(&manifest, &wrong, "1.2.2", 1500, None).is_err());
    wrong = target();
    wrong.package_kind = "appimage".into();
    assert!(validate_offer(&manifest, &wrong, "1.2.2", 1500, None).is_err());
    manifest.version = "1.2.4-rc.1".into();
    manifest.url = release_url(&manifest.version, &manifest.artifact_name).unwrap();
    assert!(validate_shape(&manifest).is_err());
    manifest.channel = "preview".into();
    let mut preview = target();
    preview.channel = "preview".into();
    validate_offer(&manifest, &preview, "1.2.4-beta.2", 1500, None).unwrap();
    assert!(validate_offer(&manifest, &preview, "1.2.4", 1500, None).is_err());
}

#[test]
fn signed_urls_are_bound_to_exact_repository_release_and_artifact() {
    let (mut manifest, _, _) = fixture();
    for url in [
        "http://github.com/antonydoyle/Recorde/releases/download/v1.2.3/a",
        "https://github.com.evil.test/antonydoyle/Recorde/releases/download/v1.2.3/a",
        "https://github.com/attacker/Recorde/releases/download/v1.2.3/a",
        "https://github.com/antonydoyle/Recorde/releases/download/v1.2.2/a",
        "https://token@github.com/antonydoyle/Recorde/releases/download/v1.2.3/a",
        "https://github.com/antonydoyle/Recorde/releases/download/v1.2.3/a?secret=x",
    ] {
        manifest.url = url.into();
        assert!(validate_shape(&manifest).is_err());
    }
    assert!(release_url("1.2.3", "../recorde").is_err());
    assert!(release_url("1.2.3", "recorde%2f.exe").is_err());
}

#[test]
fn streamed_artifacts_reject_tampering_truncation_and_oversize() {
    let (manifest, _, _) = fixture();
    let mut output = vec![];
    copy_verified(&b"fixture executable"[..], &mut output, &manifest).unwrap();
    assert_eq!(output, b"fixture executable");
    assert!(copy_verified(&b"fixture Executable"[..], std::io::sink(), &manifest).is_err());
    assert!(copy_verified(&b"fixture"[..], std::io::sink(), &manifest).is_err());
    assert!(copy_verified(&b"fixture executable extra"[..], std::io::sink(), &manifest).is_err());
}
#[test]
fn cross_channel_floor_and_clock_rollback_are_preserved() {
    let (manifest, _, _) = fixture();
    let stable = target();
    let mut preview = stable.clone();
    preview.channel = "preview".into();
    assert_eq!(stable.checkpoint_key(), preview.checkpoint_key());
    validate_offer(&manifest, &preview, "1.2.2", 1500, None).unwrap();
    let seen = Checkpoint {
        version: "2.0.0-beta.1".into(),
        issued_at: 1000,
    };
    assert!(
        validate_offer(&manifest, &stable, "1.2.2", 1500, Some(&seen))
            .unwrap_err()
            .contains("rollback")
    );
    let seen = Checkpoint {
        version: "1.2.3".into(),
        issued_at: 1500,
    };
    assert!(
        validate_offer(&manifest, &stable, "1.2.2", 800, Some(&seen))
            .unwrap_err()
            .contains("clock")
    );
}

#[test]
fn cancellable_stream_never_writes_when_cancelled_or_out_of_time() {
    let (manifest, _, _) = fixture();
    let cancelled = std::sync::atomic::AtomicBool::new(true);
    let mut output = vec![];
    assert!(copy_verified_cancelled(
        &b"fixture executable"[..],
        &mut output,
        &manifest,
        &cancelled,
        std::time::Duration::from_secs(1)
    )
    .unwrap_err()
    .contains("cancelled"));
    assert!(output.is_empty());
    cancelled.store(false, std::sync::atomic::Ordering::Relaxed);
    assert!(copy_verified_cancelled(
        &b"fixture executable"[..],
        &mut output,
        &manifest,
        &cancelled,
        std::time::Duration::ZERO
    )
    .unwrap_err()
    .contains("time limit"));
    assert!(output.is_empty());
}

#[test]
fn universal_architecture_is_only_valid_for_macos() {
    let (mut manifest, _, _) = fixture();
    manifest.arch = "universal".into();
    assert!(validate_shape(&manifest).is_err());
    manifest.platform = "macos".into();
    manifest.package_kind = "dmg".into();
    validate_shape(&manifest).unwrap();
}

#[test]
fn staging_control_names_and_windows_devices_cannot_be_artifacts() {
    for name in [
        "apply.json",
        "ENVELOPE.JSON",
        "healthy",
        "continue",
        "recorde-update-helper.exe",
        "NUL",
        "con.exe",
        "COM1.log",
        "Lpt9",
        "recorde.",
    ] {
        assert!(release_url("1.2.3", name).is_err(), "{name}");
    }
    assert!(release_url("1.2.3", "recorde-1.2.3.exe").is_ok());
}
