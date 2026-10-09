//! Launch external applications without interpreting repository/provider data as shell code.
pub fn web_url(input: &str) -> Result<url::Url, String> {
    if input.chars().any(char::is_control) || input.trim() != input {
        return Err("The link contains invalid whitespace or control characters.".into());
    }
    let url = url::Url::parse(input).map_err(|_| "The provider returned an invalid web link.")?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Only HTTP(S) web links without embedded credentials can be opened.".into());
    }
    Ok(url)
}

pub fn open_url(input: &str) -> Result<(), String> {
    let url = web_url(input)?;
    // webbrowser invokes the registered browser directly on Windows, without cmd/start.
    webbrowser::open(url.as_str()).map_err(|e| format!("Could not open browser: {e}"))
}

/// Small argument parser: quoted spaces work; Windows path separators stay literal.
/// This is not a shell: there is no expansion, substitution or redirection.
pub fn editor_args(input: &str) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    let mut argument = String::new();
    let mut quote = None;
    let mut present = false;
    for ch in input.chars() {
        if ch.is_control() {
            return Err("Editor command contains control characters.".into());
        }
        if quote == Some(ch) {
            quote = None;
        } else if quote.is_none() && matches!(ch, '\'' | '"') {
            quote = Some(ch);
            present = true;
        } else if quote.is_none() && ch.is_whitespace() {
            if present {
                result.push(std::mem::take(&mut argument));
                present = false;
            }
        } else {
            argument.push(ch);
            present = true;
        }
    }
    if quote.is_some() {
        return Err("Close the quoted editor path or argument.".into());
    }
    if present {
        result.push(argument);
    }
    if result.first().is_none_or(|program| program.is_empty()) {
        return Err("Set a default editor in App Settings first.".into());
    }
    Ok(result)
}

pub fn open_editor(path: &std::path::Path, editor: &str) -> Result<(), String> {
    let args = editor_args(editor)?;
    let program = &args[0];
    #[cfg(windows)]
    let program = {
        // Batch wrappers require a command interpreter. Use the editor executable
        // so even paths containing &, %, ! and quotes remain ordinary arguments.
        let candidate = std::path::PathBuf::from(program);
        let candidate = match candidate.extension().and_then(|s| s.to_str()) {
            Some(ext) if ext.eq_ignore_ascii_case("exe") || ext.eq_ignore_ascii_case("com") => candidate,
            None => candidate.with_extension("exe"),
            _ => return Err("Choose the editor .exe (for VS Code, Code.exe), rather than a .cmd or .bat wrapper.".into()),
        };
        if candidate.is_absolute() {
            candidate
        } else if candidate.components().count() == 1 {
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .filter(|dir| dir.is_absolute())
                .map(|dir| dir.join(&candidate)).find(|file| file.is_file())
                .ok_or("Editor executable not found on PATH. Select its full .exe path in App Settings.")?
        } else {
            return Err("Use an absolute editor executable path.".into());
        }
    };
    let mut command = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
        .args(&args[1..])
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not open editor: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn editor_receives_metacharacter_path_as_one_literal_argument() {
        let root = tempfile::tempdir().unwrap();
        let script = root.path().join("editor.sh");
        let output = root.path().join("received");
        std::fs::write(&script, "printf '%s' \"$2\" > \"$1\"\n").unwrap();
        let path = root.path().join("repo & $(touch marker)");
        open_editor(
            &path,
            &format!("/bin/sh \"{}\" \"{}\"", script.display(), output.display()),
        )
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !output.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            std::fs::read_to_string(output).unwrap(),
            path.to_string_lossy()
        );
    }
    #[test]
    fn provider_links_reject_non_web_handlers_and_credentials() {
        for input in [
            "file:///tmp/payload.exe",
            "javascript:alert(1)",
            "data:text/html,test",
            "ms-excel:payload",
            "https://user:secret@example.com",
            "https://example.com\n&calc",
            " https://example.com",
        ] {
            assert!(web_url(input).is_err(), "{input}");
        }
        let url = web_url("https://example.com/review?q=a&other=%22%26calc.exe").unwrap();
        assert_eq!(url.host_str(), Some("example.com"));
    }
    #[test]
    fn editor_parser_preserves_paths_and_does_not_expand_shell_syntax() {
        assert_eq!(
            editor_args(r#""C:\Program Files\Editor\editor.exe" --wait "$(touch marker)""#)
                .unwrap(),
            [
                r"C:\Program Files\Editor\editor.exe",
                "--wait",
                "$(touch marker)"
            ]
        );
        assert!(editor_args("\"unfinished").is_err());
        assert!(editor_args("editor\nnext").is_err());
    }
}
