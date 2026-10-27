//! Recorde's signed update wire format. No network or process execution.
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

pub const DOMAIN: &[u8] = b"Recorde update manifest v1\0";
pub const MAX_ENVELOPE: usize = 64 * 1024;
pub const MAX_ARTIFACT: u64 = 512 * 1024 * 1024;
pub const MAX_VALIDITY: u64 = 180 * 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeySet {
    pub schema: u32,
    pub keys: Vec<PublicKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicKey {
    pub key_id: String,
    pub public_key_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub key_id: String,
    pub channel: String,
    pub version: String,
    pub issued_at: u64,
    pub expires_at: u64,
    pub platform: String,
    pub arch: String,
    pub package_kind: String,
    pub artifact_name: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub manifest_base64: String,
    pub signature_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub version: String,
    pub issued_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub platform: String,
    pub arch: String,
    pub package_kind: String,
    pub channel: String,
}

impl Target {
    pub fn manifest_name(&self) -> String {
        format!(
            "recorde-update-{}-{}-{}.json",
            self.platform, self.arch, self.package_kind
        )
    }
    pub fn checkpoint_key(&self) -> String {
        format!("{}/{}/{}", self.platform, self.arch, self.package_kind)
    }
}

pub fn release_url(version: &str, name: &str) -> Result<String, String> {
    semver::Version::parse(version).map_err(|_| "Invalid update version.")?;
    if name.is_empty()
        || name.len() > 180
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-+".contains(&b))
        || matches!(name, "." | "..")
    {
        return Err("Invalid release artifact name.".into());
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if name.ends_with('.')
        || matches!(
            name.to_ascii_lowercase().as_str(),
            "apply.json"
                | "envelope.json"
                | "ready"
                | "healthy"
                | "continue"
                | "outcome.txt"
                | "helper-process.json"
                | "helper-identity.json"
                | ".recorde-update-pending.json"
                | "recorde-update-helper"
                | "recorde-update-helper.exe"
                | "recorde-install.json"
        )
        || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        return Err("Reserved update artifact name.".into());
    }
    let mut url =
        url::Url::parse("https://github.com/antonydoyle/Recorde/releases/download/").unwrap();
    url.path_segments_mut()
        .unwrap()
        .pop_if_empty()
        .push(&format!("v{version}"))
        .push(name);
    Ok(url.into())
}

pub fn verify(envelope: &[u8], keys: &KeySet) -> Result<Manifest, String> {
    if envelope.len() > MAX_ENVELOPE {
        return Err("Update manifest exceeds its size limit.".into());
    }
    if keys.schema != 1 || keys.keys.is_empty() || keys.keys.len() > 8 {
        return Err("No valid pinned update signing keys are installed.".into());
    }
    let envelope: Envelope =
        serde_json::from_slice(envelope).map_err(|_| "Invalid signed update envelope.")?;
    let raw = STANDARD
        .decode(envelope.manifest_base64)
        .map_err(|_| "Invalid manifest encoding.")?;
    if raw.len() > MAX_ENVELOPE / 2 {
        return Err("Update manifest exceeds its size limit.".into());
    }
    let signature = Signature::from_slice(
        &STANDARD
            .decode(envelope.signature_base64)
            .map_err(|_| "Invalid signature encoding.")?,
    )
    .map_err(|_| "Invalid signature length.")?;
    let signed = [DOMAIN, raw.as_slice()].concat();
    let mut matched = None;
    let mut ids = std::collections::HashSet::new();
    for key in &keys.keys {
        if !ids.insert(&key.key_id) || key.key_id.is_empty() {
            return Err("Invalid pinned key identifiers.".into());
        }
        let bytes: [u8; 32] = STANDARD
            .decode(&key.public_key_base64)
            .map_err(|_| "Invalid pinned public key.")?
            .try_into()
            .map_err(|_| "Invalid pinned public key length.")?;
        let verifying =
            VerifyingKey::from_bytes(&bytes).map_err(|_| "Invalid pinned public key.")?;
        if verifying.verify_strict(&signed, &signature).is_ok() {
            matched = Some(key.key_id.as_str());
        }
    }
    let matched = matched.ok_or("Update signature verification failed. No update was accepted.")?;
    // Do not parse or act on manifest fields until the raw byte signature is verified.
    let manifest: Manifest =
        serde_json::from_slice(&raw).map_err(|_| "Invalid authenticated update manifest.")?;
    if manifest.schema != 1 || manifest.key_id != matched {
        return Err("Update schema or signing key identity does not match.".into());
    }
    validate_shape(&manifest)?;
    Ok(manifest)
}

pub fn validate_shape(manifest: &Manifest) -> Result<(), String> {
    if manifest.schema != 1 || manifest.key_id.is_empty() || manifest.key_id.len() > 100 {
        return Err("Invalid manifest schema or key identifier.".into());
    }
    let version =
        semver::Version::parse(&manifest.version).map_err(|_| "Invalid update version.")?;
    if !matches!(manifest.channel.as_str(), "stable" | "preview")
        || (manifest.channel == "stable" && !version.pre.is_empty())
    {
        return Err("Update version does not belong to the selected channel.".into());
    }
    if !matches!(manifest.platform.as_str(), "windows" | "linux" | "macos")
        || !matches!(manifest.arch.as_str(), "x86_64" | "aarch64" | "universal")
    {
        return Err("Unsupported update platform or architecture.".into());
    }
    let valid_kind = match manifest.platform.as_str() {
        "windows" => matches!(manifest.package_kind.as_str(), "portable" | "msi"),
        "linux" => matches!(
            manifest.package_kind.as_str(),
            "portable" | "appimage" | "deb"
        ),
        "macos" => matches!(manifest.package_kind.as_str(), "portable" | "dmg"),
        _ => false,
    };
    if manifest.arch == "universal" && manifest.platform != "macos" {
        return Err("Universal architecture is only supported for macOS.".into());
    }
    if !valid_kind {
        return Err("Update package kind does not match the platform.".into());
    }
    if manifest.size == 0
        || manifest.size > MAX_ARTIFACT
        || manifest.sha256.len() != 64
        || !manifest
            .sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("Invalid update artifact size or digest.".into());
    }
    if manifest.issued_at == 0
        || manifest.expires_at <= manifest.issued_at
        || manifest.expires_at - manifest.issued_at > MAX_VALIDITY
    {
        return Err("Invalid update manifest validity period.".into());
    }
    if manifest.url != release_url(&manifest.version, &manifest.artifact_name)? {
        return Err("Update artifact URL is outside its authenticated Recorde release.".into());
    }
    Ok(())
}

