//! Git credential-helper protocol. Secrets travel over pipes, never command arguments.
use std::io::{Read, Write};
use std::path::Path;

fn fields(input: &str) -> Result<std::collections::HashMap<&str, &str>, String> {
    let mut fields = std::collections::HashMap::new();
    for line in input.lines().take_while(|line| !line.is_empty()) {
        let (key, value) = line.split_once('=').ok_or("Invalid credential request.")?;
        if fields.insert(key, value).is_some() {
            return Err("Duplicate credential request field.".into());
        }
    }
    Ok(fields)
}

fn remote(input: &str) -> Result<String, String> {
    let fields = fields(input)?;
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

fn forward_decision(
    operation: &str,
    input: &str,
    lookup: impl FnOnce(&str) -> Result<Option<(String, String)>, String>,
) -> Result<Option<&'static str>, String> {
    let action = match operation {
        "store" => "approve",
        "erase" => "reject",
        _ => return Ok(None),
    };
    let fields = fields(input)?;
    if fields.get("protocol") != Some(&"https") {
        return Ok(None);
    }
    let Some(password) = fields.get("password") else {
        return Ok(None);
    };
    let url = remote(input)?;
    match lookup(&url) {
        Err(_) => Ok(None),
        Ok(Some((username, saved_password)))
            if fields.get("username") == Some(&username.as_str())
                && *password == saved_password.as_str() =>
        {
            Ok(None)
        }
        Ok(_) => Ok(Some(action)),
    }
}

pub fn run(operation: &str) {
    // Git sends approval/rejection back to helpers. Recorde accounts are changed
    // only through explicit account settings, never copied into another helper.
    if !matches!(operation, "get" | "store" | "erase") {
        return;
    }
    if operation != "get" && std::env::var_os("RECORDE_HELPER_ACTIVE").is_some() {
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
        if operation != "get" {
            if let Some(action) = forward_decision(operation, &input, |url| {
                super::store::lookup_for_url(url, path.as_deref())
            })? {
                run_system_helper(action, &input, path.as_deref())?;
            }
            return Ok(String::new());
        }
        if let Some(output) = response(&input, |url| {
            super::store::lookup_for_url(url, path.as_deref())
        })? {
            return Ok(output);
        }
        system_helper(&input, path.as_deref())
    })();
    match result {
        Ok(output) if operation == "get" => {
            let _ = std::io::stdout().write_all(output.as_bytes());
        }
        Ok(_) => {}
        Err(error) => {
            // quit prevents Git from continuing with an unrelated account.
            if operation == "get" {
                let _ = std::io::stdout().write_all(b"quit=true\n\n");
            }
            eprintln!("Recorde: {}", super::redact::text(&error));
        }
    }
}

fn system_helper(input: &str, path: Option<&Path>) -> Result<String, String> {
    let output = run_system_helper("fill", input, path)?;
    String::from_utf8(output.stdout).map_err(|_| "Invalid system helper response.".into())
}

fn system_helper_command(action: &str, path: Option<&Path>) -> std::process::Command {
    let mut command = crate::git::process::background_command("git");
    if let Some(path) = path {
        command.arg("-C").arg(path);
    }
    command
        .args(["credential", action])
        // Drop the parent command's temporary helper override before consulting
        // the repository/system helpers. The recursion guard fails closed.
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_CONFIG_COUNT")
        .env("RECORDE_HELPER_ACTIVE", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    command
}

fn run_system_helper(
    action: &str,
    input: &str,
    path: Option<&Path>,
) -> Result<std::process::Output, String> {
    let mut command = system_helper_command(action, path);
    let output = crate::git::process::run_with_input(
        &mut command,
        input.as_bytes(),
        std::time::Duration::from_secs(60),
    )
    .map_err(|_| "System credential helper stopped.")?;
    if !output.status.success() {
        return Err("No credentials available from the configured Git helpers.".into());
    }
    Ok(output)
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

    #[test]
    fn forwards_only_credentials_not_owned_by_recorde() {
        let input = "protocol=https\nhost=github.com\npath=org/repo\nusername=alice\
                     \npassword=secret\n\n";
        let owned = |url: &str| {
            assert_eq!(url, "https://alice@github.com/org/repo");
            Ok(Some(("alice".into(), "secret".into())))
        };
        assert_eq!(forward_decision("store", input, owned), Ok(None));
        assert_eq!(
            forward_decision("store", input, |_| {
                Ok(Some(("alice".into(), "different".into())))
            }),
            Ok(Some("approve"))
        );
        assert_eq!(
            forward_decision("store", input, |_| Ok(Some((
                "bob".into(),
                "secret".into()
            )))),
            Ok(Some("approve"))
        );
        assert_eq!(
            forward_decision("store", input, |_| Ok(None)),
            Ok(Some("approve"))
        );
        assert_eq!(
            forward_decision("erase", input, |_| Ok(None)),
            Ok(Some("reject"))
        );
        assert_eq!(
            forward_decision("erase", input, |_| Err("vault error".into())),
            Ok(None)
        );
        assert_eq!(
            forward_decision("get", input, |_| panic!("unexpected lookup")),
            Ok(None)
        );
    }

    #[test]
    fn ignores_ineligible_forward_requests() {
        let lookup =
            |_: &str| -> Result<Option<(String, String)>, String> { panic!("unexpected lookup") };
        assert_eq!(
            forward_decision(
                "store",
                "protocol=http\nhost=github.com\npassword=secret\n\n",
                lookup
            ),
            Ok(None)
        );
        assert_eq!(
            forward_decision(
                "erase",
                "protocol=https\nhost=github.com\nusername=alice\n\n",
                lookup
            ),
            Ok(None)
        );
    }

    #[test]
    fn forwarding_commands_use_git_credential_actions() {
        for action in ["approve", "reject"] {
            let command = system_helper_command(action, None);
            let args: Vec<_> = command.get_args().collect();
            assert_eq!(
                args,
                [
                    std::ffi::OsStr::new("credential"),
                    std::ffi::OsStr::new(action)
                ]
            );
        }
    }
}
