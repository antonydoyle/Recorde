//! The installed Git CLI is separate from the bundled libgit2 library.
//! Probes run only on workers/operation threads; frames read an in-memory snapshot.
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

/// Worktree safety checks use `worktree list --porcelain -z`, and merge-preserving
/// history uses `log --remerge-diff`, both introduced in Git 2.36. This also covers
/// the older pathspec-from-file, end-of-options, sparse-checkout and rebase flags.
/// See https://github.com/git/git/blob/master/Documentation/RelNotes/2.36.0.adoc.
pub const MINIMUM: (u32, u32, u32) = (2, 36, 0);
pub const REQUIREMENTS: &str = "Git 2.36 or newer is required for safe NUL-delimited worktree checks and merge-preserving history. Install or update Git, then choose Retry Git check in Settings > Git.";
const SELECTED_ENV: &str = "RECORDE_GIT_EXECUTABLE";
pub(crate) const COMMAND_MARKER: &str = "RECORDE_SYSTEM_GIT_COMMAND";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Readiness {
    pub executable: Option<PathBuf>,
    pub version: String,
    pub problem: Option<String>,
    pub lfs_version: Option<String>,
    pub lfs_problem: Option<String>,
}

impl Readiness {
    pub fn summary(&self) -> String {
        self.problem
            .clone()
            .unwrap_or_else(|| format!("{} — ready", self.version))
    }

    pub fn require_ready(&self) -> Result<(), String> {
        self.problem.clone().map_or(Ok(()), Err)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Key {
    selection: String,
    environment: Vec<Option<OsString>>,
    executable: Option<PathBuf>,
    // Include configuration files and LFS: replacing either must invalidate a probe.
    files: Vec<(PathBuf, Option<(u64, SystemTime)>)>,
}

#[derive(Default)]
struct Cache {
    configured: Option<String>,
    generation: u64,
    entry: Option<(Key, Readiness)>,
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn lock() -> std::sync::MutexGuard<'static, Cache> {
    cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Empty selects PATH. Credential-helper children inherit the exact parent CLI.
pub fn configure(path: &str) {
    let path = path.trim().to_owned();
    let mut cache = lock();
    if cache.configured.as_ref() != Some(&path) {
        cache.configured = Some(path);
        cache.generation += 1;
        cache.entry = None;
    }
}

pub fn invalidate() {
    let mut cache = lock();
    cache.generation += 1;
    cache.entry = None;
}

/// No filesystem access or subprocesses; safe to reuse from Help and errors.
pub fn snapshot() -> Option<Readiness> {
    lock().entry.as_ref().map(|(_, value)| value.clone())
}

fn selection() -> String {
    lock()
        .configured
        .clone()
        .unwrap_or_else(|| std::env::var(SELECTED_ENV).unwrap_or_default())
}

fn executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return path
            .metadata()
            .is_ok_and(|m| m.permissions().mode() & 0o111 != 0);
    }
    #[cfg(not(unix))]
    true
}

/// Resolve once to an absolute path before spawning: repository cwd cannot select
/// a different executable. Windows uses executable Git, never a shell alias.
fn resolve(selection: &str, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    if !selection.is_empty() {
        let path = PathBuf::from(selection);
        return path
            .is_absolute()
            .then_some(path)
            .filter(|p| executable_file(p));
    }
    let name = if cfg!(windows) { "git.exe" } else { "git" };
    std::env::split_paths(path?)
        .filter(|p| p.is_absolute())
        .find_map(|dir| {
            let candidate = dir.join(name);
            executable_file(&candidate).then_some(candidate)
        })
}

pub(crate) fn executable() -> Option<PathBuf> {
    resolve(&selection(), std::env::var_os("PATH").as_deref())
}

fn key() -> Key {
    let selection = selection();
    let environment: Vec<_> = [
        "PATH",
        "HOME",
        "USERPROFILE",
        "XDG_CONFIG_HOME",
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_SYSTEM",
        "GIT_CONFIG_NOSYSTEM",
        "GIT_EXEC_PATH",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
    ]
    .iter()
    .map(std::env::var_os)
    .collect();
    let executable = resolve(&selection, environment[0].as_deref());
    let mut files = Vec::new();
    let mut add = |path: PathBuf| {
        let stamp = std::fs::metadata(&path)
            .ok()
            .and_then(|m| Some((m.len(), m.modified().ok()?)));
        files.push((path, stamp));
    };
    if let Some(path) = &executable {
        add(path.clone());
        add(path.with_file_name(if cfg!(windows) {
            "git-lfs.exe"
        } else {
            "git-lfs"
        }));
        // Git for Windows commonly keeps the implementation beside ../mingw64/bin.
        if let Some(parent) = path.parent().and_then(Path::parent) {
            add(parent.join("mingw64/bin/git-lfs.exe"));
        }
    }
    if let Some(path) = environment[0].as_deref() {
        for dir in std::env::split_paths(path).filter(|p| p.is_absolute()) {
            add(dir.join(if cfg!(windows) {
                "git-lfs.exe"
            } else {
                "git-lfs"
            }));
        }
    }
    for name in ["GIT_CONFIG_GLOBAL", "GIT_CONFIG_SYSTEM"] {
        if let Some(path) = std::env::var_os(name) {
            add(path.into());
        }
    }
    if let Some(home) = dirs::home_dir() {
        add(home.join(".gitconfig"));
    }
    if let Some(config) = std::env::var_os("XDG_CONFIG_HOME") {
        add(PathBuf::from(config).join("git/config"));
    } else if let Some(home) = dirs::home_dir() {
        add(home.join(".config/git/config"));
    }
    Key {
        selection,
        environment,
        executable,
        files,
    }
}

/// Accept platform/vendor suffixes, preserving the complete version for display.
fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let token = text
        .trim()
        .strip_prefix("git version ")?
        .split_whitespace()
        .next()?;
    let mut pieces = token.split('.');
    let major = pieces.next()?.parse().ok()?;
    let minor = pieces.next()?.parse().ok()?;
    let patch: String = pieces
        .next()?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    Some((major, minor, patch.parse().ok()?))
}

