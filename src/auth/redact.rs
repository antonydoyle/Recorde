//! Last-line protection for diagnostic messages containing remote URLs or PATs.
use std::sync::{Mutex, OnceLock};

static SECRETS: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn remember(secret: &str) {
    if secret.is_empty() {
        return;
    }
    if let Ok(mut secrets) = SECRETS.lock() {
        if !secrets.iter().any(|s| s == secret) {
            secrets.push(secret.into());
        }
    }
}

pub fn text(input: &str) -> String {
    static USERINFO: OnceLock<regex::Regex> = OnceLock::new();
    static QUERY: OnceLock<regex::Regex> = OnceLock::new();
    let userinfo = USERINFO.get_or_init(|| regex::Regex::new(r"(?i)(https?://)[^\s/]+@").unwrap());
    let query = QUERY.get_or_init(|| regex::Regex::new(r"(?i)(https?://[^\s?]+)\?[^\s]*").unwrap());
    let mut output = userinfo.replace_all(input, "${1}[redacted]@").into_owned();
    output = query.replace_all(&output, "${1}?[redacted]").into_owned();
    if let Ok(secrets) = SECRETS.lock() {
        for secret in secrets.iter() {
            output = output.replace(secret, "[redacted]");
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hides_url_secrets_and_echoed_vault_tokens() {
        remember("unique-test-pat-472");
        let output = text("failed https://alice:password@host/a/b?access_token=query-secret echoed unique-test-pat-472");
        for secret in ["password", "query-secret", "unique-test-pat-472", "alice"] {
            assert!(!output.contains(secret));
        }
        assert!(output.contains("host/a/b"));
        assert_eq!(text("ordinary failure"), "ordinary failure");
    }
}
