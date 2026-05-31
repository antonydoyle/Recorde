/// Token storage in ~/.local/share/recorde/tokens.toml (chmod 600 on Unix).
/// Tokens are stored in plain text in a user-only file. For higher security,
/// replace with a keyring crate that uses the OS credential store.
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
struct TokenFile {
    #[serde(default)]
    tokens: Vec<TokenEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct TokenEntry {
    host: String,
    username: String,
    token: String,
}

fn token_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("recorde")
        .join("tokens.toml")
}

fn read_file() -> TokenFile {
    std::fs::read_to_string(token_path())
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_file(tf: &TokenFile) -> Result<(), String> {
    let path = token_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content = toml::to_string_pretty(tf).map_err(|e| e.to_string())?;
    std::fs::write(&path, &content).map_err(|e| e.to_string())?;

    // Restrict to owner-read/write on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Save or update a token for (host, username).
pub fn save(host: &str, username: &str, token: &str) -> Result<(), String> {
    let mut tf = read_file();
    tf.tokens.retain(|e| !(e.host == host && e.username == username));
    tf.tokens.push(TokenEntry {
        host: host.to_string(),
        username: username.to_string(),
        token: token.to_string(),
    });
    write_file(&tf)
}

/// Retrieve the stored token for (host, username), if any.
pub fn load(host: &str, username: &str) -> Option<String> {
    read_file().tokens.into_iter()
        .find(|e| e.host == host && e.username == username)
        .map(|e| e.token)
}

/// Remove the stored token for (host, username).
pub fn remove(host: &str, username: &str) {
    let mut tf = read_file();
    tf.tokens.retain(|e| !(e.host == host && e.username == username));
    let _ = write_file(&tf);
}

/// Given an HTTPS remote URL, return (username, token) if we have a stored
/// credential for that host. The username in the token entry is used as the
/// HTTP username; for GitHub/GitLab PATs the password is the token itself.
pub fn lookup_for_url(url: &str) -> Option<(String, String)> {
    let host = extract_host(url)?;
    let tf = read_file();
    // Prefer an exact host match; accept a suffix match for enterprise hosts
    let entry = tf.tokens.iter()
        .find(|e| host == e.host || host.ends_with(&format!(".{}", e.host)))?;
    Some((entry.username.clone(), entry.token.clone()))
}

fn extract_host(url: &str) -> Option<String> {
    // HTTPS URL: https://host/path  or  https://user@host/path
    let url = url.trim_start_matches("https://").trim_start_matches("http://");
    let url = url.split_once('@').map(|(_, rest)| rest).unwrap_or(url);
    let host = url.split('/').next()?.split(':').next()?; // strip port if present
    Some(host.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_host_plain() {
        assert_eq!(extract_host("https://github.com/owner/repo.git").as_deref(), Some("github.com"));
    }

    #[test]
    fn extract_host_with_user() {
        assert_eq!(extract_host("https://alice@gitlab.com/owner/repo.git").as_deref(), Some("gitlab.com"));
    }

    #[test]
    fn extract_host_enterprise() {
        assert_eq!(extract_host("https://git.corp.example.com/owner/repo").as_deref(), Some("git.corp.example.com"));
    }
}