fn probe(path: Option<&Path>) -> Readiness {
    let mut value = Readiness {
        executable: path.map(Path::to_path_buf),
        ..Default::default()
    };
    let Some(path) = path else {
        value.problem = Some("System Git was not found. Install Git or choose its absolute executable path in Settings > Git, then choose Retry Git check. Repository browsing uses bundled libgit2.".into());
        return value;
    };
    let run = |args: &[&str]| {
        let mut command = super::process::raw_background_command(path);
        command.args(args).env("GIT_TERMINAL_PROMPT", "0");
        super::process::run(
            &mut command,
            &std::sync::atomic::AtomicBool::new(false),
            Duration::from_secs(5),
        )
    };
    match run(&["--version"]) {
        Ok(output) if output.status.success() => {
            value.version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            value.problem = match parse_version(&value.version) {
                Some(version) if version >= MINIMUM => None,
                Some(_) => Some(format!("System Git at {} reports {}. {REQUIREMENTS}", path.display(), value.version)),
                None => Some(format!("System Git at {} returned an unrecognised version. {REQUIREMENTS}", path.display())),
            };
        }
        _ => value.problem = Some(format!("System Git at {} could not be run. Check this executable, then choose Retry Git check in Settings > Git.", path.display())),
    }
    // LFS is optional and is never folded into the core Git failure.
    if !value.version.is_empty() {
        match run(&["lfs", "version"]) {
            Ok(output) if output.status.success() => {
                let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                if version.starts_with("git-lfs/") { value.lfs_version = Some(version); }
                else { value.lfs_problem = Some("Git LFS returned an unrecognised version. Install Git LFS for LFS repositories, then retry.".into()); }
            }
            _ => value.lfs_problem = Some("Git LFS is unavailable. Install Git LFS separately for LFS repositories, then retry.".into()),
        }
    }
    value
}

