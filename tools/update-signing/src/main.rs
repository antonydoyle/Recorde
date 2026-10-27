use base64::{engine::general_purpose::STANDARD, Engine};
use recorde_update_signing::{self as protocol, KeySet, Manifest};
use std::collections::HashMap;
use std::path::Path;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn read(path: &str, limit: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut bytes = vec![];
    std::fs::File::open(path)
        .map_err(|_| "Cannot open input file.")?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read input file.")?;
    if bytes.len() > limit {
        return Err("Input exceeds its size limit.".into());
    }
    Ok(bytes)
}

fn write(path: &str, bytes: &[u8], secret: bool) -> Result<(), String> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(if secret { 0o600 } else { 0o644 });
    }
    #[cfg(not(unix))]
    {
        let _ = secret;
    }
    let mut file = options
        .open(path)
        .map_err(|_| "Cannot create output file (existing files are never overwritten).")?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "Cannot save output file.".into())
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let command = args
        .next()
        .ok_or("Usage: recorde-update-signing generate|manifest|sign|verify --option value ...")?;
    let mut options = HashMap::new();
    while let Some(name) = args.next() {
        if !name.starts_with("--")
            || options
                .insert(name, args.next().ok_or("Missing option value.")?)
                .is_some()
        {
            return Err("Invalid or duplicate option.".into());
        }
    }
    let allowed: &[&str] = match command.as_str() {
        "generate" | "keygen" => &["--key-id", "--private-key-file", "--public-key-file"],
        "manifest" => &[
            "--artifact",
            "--asset-name",
            "--version",
            "--key-id",
            "--channel",
            "--issued-at",
            "--expires-at",
            "--platform",
            "--arch",
            "--kind",
            "--output",
        ],
        "sign" => &["--key-file", "--manifest", "--output"],
        "verify" => &["--keys", "--envelope", "--artifact"],
        _ => return Err("Unknown command. Use generate, manifest, sign or verify.".into()),
    };
    if options.keys().any(|name| !allowed.contains(&name.as_str())) {
        return Err("Unknown command option.".into());
    }
    let value = |name: &str| {
        options
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| format!("Missing {name}."))
    };
    match command.as_str() {
        "generate" | "keygen" => {
            let private_path = value("--private-key-file")?;
            if Path::new(private_path).exists() || Path::new(value("--public-key-file")?).exists() {
                return Err("Key output files already exist; no keys were changed.".into());
            }
            let mut seed = [0u8; 32];
            getrandom::fill(&mut seed).map_err(|_| "OS random source failed.")?;
            let key = protocol::public_key(&seed, value("--key-id")?.into());
            let public = serde_json::to_vec_pretty(&KeySet {
                schema: 1,
                keys: vec![key],
            })
            .map_err(|_| "Cannot serialize public key.")?;
            write(private_path, STANDARD.encode(seed).as_bytes(), true)?;
            write(value("--public-key-file")?, &public, false)?;
            seed.fill(0);
            println!("Signing key saved to the private file; only the public key file belongs in the repository.");
        }
        "manifest" => {
            let artifact = std::fs::File::open(value("--artifact")?)
                .map_err(|_| "Cannot open release artifact.")?;
            let (size, sha256) = protocol::digest(artifact)?;
            let version = value("--version")?.to_string();
            let artifact_name = value("--asset-name")?.to_string();
            let manifest = Manifest {
                schema: 1,
                key_id: value("--key-id")?.into(),
                channel: value("--channel")?.into(),
                url: protocol::release_url(&version, &artifact_name)?,
                version,
                artifact_name,
                issued_at: value("--issued-at")?
                    .parse()
                    .map_err(|_| "issued-at must be Unix seconds.")?,
                expires_at: value("--expires-at")?
                    .parse()
                    .map_err(|_| "expires-at must be Unix seconds.")?,
                platform: value("--platform")?.into(),
                arch: value("--arch")?.into(),
                package_kind: value("--kind")?.into(),
                size,
                sha256,
            };
            protocol::validate_shape(&manifest)?;
            write(
                value("--output")?,
                &serde_json::to_vec(&manifest).map_err(|_| "Cannot serialize manifest.")?,
                false,
            )?;
        }
        "sign" => {
            let secret = if let Some(path) = options.get("--key-file") {
                String::from_utf8(read(path, 512)?).map_err(|_| "Invalid private key encoding.")?
            } else {
                std::env::var("RECORDE_UPDATE_SIGNING_KEY").map_err(|_| "Provide --key-file or RECORDE_UPDATE_SIGNING_KEY; never pass key bytes as an argument.")?
            };
            let mut seed: [u8; 32] = STANDARD
                .decode(secret.trim())
                .map_err(|_| "Invalid private key encoding.")?
                .try_into()
                .map_err(|_| "Private key must be a base64-encoded 32-byte Ed25519 seed.")?;
            let raw = read(value("--manifest")?, protocol::MAX_ENVELOPE / 2)?;
            let envelope = protocol::sign(&raw, &seed)?;
            seed.fill(0);
            write(value("--output")?, &envelope, false)?;
        }
        "verify" => {
            let keys = serde_json::from_slice(&read(value("--keys")?, protocol::MAX_ENVELOPE)?)
                .map_err(|_| "Invalid public key file.")?;
            let manifest =
                protocol::verify(&read(value("--envelope")?, protocol::MAX_ENVELOPE)?, &keys)?;
            if let Some(artifact) = options.get("--artifact") {
                protocol::copy_verified(
                    std::fs::File::open(artifact).map_err(|_| "Cannot open artifact.")?,
                    std::io::sink(),
                    &manifest,
                )?;
            }
            println!(
                "Verified {} {} {} {}",
                manifest.version, manifest.platform, manifest.arch, manifest.package_kind
            );
        }
        _ => return Err("Unknown command. Use generate, manifest, sign or verify.".into()),
    }
    Ok(())
}