pub fn validate_offer(
    manifest: &Manifest,
    target: &Target,
    current_version: &str,
    now: u64,
    checkpoint: Option<&Checkpoint>,
) -> Result<(), String> {
    validate_shape(manifest)?;
    if (&manifest.platform, &manifest.arch, &manifest.package_kind)
        != (&target.platform, &target.arch, &target.package_kind)
        || !(manifest.channel == target.channel
            || (target.channel == "preview" && manifest.channel == "stable"))
    {
        return Err("Update manifest targets a different platform, package or channel.".into());
    }
    if checkpoint.is_some_and(|seen| now.saturating_add(600) < seen.issued_at) {
        return Err("System clock is older than an authenticated update checkpoint. Check the clock before updating.".into());
    }
    let effective_now = now.max(checkpoint.map(|seen| seen.issued_at).unwrap_or(0));
    if manifest.issued_at > now.saturating_add(600) || manifest.expires_at <= effective_now {
        return Err("Update manifest is expired or its clock is in the future. Check the system clock or wait for a freshly signed release.".into());
    }
    let offered =
        semver::Version::parse(&manifest.version).map_err(|_| "Invalid update version.")?;
    let current = semver::Version::parse(current_version)
        .map_err(|_| "Invalid running application version.")?;
    if !offered.cmp_precedence(&current).is_gt() {
        return Err("No newer authenticated version is available.".into());
    }
    if let Some(seen) = checkpoint {
        let version = semver::Version::parse(&seen.version)
            .map_err(|_| "Update rollback checkpoint is invalid; it was preserved.")?;
        if offered.cmp_precedence(&version).is_lt() || manifest.issued_at < seen.issued_at {
            return Err("Update replay or rollback rejected.".into());
        }
    }
    Ok(())
}

