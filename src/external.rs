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

fn terminal_plans(
    configured: &str,
    repo: &std::path::Path,
) -> Result<Vec<Vec<std::ffi::OsString>>, String> {
    if !configured.trim().is_empty() {
        return Ok(vec![editor_args(configured)?
            .into_iter()
            .map(Into::into)
            .collect()]);
    }
    #[cfg(target_os = "linux")]
    let plans = [
        "xdg-terminal-exec",
        "xterm",
        "konsole",
        "gnome-terminal",
        "xfce4-terminal",
        "alacritty",
        "kitty",
    ]
    .into_iter()
    .map(|program| vec![program.into()])
    .collect();
    #[cfg(target_os = "macos")]
    let plans = vec![vec![
        "open".into(),
        "-a".into(),
        "Terminal".into(),
        repo.into(),
    ]];
    #[cfg(target_os = "windows")]
    let plans = vec![
        vec!["wt.exe".into(), "-d".into(), repo.into()],
        vec!["powershell.exe".into(), "-NoExit".into()],
        vec!["cmd.exe".into(), "/K".into()],
    ];
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    let plans = Vec::new();
    let _ = repo;
    Ok(plans)
}

pub(crate) fn open_terminal(repo: &std::path::Path, configured: &str) -> Result<(), String> {
    let plans = terminal_plans(configured, repo)?;
    let mut last_error = None;
    for args in plans {
        #[cfg(windows)]
        let program = if !configured.trim().is_empty() {
            let candidate = std::path::PathBuf::from(&args[0]);
            let candidate = match candidate.extension().and_then(|ext| ext.to_str()) {
                Some(ext) if ext.eq_ignore_ascii_case("exe") || ext.eq_ignore_ascii_case("com") => {
                    candidate
                }
                None => candidate.with_extension("exe"),
                _ => {
                    return Err(
                        "Choose a terminal .exe or .com, not a script or batch file.".into(),
                    );
                }
            };
            if candidate.is_absolute() {
                candidate
            } else if candidate.components().count() == 1 {
                std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                    .filter(|dir| dir.is_absolute())
                    .map(|dir| dir.join(&candidate))
                    .find(|file| file.is_file())
                    .ok_or("Terminal executable not found on PATH.")?
            } else {
                return Err("Use an absolute terminal executable path.".into());
            }
        } else {
            std::path::PathBuf::from(&args[0])
        };
        #[cfg(not(windows))]
        let program = &args[0];
        let mut command = std::process::Command::new(program);
        command.args(&args[1..]).current_dir(repo);
        // Console fallbacks need the new console's standard handles for interaction.
        if !cfg!(windows) || !configured.trim().is_empty() {
            command
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x00000010); // CREATE_NEW_CONSOLE
        }
        match command.spawn() {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Ok(());
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(match last_error {
        Some(error) => format!("Could not open terminal: {error}"),
        None => "No terminal launcher is available on this platform.".into(),
    })
}

/// The comparison shown by an external Git diff tool.
pub enum DiffToolArgs<'a> {
    Unstaged { path: &'a str },
    Staged { path: &'a str },
    Commit { path: &'a str, oid: &'a str },
}

fn git_tool_args(verb: &str, tool: Option<&str>, extra: &[String], path: &str) -> Vec<String> {
    let mut args = vec![
        "--literal-pathspecs".into(),
        verb.into(),
        "--no-prompt".into(),
    ];
    if let Some(tool) = tool {
        args.push(format!("--tool={tool}"));
    }
    args.extend_from_slice(extra);
    args.extend(["--".into(), path.into()]);
    args
}

pub(crate) fn configured_git_tool(
    repo: &git2::Repository,
    configured: &str,
    kind: &str,
) -> Result<Option<String>, String> {
    if !configured.trim().is_empty() {
        return Ok(Some(configured.trim().to_owned()));
    }
    let config = repo.config().map_err(|e| e.to_string())?;
    for key in [format!("{kind}.tool"), format!("{kind}.guitool")] {
        if let Ok(value) = config.get_string(&key) {
            if !value.trim().is_empty() {
                return Ok(Some(value.trim().to_owned()));
            }
        }
    }
    Ok(None)
}

/// Start a user-owned Git tool without a supervised deadline or mutation lock.
/// `background_command` only hides Git's console; it adds no job or kill-on-drop.
pub(crate) fn spawn_git_tool(
    repo: &std::path::Path,
    args: &[String],
) -> Result<std::process::Child, String> {
    let mut command = crate::git::process::background_command("git");
    command
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("RECORDE_CREDENTIAL_REPO")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command
        .spawn()
        .map_err(|e| format!("Could not launch Git tool: {e}"))
}

fn diff_tool_args(tool: Option<&str>, extra: &[String], path: &str) -> Vec<String> {
    git_tool_args("difftool", tool, extra, path)
}

fn commit_diff_range(oid: &str, parent: Option<&str>) -> Vec<String> {
    vec![
        parent
            .unwrap_or("4b825dc642cb6eb9a060e54bf8d69288fbee4904")
            .into(),
        oid.into(),
    ]
}

/// Launch a diff tool for a worktree, staged, or commit file.
pub fn open_diff_tool(repo: &std::path::Path, request: DiffToolArgs<'_>) -> Result<(), String> {
    let path = match &request {
        DiffToolArgs::Unstaged { path }
        | DiffToolArgs::Staged { path }
        | DiffToolArgs::Commit { path, .. } => *path,
    };
    crate::git::safe_path::relative(path)?;
    let repository = git2::Repository::open(repo).map_err(|e| e.to_string())?;
    let config = crate::config::Config::load();
    let tool = configured_git_tool(&repository, &config.diff_tool, "diff")?
        .ok_or("Set a diff tool in App Settings or configure diff.tool in Git.")?;
    let (path, extra) = match request {
        DiffToolArgs::Unstaged { path } => (path, vec![]),
        DiffToolArgs::Staged { path } => (path, vec!["--cached".into()]),
        DiffToolArgs::Commit { path, oid } => {
            let oid = git2::Oid::from_str(oid).map_err(|e| e.to_string())?;
            let commit = repository.find_commit(oid).map_err(|e| e.to_string())?;
            let parent = if commit.parent_count() == 0 {
                repository
                    .treebuilder(None)
                    .and_then(|tree| tree.write())
                    .map_err(|e| e.to_string())?;
                None
            } else {
                Some(commit.parent_id(0).map_err(|e| e.to_string())?.to_string())
            };
            (path, commit_diff_range(&oid.to_string(), parent.as_deref()))
        }
    };
    let git_config = repository.config().map_err(|e| e.to_string())?;
    let explicit = if !config.diff_tool.trim().is_empty()
        || !git_config
            .get_string("diff.tool")
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
    {
        Some(tool.as_str())
    } else {
        None
    };
    let mut child = spawn_git_tool(repo, &diff_tool_args(explicit, &extra, path))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

pub(crate) fn merge_tool_args(tool: Option<&str>, path: &str) -> Vec<String> {
    git_tool_args("mergetool", tool, &[], path)
}

/// The editor command for `open_editor`: App Settings first, then `VISUAL`/`EDITOR`.
/// Windows falls back to Notepad, launched directly rather than through a shell.
pub fn editor_command(configured: &str) -> Result<String, String> {
    resolve_editor(configured, |name| std::env::var(name).ok())
}

fn resolve_editor(
    configured: &str,
    env: impl Fn(&str) -> Option<String>,
) -> Result<String, String> {
    let fallback = || {
        ["VISUAL", "EDITOR"]
            .into_iter()
            .filter_map(&env)
            .find(|value| !value.trim().is_empty())
    };
    match (configured.trim(), fallback()) {
        ("", Some(editor)) => Ok(editor.trim().to_string()),
        ("", None) if cfg!(windows) => Ok("notepad.exe".into()),
        ("", None) => Err("Set a default editor in App Settings first.".into()),
        (editor, _) => Ok(editor.to_string()),
    }
}

/// Open the system file manager at `path`, selecting it where the platform supports that.
pub fn reveal_path(path: &std::path::Path) -> Result<(), String> {
    let (program, argument) = reveal_plan(path);
    let mut command = std::process::Command::new(program);
    #[cfg(windows)]
    {
        // Explorer requires `/select,"path"` as one token; Windows paths cannot contain quotes.
        use std::os::windows::process::CommandExt;
        command.raw_arg(argument);
    }
    #[cfg(not(windows))]
    command.args(argument);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not open the file manager: {e}"))
}

#[cfg(windows)]
fn reveal_plan(path: &std::path::Path) -> (&'static str, String) {
    ("explorer.exe", format!("/select,\"{}\"", path.display()))
}

#[cfg(target_os = "macos")]
fn reveal_plan(path: &std::path::Path) -> (&'static str, Vec<std::ffi::OsString>) {
    ("open", vec!["-R".into(), path.into()])
}

#[cfg(all(unix, not(target_os = "macos")))]
fn reveal_plan(path: &std::path::Path) -> (&'static str, Vec<std::ffi::OsString>) {
    // The freedesktop opener has no portable "select"; show the containing folder.
    let folder = path.parent().filter(|_| !path.is_dir()).unwrap_or(path);
    ("xdg-open", vec![folder.into()])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_plans_keep_arguments_literal_and_provide_fallbacks() {
        let repo = std::path::Path::new("x&calc %PATH% ^ $(touch m)");
        let configured =
            terminal_plans("term --option 'x&calc %PATH% ^ $(touch m)'", repo).unwrap();
        assert_eq!(
            configured[0],
            ["term", "--option", "x&calc %PATH% ^ $(touch m)"].map(std::ffi::OsString::from)
        );
        let fallback = terminal_plans("", repo).unwrap();
        assert!(!fallback.is_empty());
        #[cfg(windows)]
        assert_eq!(
            fallback[0],
            [std::ffi::OsString::from("wt.exe"), "-d".into(), repo.into()]
        );
    }

    #[test]
    fn git_tool_arguments_preserve_literal_paths_and_comparisons() {
        let path = "x&calc %PATH% ^ $(touch m)";
        let base =
            |extra: &[&str]| -> Vec<String> { extra.iter().map(|s| (*s).to_owned()).collect() };
        assert_eq!(
            diff_tool_args(None, &[], path),
            base(&["--literal-pathspecs", "difftool", "--no-prompt", "--", path])
        );
        assert_eq!(
            diff_tool_args(Some("meld"), &["--cached".into()], path),
            base(&[
                "--literal-pathspecs",
                "difftool",
                "--no-prompt",
                "--tool=meld",
                "--cached",
                "--",
                path,
            ])
        );
        for (parent, expected) in [
            (Some("parent"), "parent"),
            (None, "4b825dc642cb6eb9a060e54bf8d69288fbee4904"),
        ] {
            assert_eq!(
                diff_tool_args(None, &commit_diff_range("commit", parent), path),
                base(&[
                    "--literal-pathspecs",
                    "difftool",
                    "--no-prompt",
                    expected,
                    "commit",
                    "--",
                    path,
                ])
            );
        }
        assert_eq!(
            merge_tool_args(None, path),
            base(&[
                "--literal-pathspecs",
                "mergetool",
                "--no-prompt",
                "--",
                path
            ])
        );
        assert_eq!(
            merge_tool_args(Some("meld"), path),
            base(&[
                "--literal-pathspecs",
                "mergetool",
                "--no-prompt",
                "--tool=meld",
                "--",
                path
            ])
        );
    }
    #[test]
    fn editor_command_prefers_settings_then_environment() {
        let env = |name: &str| match name {
            "VISUAL" => Some("  ".to_string()),
            "EDITOR" => Some(" gvim -f ".to_string()),
            _ => None,
        };
        assert_eq!(resolve_editor(" code --wait ", env).unwrap(), "code --wait");
        assert_eq!(resolve_editor("", env).unwrap(), "gvim -f");
        let none = resolve_editor("", |_| None);
        if cfg!(windows) {
            assert_eq!(none.unwrap(), "notepad.exe");
        } else {
            assert!(none.is_err());
        }
    }
    #[test]
    fn reveal_passes_metacharacter_paths_literally() {
        let path = std::path::Path::new("repo").join("x&calc %PATH% ^.txt");
        let (program, argument) = reveal_plan(&path);
        #[cfg(windows)]
        {
            assert_eq!(program, "explorer.exe");
            assert_eq!(argument, format!("/select,\"{}\"", path.display()));
        }
        #[cfg(target_os = "macos")]
        {
            assert_eq!(program, "open");
            assert_eq!(argument, [std::ffi::OsString::from("-R"), path.into()]);
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            assert_eq!(program, "xdg-open");
            assert_eq!(argument, [std::ffi::OsString::from("repo")]);
        }
    }
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
