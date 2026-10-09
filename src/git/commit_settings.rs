//! Native Git owns identity resolution, signing, and key-agent access.
use std::path::Path;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signing {
    pub enabled: bool,
    pub format: String,
    pub key: String,
}

fn value(path: &Path, key: &str, boolean: bool) -> Result<Option<String>, String> {
    let mut command = super::process::command(path);
    command.args(["config", "--get"]);
    if boolean {
        command.arg("--bool");
    }
    command.arg(key);
    let out = super::process::run(
        &mut command,
        &AtomicBool::new(false),
        std::time::Duration::from_secs(30),
    )
    .map_err(|e| e.user_message())?;
    if out.status.code() == Some(1) {
        return Ok(None);
    }
    if !out.status.success() {
        return Err(crate::auth::redact::text(&String::from_utf8_lossy(
            &out.stderr,
        )));
    }
    let text = String::from_utf8(out.stdout).map_err(|_| "Git configuration must be UTF-8")?;
    Ok(Some(text.trim_end_matches(['\r', '\n']).into()))
}

pub fn load(path: &Path) -> Result<Signing, String> {
    Ok(Signing {
        enabled: value(path, "commit.gpgsign", true)?.as_deref() == Some("true"),
        format: value(path, "gpg.format", false)?.unwrap_or_else(|| "openpgp".into()),
        key: value(path, "user.signingkey", false)?.unwrap_or_default(),
    })
}

pub fn save(path: &Path, settings: &Signing) -> Result<(), String> {
    if !matches!(settings.format.as_str(), "openpgp" | "ssh" | "x509") {
        return Err("Select OpenPGP, SSH, or X.509 signing.".into());
    }
    if settings.key.len() > 4096 || settings.key.chars().any(char::is_control) {
        return Err("Use a key ID, public key, or key path without control characters.".into());
    }
    if settings.enabled && settings.format == "ssh" && settings.key.trim().is_empty() {
        return Err("SSH signing needs a signing key or public key path.".into());
    }
    // Use Git's existing worktree scope when enabled; never turn it on implicitly.
    let scope = if value(path, "extensions.worktreeConfig", true)?.as_deref() == Some("true") {
        "--worktree"
    } else {
        "--local"
    };
    for (key, setting) in [
        ("gpg.format", settings.format.as_str()),
        ("user.signingkey", settings.key.as_str()),
        (
            "commit.gpgsign",
            if settings.enabled { "true" } else { "false" },
        ),
    ] {
        if key == "user.signingkey" && setting.trim().is_empty() {
            let mut command = super::process::command(path);
            command.args(["config", scope, "--unset-all", key]);
            let out = super::process::run(
                &mut command,
                &AtomicBool::new(false),
                std::time::Duration::from_secs(30),
            )
            .map_err(|e| e.user_message())?;
            if !out.status.success() && out.status.code() != Some(5) {
                return Err(crate::auth::redact::text(&String::from_utf8_lossy(
                    &out.stderr,
                )));
            }
            continue;
        }
        super::process::git(
            path,
            &["config", scope, "--replace-all", "--", key, setting],
            &AtomicBool::new(false),
        )
        .map_err(|e| {
            format!(
                "Could not finish saving signing settings; reload to inspect current values. {}",
                e.user_message()
            )
        })?;
    }
    Ok(())
}

pub fn validate_identity(name: &str, email: &str) -> Result<(), String> {
    for (label, text) in [("Name", name), ("Email", email)] {
        if text.trim().is_empty()
            || text.len() > 4096
            || text
                .chars()
                .any(|c| c.is_control() || matches!(c, '<' | '>'))
        {
            return Err(format!(
                "{label} is required and must not contain control characters or angle brackets."
            ));
        }
    }
    Ok(())
}

pub(super) fn check_identity(path: &Path, amend: bool) -> Result<(), String> {
    let vars: &[&str] = if amend {
        &["GIT_COMMITTER_IDENT"]
    } else {
        &["GIT_COMMITTER_IDENT", "GIT_AUTHOR_IDENT"]
    };
    for var in vars {
        super::process::git(path, &["var", var], &AtomicBool::new(false))
            .map_err(|e| format!("Git could not resolve the commit identity. Set Name and Email in Settings > Repository, then retry. Your commit draft is retained.\n{}", e.user_message()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::TestRepo;

    #[test]
    fn signing_settings_are_literal_and_worktree_scoped() {
        let main = TestRepo::new();
        main.commit("file", b"base");
        main.git_ok(&["config", "extensions.worktreeConfig", "true"]);
        let root = tempfile::tempdir().unwrap();
        let linked = root.path().join("linked");
        main.git_ok(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
        let settings = Signing {
            enabled: true,
            format: "ssh".into(),
            key: "key with spaces;$(echo literal).pub".into(),
        };
        save(&linked, &settings).unwrap();
        assert_eq!(load(&linked).unwrap(), settings);
        assert!(!load(main.path()).unwrap().enabled);
        let invalid = Signing {
            key: "bad\nvalue".into(),
            ..settings.clone()
        };
        assert!(save(&linked, &invalid).is_err());
        assert_eq!(load(&linked).unwrap(), settings);
    }

    #[test]
    fn identity_validation_rejects_empty_and_header_injection() {
        assert!(validate_identity("", "a@b").is_err());
        assert!(validate_identity("User", "a@b\nInjected").is_err());
        assert!(validate_identity("User <other>", "a@b").is_err());
        assert!(validate_identity("Test User", "user@example.invalid").is_ok());
    }
}