pub fn sign(raw: &[u8], seed: &[u8; 32]) -> Result<Vec<u8>, String> {
    if raw.len() > MAX_ENVELOPE / 2 {
        return Err("Manifest exceeds its size limit.".into());
    }
    let manifest: Manifest = serde_json::from_slice(raw).map_err(|_| "Invalid manifest JSON.")?;
    validate_shape(&manifest)?;
    let key = SigningKey::from_bytes(seed);
    let signature = key.sign(&[DOMAIN, raw].concat());
    serde_json::to_vec(&Envelope {
        manifest_base64: STANDARD.encode(raw),
        signature_base64: STANDARD.encode(signature.to_bytes()),
    })
    .map_err(|_| "Cannot serialize signed envelope.".into())
}

pub fn public_key(seed: &[u8; 32], key_id: String) -> PublicKey {
    PublicKey {
        key_id,
        public_key_base64: STANDARD.encode(SigningKey::from_bytes(seed).verifying_key().to_bytes()),
    }
}

pub fn copy_verified(
    reader: impl Read,
    writer: impl Write,
    manifest: &Manifest,
) -> Result<(), String> {
    copy_checked(reader, writer, manifest, || Ok(()))
}

pub fn copy_verified_cancelled(
    reader: impl Read,
    writer: impl Write,
    manifest: &Manifest,
    cancel: &std::sync::atomic::AtomicBool,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let deadline = std::time::Instant::now() + timeout;
    copy_checked(reader, writer, manifest, || {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("Update download cancelled.".into());
        }
        if std::time::Instant::now() >= deadline {
            return Err("Update download exceeded its overall time limit.".into());
        }
        Ok(())
    })
}

fn copy_checked(
    mut reader: impl Read,
    mut writer: impl Write,
    manifest: &Manifest,
    mut check: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    let mut count = 0u64;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        check()?;
        let n = reader
            .read(&mut buffer)
            .map_err(|_| "Update download was interrupted.")?;
        check()?;
        if n == 0 {
            break;
        }
        count = count
            .checked_add(n as u64)
            .ok_or("Update download size overflow.")?;
        if count > manifest.size || count > MAX_ARTIFACT {
            return Err("Update download exceeded the signed size.".into());
        }
        hash.update(&buffer[..n]);
        writer
            .write_all(&buffer[..n])
            .map_err(|_| "Cannot write staged update.")?;
    }
    if count != manifest.size || format!("{:x}", hash.finalize()) != manifest.sha256 {
        return Err(
            "Update artifact is truncated or its digest does not match the signed manifest.".into(),
        );
    }
    writer
        .flush()
        .map_err(|_| "Cannot flush staged update.".into())
}

pub fn digest(mut reader: impl Read) -> Result<(u64, String), String> {
    let mut size = 0u64;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = reader
            .read(&mut buffer)
            .map_err(|_| "Cannot read artifact.")?;
        if n == 0 {
            break;
        }
        size += n as u64;
        if size > MAX_ARTIFACT {
            return Err("Artifact exceeds the update size limit.".into());
        }
        hash.update(&buffer[..n]);
    }
    Ok((size, format!("{:x}", hash.finalize())))
}

#[cfg(test)]
mod tests;