/// Serialized probes coalesce startup, retry and operation requests. A changed
/// selection or replaced executable cannot publish a stale in-flight result.
pub fn check() -> Readiness {
    static PROBE: Mutex<()> = Mutex::new(());
    let _probe = PROBE.lock().unwrap_or_else(|p| p.into_inner());
    for _ in 0..3 {
        let generation = lock().generation;
        let input = key();
        if let Some((old, value)) = &lock().entry {
            if old == &input {
                return value.clone();
            }
        }
        let value = probe(input.executable.as_deref());
        if generation != lock().generation || input != key() {
            continue;
        }
        let mut cache = lock();
        if generation != cache.generation {
            continue;
        }
        cache.entry = Some((input, value.clone()));
        return value;
    }
    Readiness {
        problem: Some("System Git selection changed repeatedly during its check. Choose Retry Git check in Settings > Git.".into()),
        ..Default::default()
    }
}

/// Owned by the app; polling performs only channel/memory/time operations.
pub struct Monitor {
    pending: Option<std::sync::mpsc::Receiver<Readiness>>,
    next_check: std::time::Instant,
    pub value: Option<Readiness>,
}

impl Default for Monitor {
    fn default() -> Self {
        Self {
            pending: None,
            next_check: std::time::Instant::now(),
            value: None,
        }
    }
}

impl Monitor {
    pub fn retry(&mut self) {
        invalidate();
        self.value = None;
        self.next_check = std::time::Instant::now();
    }

