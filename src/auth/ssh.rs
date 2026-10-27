use std::path::{Path, PathBuf};

pub struct SshDiagnostics {
    pub agent_running: bool,
    pub agent_keys: Vec<String>,        // lines from ssh-add -l
    pub key_files: Vec<(String, bool)>, // (name, exists)
}

pub fn diagnose() -> SshDiagnostics {
    let mut command = crate::git::process::background_command("ssh-add");
    command.arg("-l");
    let agent_output = crate::git::process::run(
        &mut command,
        &std::sync::atomic::AtomicBool::new(false),
        std::time::Duration::from_secs(10),
    );

    let (agent_running, agent_keys) = match agent_output {
        Ok(out) if out.status.success() => {
            let keys = String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(str::to_string)
                .collect();
            (true, keys)
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            // Exit 1 with "The agent has no identities" = agent running but empty
            if out.status.code() == Some(1) && !stderr.contains("Could not") {
                (true, Vec::new())
            } else {
                (false, Vec::new())
            }
        }
        Err(_) => (false, Vec::new()),
    };

    let key_files = key_file_status();

    SshDiagnostics {
        agent_running,
        agent_keys,
        key_files,
    }
}

fn ssh_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".ssh")
}

fn key_file_status() -> Vec<(String, bool)> {
    let ssh_dir = ssh_dir();
    ["id_ed25519", "id_ecdsa", "id_rsa", "id_ed25519_sk"]
        .iter()
        .map(|name| (name.to_string(), ssh_dir.join(name).exists()))
        .collect()
}

/// The first standard key name that is not taken yet.
pub fn suggested_key_path() -> PathBuf {
    let dir = ssh_dir();
    let mut path = dir.join("id_ed25519");
    let mut n = 2;
    while path.exists() || public_path(&path).exists() {
        path = dir.join(format!("id_ed25519_{n}"));
        n += 1;
    }
    path
}

fn public_path(private: &Path) -> PathBuf {
    let mut name = private.as_os_str().to_owned();
    name.push(".pub");
    PathBuf::from(name)
}

