//! Git credential-helper protocol. Secrets travel over pipes, never command arguments.
use std::io::{Read, Write};
use std::path::Path;

fn remote(input: &str) -> Result<String, String> {
    let mut fields = std::collections::HashMap::new();
    for line in input.lines().take_while(|line| !line.is_empty()) {
        let (key, value) = line.split_once('=').ok_or("Invalid credential request.")?;
        if fields.insert(key, value).is_some() {
            return Err("Duplicate credential request field.".into());
        }
    }
    if fields.get("protocol") != Some(&"https") {
        return Err("Saved credentials require HTTPS.".into());
    }
    let host =
        super::endpoint::normalize_host(fields.get("host").ok_or("Missing credential host.")?)?;
    let path = fields.get("path").copied().unwrap_or("recorde/credential");
    let mut url =
        url::Url::parse(&format!("https://{host}/")).map_err(|_| "Invalid credential host.")?;
    url.set_path(path);
    if let Some(username) = fields.get("username") {
        url.set_username(username)
            .map_err(|_| "Invalid username.")?;
    }
    Ok(url.into())
}

fn response(
    input: &str,
    resolve: impl FnOnce(&str) -> Result<Option<(String, String)>, String>,
) -> Result<Option<String>, String> {
    let url = remote(input)?;
    let Some((username, password)) = resolve(&url)? else {
        return Ok(None);
    };
    if username
        .chars()
        .chain(password.chars())
        .any(char::is_control)
    {
        return Err("Credential contains invalid control characters.".into());
    }
    Ok(Some(format!(
        "username={username}\npassword={password}\n\n"
    )))
}

pub fn run(operation: &str) {
    // Git sends approval/rejection back to helpers. Recorde accounts are changed
    // only through explicit account settings, never copied into another helper.
    if operation != "get" {
        return;
    }
    let result = (|| -> Result<String, String> {
        let mut input = String::new();
        std::io::stdin()
            .take(65536)
            .read_to_string(&mut input)
            .map_err(|_| "Cannot read credential request.")?;
        if input.len() >= 65536 {
            return Err("Credential request is too large.".into());
        }
        if std::env::var_os("RECORDE_HELPER_ACTIVE").is_some() {
            return Err("Recursive credential helper configuration.".into());
        }
        let path = std::env::var_os("RECORDE_CREDENTIAL_REPO").map(std::path::PathBuf::from);
        if let Some(output) = response(&input, |url| {
            super::store::lookup_for_url(url, path.as_deref())
        })? {
            return Ok(output);
        }
        system_helper(&input, path.as_deref())
    })();
    match result {
        Ok(output) => {
            let _ = std::io::stdout().write_all(output.as_bytes());
        }
        Err(error) => {
            // quit prevents Git from continuing with an unrelated account.
            let _ = std::io::stdout().write_all(b"quit=true\n\n");
            eprintln!("Recorde: {}", super::redact::text(&error));
        }
    }
}

fn system_helper(input: &str, path: Option<&Path>) -> Result<String, String> {
    let mut command = crate::git::process::background_command("git");
    if let Some(path) = path {
        command.arg("-C").arg(path);
    }
    command
        .args(["credential", "fill"])
        // Drop the parent command's temporary helper override before consulting
        // the repository/system helpers. The recursion guard fails closed.
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_CONFIG_COUNT")
        .env("RECORDE_HELPER_ACTIVE", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    let output = crate::git::process::run_with_input(
        &mut command,
        input.as_bytes(),
        std::time::Duration::from_secs(60),
    )
    .map_err(|_| "System credential helper stopped.")?;
    if !output.status.success() {
        return Err("No credentials available from the configured Git helpers.".into());
    }
    String::from_utf8(output.stdout).map_err(|_| "Invalid system helper response.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn binds_protocol_host_path_and_username_without_url_injection() {
        let input = "protocol=https\nhost=GitHub.com\npath=org/repo.git\nusername=alice\n\n";
        let output = response(input, |url| {
            assert_eq!(url, "https://alice@github.com/org/repo.git");
            Ok(Some(("alice".into(), "test-pat".into())))
        })
        .unwrap()
        .unwrap();
        assert_eq!(output, "username=alice\npassword=test-pat\n\n");
        assert!(remote("protocol=https\nhost=github.com@evil.test\n").is_err());
        assert!(remote("protocol=http\nhost=github.com\n").is_err());
        assert!(response(input, |_| Ok(Some((
            "a".into(),
            "secret\nquit=false".into()
        ))))
        .is_err());
    }
    #[test]
    fn ambiguity_propagates_instead_of_falling_back() {
        let input = "protocol=https\nhost=github.com\n\n";
        assert!(response(input, |_| Err("choose an account".into())).is_err());
        assert!(response(input, |_| Ok(None)).unwrap().is_none());
    }
}