    pub fn checking(&self) -> bool {
        self.pending.is_some() || self.value.is_none()
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        // Operation workers can discover a replacement/failure between timer ticks.
        // Reuse that same result immediately without doing work on the UI thread.
        self.value = snapshot();
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(_) => {
                    self.value = snapshot();
                    self.pending = None;
                    if self.value.is_none() {
                        self.next_check = std::time::Instant::now();
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
            }
        }
        if self.pending.is_none() && std::time::Instant::now() >= self.next_check {
            let (tx, rx) = std::sync::mpsc::channel();
            self.pending = Some(rx);
            self.next_check = std::time::Instant::now() + Duration::from_secs(30);
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(check());
                ctx.request_repaint();
            });
        }
        ctx.request_repaint_after(if self.pending.is_some() {
            Duration::from_millis(100)
        } else {
            self.next_check
                .saturating_duration_since(std::time::Instant::now())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_platform_vendor_versions_and_rejects_untrusted_formats() {
        for text in [
            "git version 2.36.0",
            "git version 2.47.1.windows.2",
            "git version 2.39.5 (Apple Git-154)",
            "git version 2.43.0-vendor",
        ] {
            assert!(parse_version(text).unwrap() >= MINIMUM, "{text}");
        }
        assert_eq!(parse_version("git version 2.9.5"), Some((2, 9, 5)));
        for text in [
            "git version broken",
            "git version 2.36",
            "git-lfs/3.0.0",
            "2.36.0",
        ] {
            assert_eq!(parse_version(text), None);
        }
    }

    #[test]
    fn missing_git_has_actionable_error_without_spawn() {
        let value = probe(None);
        assert!(value
            .require_ready()
            .unwrap_err()
            .contains("Retry Git check"));
        assert!(value.lfs_version.is_none());
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve("", Some(dir.path().as_os_str())).is_none());
        assert!(resolve("relative/git", None).is_none());
    }

    fn fixture(path: &Path, version: &str, lfs: bool, marker: &Path) {
        let probes = marker.with_extension("probes");
        #[cfg(windows)]
        let text = format!(
            "@echo off\r\necho probe>>\"{}\"\r\nif \"%~1\"==\"--version\" goto version\r\nif \"%~1\"==\"lfs\" goto lfs\r\necho ran %*>>\"{}\"\r\nexit /b 0\r\n:version\r\necho git version {version}\r\nexit /b 0\r\n:lfs\r\n{}\r\n", probes.display(), marker.display(),
            if lfs { "echo git-lfs/3.7.0 (fixture)\r\nexit /b 0" } else { "exit /b 1" }
        );
        #[cfg(unix)]
        let text = format!(
            "#!/bin/sh\necho probe >> '{}'\ncase \"$1\" in\n--version) echo 'git version {version}';;\nlfs) {};;\n*) echo \"ran $*\" >> '{}';;\nesac\n",
            probes.display(), if lfs { "echo 'git-lfs/3.7.0 (fixture)'" } else { "exit 1" }, marker.display()
        );
        std::fs::write(path, text).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// One fixture owns global selection throughout; never edits PATH or user config.
    #[test]
    fn selected_executable_old_vendor_lfs_replacement_and_retry() {
        struct Restore(Option<String>);
        impl Drop for Restore {
            fn drop(&mut self) {
                let mut cache = lock();
                cache.configured = self.0.take();
                cache.generation += 1;
                cache.entry = None;
            }
        }
        let _restore = Restore(lock().configured.clone());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(if cfg!(windows) {
            "fixture git.cmd"
        } else {
            "fixture git"
        });
        let marker = dir.path().join("operations");
        configure(path.to_str().unwrap());
        assert!(check().problem.unwrap().contains("not found"));
        // Installing Git after a failed check invalidates the cached missing result.
        fixture(&path, "2.35.9.vendor", false, &marker);
        let old = check();
        assert!(old.problem.unwrap().contains("2.36"));
        let probes = marker.with_extension("probes");
        let count = std::fs::read_to_string(&probes).unwrap();
        check();
        assert_eq!(
            std::fs::read_to_string(&probes).unwrap(),
            count,
            "cached check must not run another probe"
        );
        let repo_dir = dir.path().join("repository");
        let repo = git2::Repository::init(&repo_dir).unwrap();
        // The real command entry point refuses before invoking an old executable.
        let error = super::super::process::git(
            &repo_dir,
            &["status"],
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(error.user_message().contains("Retry Git check"));
        assert!(!marker.exists());
        // Recovery must not write objects/refs before the same readiness refusal.
        assert!(super::super::recovery::capture(&repo_dir, "old Git fixture").is_err());
        assert!(!repo.path().join("recorde/recovery").exists());
        assert!(repo.references().unwrap().next().is_none());

        fixture(&path, "2.47.1.windows.2", false, &marker);
        let ready = check();
        assert!(ready.problem.is_none(), "{ready:?}");
        assert_eq!(ready.executable.as_deref(), Some(path.as_path()));
        assert!(ready.lfs_version.is_none());
        assert!(ready.lfs_problem.unwrap().contains("separately"));
        let mut command = super::super::process::command(&repo_dir);
        assert_eq!(command.get_program(), path.as_os_str());
        command.arg("status");
        super::super::process::run(
            &mut command,
            &std::sync::atomic::AtomicBool::new(false),
            Duration::from_secs(5),
        )
        .unwrap();
        assert!(
            marker.exists(),
            "operation must use the selected executable"
        );

        fixture(&path, "2.48.0-vendor-build", true, &marker);
        let replaced = check();
        assert!(replaced.version.contains("2.48.0"));
        assert!(replaced.lfs_version.is_some());
        let mut monitor = Monitor::default();
        monitor.value = Some(replaced);
        monitor.retry();
        assert!(snapshot().is_none());
        assert!(monitor.checking());
        assert!(check().lfs_version.is_some());

        let other = dir.path().join(if cfg!(windows) {
            "replacement.cmd"
        } else {
            "replacement"
        });
        fixture(&other, "2.49.0", true, &marker);
        configure(other.to_str().unwrap());
        assert!(snapshot().is_none());
        assert_eq!(check().executable.as_deref(), Some(other.as_path()));
        // A command prepared with the previous selection cannot run accidentally.
        assert!(super::super::process::run(
            &mut command,
            &std::sync::atomic::AtomicBool::new(false),
            Duration::from_secs(5)
        )
        .unwrap_err()
        .user_message()
        .contains("changed"));
        assert!(git2::Repository::open(&repo_dir).is_ok());

        // An in-flight worker must discard the previous selection's result.
        let started = dir.path().join("started");
        let release = dir.path().join("release");
        let text = std::fs::read_to_string(&path).unwrap();
        #[cfg(windows)]
        let text = text.replace(
            ":version\r\n",
            &format!(
                ":version\r\necho started>\"{}\"\r\n:wait\r\nif not exist \"{}\" goto wait\r\n",
                started.display(),
                release.display()
            ),
        );
        #[cfg(unix)]
        let text = text.replace(
            "--version)",
            &format!(
                "--version) echo started > '{}'; while [ ! -e '{}' ]; do :; done;",
                started.display(),
                release.display()
            ),
        );
        std::fs::write(&path, text).unwrap();
        configure(path.to_str().unwrap());
        let worker = std::thread::spawn(check);
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !started.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let saw_started = started.exists();
        configure(other.to_str().unwrap());
        std::fs::write(&release, "release").unwrap();
        let result = worker.join().unwrap();
        assert!(saw_started, "fixture must reach its blocked version probe");
        assert_eq!(result.executable.as_deref(), Some(other.as_path()));
        assert_eq!(
            snapshot().unwrap().executable.as_deref(),
            Some(other.as_path())
        );

        // Exercise the public diff-menu click entry while a version probe is
        // deliberately blocked. The click must return without waiting for Git.
        std::fs::remove_file(&marker).unwrap();
        std::fs::remove_file(&started).unwrap();
        std::fs::remove_file(&release).unwrap();
        configure(path.to_str().unwrap());
        repo.config()
            .unwrap()
            .set_str("diff.tool", "fixture")
            .unwrap();
        let state = crate::state::new_shared_state();
        state.lock().unwrap().repo_path = Some(repo_dir.clone());
        let clicked = std::time::Instant::now();
        crate::external::open_diff_tool(
            &repo_dir,
            crate::external::DiffToolArgs::Unstaged { path: "file" },
            state.clone(),
        )
        .unwrap();
        let elapsed = clicked.elapsed();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !started.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let reached_probe = started.exists();
        let launched_early = marker.exists();
        std::fs::write(&release, "release").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            elapsed < Duration::from_millis(250),
            "click waited {elapsed:?} for the probe"
        );
        assert!(
            reached_probe,
            "launch worker must perform readiness off the click path"
        );
        assert!(
            !launched_early,
            "tool must not launch before readiness succeeds"
        );
        assert!(
            marker.exists(),
            "ready Git must launch the requested diff tool"
        );
        assert!(state.lock().unwrap().error.is_none());

        // An asynchronous readiness refusal reaches the existing visible error
        // state, without launching the tool or waiting on the click thread.
        std::fs::remove_file(&marker).unwrap();
        fixture(&path, "2.35.0", false, &marker);
        invalidate();
        let clicked = std::time::Instant::now();
        crate::external::open_diff_tool(
            &repo_dir,
            crate::external::DiffToolArgs::Staged { path: "file" },
            state.clone(),
        )
        .unwrap();
        assert!(clicked.elapsed() < Duration::from_millis(250));
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while state.lock().unwrap().error.is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut state = state.lock().unwrap();
        assert!(state
            .error
            .as_ref()
            .is_some_and(|error| error.contains("Retry Git check")));
        state.take_pending_error();
        assert_eq!(
            state.errors.back().unwrap().kind,
            crate::ui::error_notice::ErrorKind::GitReadiness
        );
        // A refresh after the first launch may still run ordinary Git
        // commands on a slow machine; only a tool launch must not happen.
        let operations = std::fs::read_to_string(&marker).unwrap_or_default();
        assert!(
            !operations.contains("difftool"),
            "refused readiness must not launch the tool: {operations:?}"
        );
    }
}