/// Generate an OpenSSH Ed25519 key pair in process and return its public key.
/// Secrets never reach process arguments, environment variables or temporary
/// plaintext files. A nonempty passphrase uses bcrypt-PBKDF and AES-256-CTR.
pub fn generate_key(private: &Path, comment: &str, passphrase: &str) -> Result<String, String> {
    use ssh_key::{rand_core::OsRng, Algorithm, LineEnding, PrivateKey};
    use std::io::Write;

    let public = public_path(private);
    // symlink_metadata also detects dangling links; create_new below closes the
    // race between this friendly check and installation without replacement.
    for path in [private, &public] {
        match std::fs::symlink_metadata(path) {
            Ok(_) => {
                return Err(format!(
                    "{} already exists. Choose another file name.",
                    path.display()
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Cannot inspect the chosen SSH key path.".into()),
        }
    }
    if comment.chars().any(char::is_control) || passphrase.chars().any(char::is_control) {
        return Err("The comment and passphrase cannot contain control characters.".into());
    }
    if passphrase.len() > 1023 {
        return Err("The passphrase is too long for OpenSSH (maximum 1023 UTF-8 bytes).".into());
    }
    if let Some(parent) = private.parent().filter(|path| !path.as_os_str().is_empty()) {
        if !parent.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                std::fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(parent)
                    .map_err(|e| e.to_string())?;
            }
            #[cfg(not(unix))]
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    let mut key = PrivateKey::random(&mut OsRng, Algorithm::Ed25519)
        .map_err(|_| "Could not generate an Ed25519 key using the OS random source.")?;
    key.set_comment(comment);
    let public_key = key
        .public_key()
        .to_openssh()
        .map_err(|_| "Could not encode the public key.")?;
    let saved = if passphrase.is_empty() {
        key
    } else {
        key.encrypt(&mut OsRng, passphrase)
            .map_err(|_| "Could not encrypt the private key.")?
    };
    // ssh-key returns a Zeroizing<String>; no additional plaintext copy is made.
    let encoded = saved
        .to_openssh(LineEnding::LF)
        .map_err(|_| "Could not encode the private key.")?;
    let mut private_file = crate::update::create_private_file(private)?;
    private_file.write_all(encoded.as_bytes()).and_then(|_| private_file.sync_all())
        .map_err(|_| format!("Could not finish writing {}. The incomplete file was retained; choose another name or remove it before retrying.", private.display()))?;
    let mut public_file = crate::update::create_private_file(&public).map_err(|error| {
        format!(
            "Private key saved at {}; could not create {} without replacement: {error}",
            private.display(),
            public.display()
        )
    })?;
    writeln!(public_file, "{public_key}")
        .and_then(|_| public_file.sync_all())
        .map_err(|_| {
            format!(
                "Private key saved at {}; could not finish writing its public key at {}.",
                private.display(),
                public.display()
            )
        })?;
    Ok(public_key)
}

pub fn read_public_key(private: &Path) -> Result<String, String> {
    let key = std::fs::read_to_string(public_path(private)).map_err(|e| e.to_string())?;
    let key = key.trim();
    if !key.starts_with("ssh-") && !key.starts_with("ecdsa-") && !key.starts_with("sk-") {
        return Err("The public key file is not in OpenSSH format.".into());
    }
    Ok(key.to_string())
}

/// Load the key into the running SSH agent. Keys with a passphrase need a
/// terminal or askpass program, which Recorde does not provide.
pub fn add_to_agent(private: &Path) -> Result<(), String> {
    let mut command = crate::git::process::background_command("ssh-add");
    command.arg(private).env("SSH_ASKPASS_REQUIRE", "never");
    let output = crate::git::process::run(
        &mut command,
        &std::sync::atomic::AtomicBool::new(false),
        std::time::Duration::from_secs(20),
    )
    .map_err(|_| "ssh-add is not available.".to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "ssh-add failed: {}. Keys with a passphrase must be added from a terminal.",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// Where a provider lists the account's SSH keys.
pub fn settings_url(provider: &super::ProviderKind, host: &str) -> Option<String> {
    use super::ProviderKind as P;
    Some(match provider {
        P::GitHubCom | P::GitHubEnterprise => format!("https://{host}/settings/keys"),
        P::GitLabCom | P::GitLabSelf => format!("https://{host}/-/user_settings/ssh_keys"),
        P::BitbucketCloud => "https://bitbucket.org/account/settings/ssh-keys/".into(),
        P::AzureDevOps => return None,
    })
}

fn upload_url(provider: &super::ProviderKind, host: &str) -> Option<String> {
    use super::ProviderKind as P;
    match provider {
        P::GitHubCom => Some("https://api.github.com/user/keys".into()),
        P::GitHubEnterprise => Some(format!("https://{host}/api/v3/user/keys")),
        P::GitLabCom | P::GitLabSelf => Some(format!("https://{host}/api/v4/user/keys")),
        _ => None,
    }
}

pub fn can_upload(provider: &super::ProviderKind) -> bool {
    upload_url(provider, "host").is_some()
}

/// Add the public key to a GitHub or GitLab account.
pub fn upload_key(account: &super::StoredAccount, title: &str, key: &str) -> Result<(), String> {
    let host = super::endpoint::normalize_host(&account.host)?;
    let url = upload_url(&account.provider, &host)
        .ok_or("Upload is available for GitHub and GitLab accounts.")?;
    let token = super::store::token_for_account(&host, &account.username)?
        .ok_or("No token is saved for this account.")?;
    upload_with(&url, &account.provider, &token, title, key)
}

fn upload_with(
    url: &str,
    provider: &super::ProviderKind,
    token: &str,
    title: &str,
    key: &str,
) -> Result<(), String> {
    // Never forward a token to a redirected endpoint.
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(std::time::Duration::from_secs(30))
        .build();
    let result = agent
        .post(url)
        // Bearer works with GitLab PATs and OAuth access tokens; PRIVATE-TOKEN
        // rejects OAuth tokens obtained by Recorde's browser sign-in.
        .set("Authorization", &format!("Bearer {token}"))
        .set("User-Agent", "Recorde")
        .set("Accept", "application/json")
        .send_json(serde_json::json!({ "title": title, "key": key }));
    match result {
        Ok(response) if (200..300).contains(&response.status()) => Ok(()),
        Ok(response) => Err(format!(
            "Provider returned HTTP {}; no redirect was followed and the upload was not confirmed.",
            response.status()
        )),
        Err(ureq::Error::Status(code, response)) => {
            let body = response.into_string().unwrap_or_default();
            let detail = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("message").cloned())
                .map(|m| crate::auth::redact::text(&m.to_string().replace('"', "")))
                .unwrap_or_default();
            Err(match code {
                401 | 403 | 404 => {
                    let guidance = match provider {
                        super::ProviderKind::GitHubCom | super::ProviderKind::GitHubEnterprise =>
                            "GitHub browser sign-in does not request SSH-key write permission. Use Copy and the provider's SSH keys page button to add this public key in your browser, or reconnect with a token that can manage SSH keys (classic PAT: write:public_key; fine-grained token: Git SSH keys write permission).",
                        _ => "GitLab requires an access token with api scope. Reconnect an expired or revoked account, or use Copy and the provider's SSH keys page to add the public key in your browser.",
                    };
                    format!("The provider refused the upload ({code}). {guidance} {detail}")
                }
                400 | 422 => format!("The provider rejected the key: {detail}"),
                _ => format!("Upload failed with status {code}. {detail}"),
            })
        }
        Err(_) => Err("Could not reach the provider. Check the connection and retry.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_ssh_keygen() -> bool {
        let mut command = crate::git::process::background_command("ssh-keygen");
        command.arg("-?");
        crate::git::process::run(
            &mut command,
            &std::sync::atomic::AtomicBool::new(false),
            std::time::Duration::from_secs(10),
        )
        .is_ok()
    }

    #[test]
    fn generates_ed25519_key_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let private = dir.path().join("nested").join("id_test");
        let public = generate_key(&private, "dev@example.com", "").unwrap();
        assert!(public.starts_with("ssh-ed25519 "), "{public}");
        assert!(public.ends_with("dev@example.com"));
        assert_eq!(read_public_key(&private).unwrap(), public);
        let before = std::fs::read(&private).unwrap();
        assert!(generate_key(&private, "other", "").is_err());
        assert_eq!(std::fs::read(&private).unwrap(), before);
        assert!(generate_key(&dir.path().join("x"), "bad\ncomment", "").is_err());
        assert!(!dir.path().join("x").exists());
        let decoded = ssh_key::PrivateKey::from_openssh(std::fs::read(&private).unwrap()).unwrap();
        assert!(!decoded.is_encrypted());
        assert_eq!(decoded.algorithm(), ssh_key::Algorithm::Ed25519);
        assert_eq!(decoded.public_key().to_openssh().unwrap(), public);

        let public_only = dir.path().join("public_only");
        std::fs::write(public_path(&public_only), b"existing public key").unwrap();
        assert!(generate_key(&public_only, "fixture", "y secret").is_err());
        assert!(!public_only.exists());
        assert_eq!(
            std::fs::read(public_path(&public_only)).unwrap(),
            b"existing public key"
        );
    }

    #[test]
    fn encrypted_key_round_trip_rejects_wrong_passphrase_without_subprocesses() {
        let dir = tempfile::tempdir().unwrap();
        let private = dir.path().join("encrypted_key");
        let secret = "y example secret with spaces";
        let public = generate_key(&private, "encrypted fixture", secret).unwrap();
        let encrypted =
            ssh_key::PrivateKey::from_openssh(std::fs::read(&private).unwrap()).unwrap();
        assert!(encrypted.is_encrypted());
        assert_eq!(encrypted.cipher(), ssh_key::Cipher::Aes256Ctr);
        assert!(encrypted.decrypt("wrong").is_err());
        let correct = encrypted.decrypt(secret).unwrap();
        assert_eq!(correct.public_key().to_openssh().unwrap(), public);
        assert_eq!(correct.comment(), "encrypted fixture");
        assert!(!std::fs::read_to_string(&private).unwrap().contains(secret));
        assert!(generate_key(&dir.path().join("too_long"), "fixture", &"x".repeat(1024)).is_err());
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            2,
            "No temporary plaintext keys should be left behind"
        );
    }

    #[test]
    fn native_openssh_accepts_generated_public_key_fingerprint() {
        if !has_ssh_keygen() {
            eprintln!("ssh-keygen unavailable; skipping independent public fingerprint check");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let private = dir.path().join("fingerprint");
        let public = generate_key(&private, "native compatibility fixture", "").unwrap();
        let expected = ssh_key::PublicKey::from_openssh(&public)
            .unwrap()
            .fingerprint(ssh_key::HashAlg::Sha256)
            .to_string();
        let mut command = crate::git::process::background_command("ssh-keygen");
        command
            .args(["-l", "-E", "sha256", "-f"])
            .arg(public_path(&private));
        let output = crate::git::process::run(
            &mut command,
            &std::sync::atomic::AtomicBool::new(false),
            std::time::Duration::from_secs(10),
        )
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains(&expected));
    }

    #[cfg(unix)]
    #[test]
    fn generated_private_key_is_mode_600_even_in_permissive_parent() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        let private = dir.path().join("key");
        generate_key(&private, "permissions fixture", "").unwrap();
        assert_eq!(
            std::fs::metadata(&private).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
            0o777
        );
        let target = dir.path().join("missing");
        let linked = dir.path().join("linked");
        std::os::unix::fs::symlink(&target, &linked).unwrap();
        assert!(generate_key(&linked, "fixture", "").is_err());
        assert!(!target.exists());
    }

    #[test]
    fn upload_sends_title_and_key_and_explains_permission_errors() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/user/keys", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for status in [
                "201 Created",
                "403 Forbidden",
                "302 Found",
                "201 Created",
                "401 Unauthorized",
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                loop {
                    let n = stream.read(&mut buffer).unwrap();
                    assert!(n > 0, "request ended early");
                    request.extend_from_slice(&buffer[..n]);
                    let text = String::from_utf8_lossy(&request).into_owned();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let length = text
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8_lossy(&request).into_owned());
                let body = r#"{"message":"Resource not accessible"}"#;
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
            requests
        });
        let github = super::super::ProviderKind::GitHubCom;
        upload_with(&url, &github, "secret", "laptop", "ssh-ed25519 AAAA test").unwrap();
        let error =
            upload_with(&url, &github, "secret", "laptop", "ssh-ed25519 AAAA test").unwrap_err();
        assert!(error.contains("write:public_key"), "{error}");
        assert!(
            error.contains("browser sign-in") && error.contains("SSH keys page"),
            "{error}"
        );
        let redirect =
            upload_with(&url, &github, "secret", "laptop", "ssh-ed25519 AAAA test").unwrap_err();
        assert!(redirect.contains("302") && redirect.contains("not confirmed"));
        let gitlab = super::super::StoredAccount {
            provider: super::super::ProviderKind::GitLabCom,
            host: "gitlab.com".into(),
            username: "fixture".into(),
        };
        upload_with(
            &url,
            &gitlab.provider,
            "oauth-access-token",
            "gitlab laptop",
            "ssh-ed25519 AAAA test",
        )
        .unwrap();
        let error = upload_with(
            &url,
            &gitlab.provider,
            "oauth-access-token",
            "gitlab laptop",
            "ssh-ed25519 AAAA test",
        )
        .unwrap_err();
        assert!(
            error.contains("api scope") && !error.contains("write:public_key"),
            "{error}"
        );
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("POST /user/keys "));
        assert!(requests[0]
            .to_ascii_lowercase()
            .contains("authorization: bearer secret"));
        assert!(requests[3]
            .to_ascii_lowercase()
            .contains("authorization: bearer oauth-access-token"));
        assert!(!requests[3].to_ascii_lowercase().contains("private-token:"));
        assert!(requests[3].contains(r#""title":"gitlab laptop""#));
        assert!(requests[0].contains(r#""title":"laptop""#));
        assert!(requests[0].contains(r#""key":"ssh-ed25519 AAAA test""#));
    }
}
