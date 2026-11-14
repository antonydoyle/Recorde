//! Local diagnostics deliberately do not consume `log::Record`, error strings,
//! panic payloads, repository contents, credentials or remote addresses.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{mpsc, Arc, OnceLock};
use std::time::Duration;

pub const LOG_BYTES: u64 = 64 * 1024;
pub const LOG_FILES: usize = 3;
const EVENT_BYTES: usize = 256;
static STATUS: AtomicU8 = AtomicU8::new(0);
static DROPPED: AtomicU64 = AtomicU64::new(0);
static PANICS: AtomicU64 = AtomicU64::new(0);
static EVENTS: OnceLock<mpsc::SyncSender<Event>> = OnceLock::new();

#[derive(Clone, Copy)]
pub enum Event {
    Started,
    Stopped,
    GitReady,
    GitUnavailable,
    ReportPreviewed,
    ReportSaved,
    OperationFailed(crate::ui::error_notice::ErrorKind),
    RustPanic,
}

impl Event {
    fn code(self) -> &'static str {
        use crate::ui::error_notice::ErrorKind as K;
        match self {
            Self::Started => "app_started",
            Self::Stopped => "app_stopped",
            Self::GitReady => "git_ready",
            Self::GitUnavailable => "git_unavailable",
            Self::ReportPreviewed => "support_previewed",
            Self::ReportSaved => "support_saved",
            Self::RustPanic => "rust_panic_payload_omitted",
            Self::OperationFailed(kind) => match kind {
                K::GitReadiness => "failure_git_readiness",
                K::WorkflowScope => "failure_workflow_permission",
                K::SignIn => "failure_sign_in",
                K::HostKey => "failure_host_key",
                K::PushRejected => "failure_push_rejected",
                K::LocalChanges => "failure_local_changes",
                K::Conflict => "failure_conflict",
                K::Hook => "failure_hook",
                K::Network => "failure_network",
                K::Other => "failure_other",
            },
        }
    }
    fn line(self) -> String {
        format!("{} {}\n", crate::update::now(), self.code())
    }
}

/// Never waits for disk or queue capacity. Full queues drop events, counted in the
/// report. Arbitrary application/dependency log messages are not a logging input.
pub fn record(event: Event) {
    if let Some(sender) = EVENTS.get() {
        if sender.try_send(event).is_err() {
            DROPPED.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Panic hooks touch only an atomic counter: no payload, location, probe or lock.
/// The writer records this best effort if the process remains alive long enough.
pub fn rust_panic() {
    PANICS.fetch_add(1, Ordering::Relaxed);
}

pub fn log_directory() -> Option<PathBuf> {
    crate::app_dirs::data_dir().map(|path| path.join("recorde/diagnostics"))
}

#[derive(Debug, PartialEq, Eq)]
enum LogError {
    Unavailable,
    Busy,
}

fn regular_file(path: &Path) -> Result<bool, LogError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    return Err(LogError::Unavailable);
                }
            }
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        _ => Err(LogError::Unavailable),
    }
}

struct RotatingLog {
    root: PathBuf,
}
impl RotatingLog {
    fn new(root: PathBuf) -> Result<Self, LogError> {
        for ancestor in root.ancestors() {
            if let Ok(meta) = std::fs::symlink_metadata(ancestor) {
                if meta.file_type().is_symlink() {
                    return Err(LogError::Unavailable);
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 {
                        return Err(LogError::Unavailable);
                    }
                }
            }
        }
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&root).map_err(|_| LogError::Unavailable)?;
        Ok(Self { root })
    }

    fn file(&self, index: usize) -> PathBuf {
        self.root.join(format!("events-{index}.log"))
    }

    fn append(&mut self, event: Event) -> Result<(), LogError> {
        use fs2::FileExt;
        let line = event.line();
        if line.len() > EVENT_BYTES {
            return Err(LogError::Unavailable);
        }
        let lock_path = self.root.join("events.lock");
        let lock = if regular_file(&lock_path)? {
            std::fs::OpenOptions::new()
                .write(true)
                .open(&lock_path)
                .map_err(|_| LogError::Unavailable)?
        } else {
            crate::update::create_private_file(&lock_path).map_err(|_| LogError::Unavailable)?
        };
        lock.try_lock_exclusive().map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                LogError::Busy
            } else {
                LogError::Unavailable
            }
        })?;
        for index in 0..LOG_FILES {
            regular_file(&self.file(index))?;
        }
        let current = self.file(0);
        if std::fs::metadata(&current)
            .map(|meta| meta.len())
            .unwrap_or(0)
            + line.len() as u64
            > LOG_BYTES
        {
            if regular_file(&self.file(LOG_FILES - 1))? {
                std::fs::remove_file(self.file(LOG_FILES - 1))
                    .map_err(|_| LogError::Unavailable)?;
            }
            for index in (0..LOG_FILES - 1).rev() {
                if regular_file(&self.file(index))? {
                    std::fs::rename(self.file(index), self.file(index + 1))
                        .map_err(|_| LogError::Unavailable)?;
                }
            }
        }
        let mut file = if regular_file(&current)? {
            std::fs::OpenOptions::new()
                .append(true)
                .open(&current)
                .map_err(|_| LogError::Unavailable)?
        } else {
            crate::update::create_private_file(&current).map_err(|_| LogError::Unavailable)?
        };
        file.write_all(line.as_bytes())
            .map_err(|_| LogError::Unavailable)
    }
}

pub struct LoggingGuard {
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for LoggingGuard {
    fn drop(&mut self) {
        record(Event::Stopped);
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub fn start_logging() -> LoggingGuard {
    start_logging_with(log_directory)
}

fn start_logging_with(root: impl FnOnce() -> Option<PathBuf> + Send + 'static) -> LoggingGuard {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::sync_channel(128);
    if EVENTS.set(tx).is_err() {
        return LoggingGuard { stop, worker: None };
    }
    STATUS.store(1, Ordering::Relaxed);
    let worker_stop = stop.clone();
    let worker = std::thread::Builder::new()
        .name("local-diagnostics".into())
        .spawn(move || {
            let mut storage = root().and_then(|root| RotatingLog::new(root).ok());
            STATUS.store(if storage.is_some() { 2 } else { 3 }, Ordering::Relaxed);
            loop {
                let event = match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(event) => Some(event),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                for event in event
                    .into_iter()
                    .chain((PANICS.swap(0, Ordering::Relaxed) > 0).then_some(Event::RustPanic))
                {
                    match storage.as_mut().map(|log| log.append(event)) {
                        Some(Ok(())) => {
                            STATUS.store(2, Ordering::Relaxed);
                        }
                        Some(Err(LogError::Busy)) => {
                            DROPPED.fetch_add(1, Ordering::Relaxed);
                        }
                        _ => {
                            STATUS.store(3, Ordering::Relaxed);
                            DROPPED.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                if worker_stop.load(Ordering::Acquire)
                    && rx
                        .try_iter()
                        .map(|event| {
                            if let Some(log) = &mut storage {
                                let _ = log.append(event);
                            }
                        })
                        .count()
                        == 0
                {
                    break;
                }
            }
        })
        .ok();
    if worker.is_none() {
        STATUS.store(3, Ordering::Relaxed);
    }
    record(Event::Started);
    LoggingGuard { stop, worker }
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdaterState {
    NotRunning,
    Idle,
    CheckingOrDownloading,
    Ready,
    Applying,
    NeedsAttention,
}

#[derive(Clone)]
pub struct Report {
    text: String,
}
impl Report {
    pub fn text(&self) -> &str {
        &self.text
    }
}

fn numeric_version(raw: &str, prefix: &str) -> Option<String> {
    let raw = raw.trim().strip_prefix(prefix)?;
    let mut fields = raw.split('.');
    let major: u32 = fields.next()?.parse().ok()?;
    let minor: u32 = fields.next()?.parse().ok()?;
    let patch: String = fields
        .next()?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let patch: u32 = patch.parse().ok()?;
    Some(format!("{major}.{minor}.{patch}"))
}

fn git_location(path: Option<&Path>) -> &'static str {
    let Some(path) = path else {
        return "not found";
    };
    // Only canonical, well-known installation locations can cross the boundary.
    // Custom paths may include usernames/repository/file names: omit them entirely.
    match path.to_str() {
        Some("/usr/bin/git") => "/usr/bin/git",
        Some("/bin/git") => "/bin/git",
        Some("/usr/local/bin/git") => "/usr/local/bin/git",
        Some("/opt/homebrew/bin/git") => "/opt/homebrew/bin/git",
        Some(path)
            if path
                .replace('\\', "/")
                .eq_ignore_ascii_case("C:/Program Files/Git/cmd/git.exe") =>
        {
            "C:/Program Files/Git/cmd/git.exe"
        }
        Some(path)
            if path
                .replace('\\', "/")
                .eq_ignore_ascii_case("C:/Program Files/Git/bin/git.exe") =>
        {
            "C:/Program Files/Git/bin/git.exe"
        }
        _ => "[private executable path omitted]",
    }
}

fn build_report(git: &crate::git::readiness::Readiness, updater: UpdaterState) -> Report {
    #[derive(serde::Serialize)]
    struct SafeReport {
        schema: u8,
        recorde: &'static str,
        os: &'static str,
        arch: &'static str,
        build: &'static str,
        build_commit: &'static str,
        git_location: &'static str,
        git_version: Option<String>,
        git_ready: bool,
        cli_minimum: &'static str,
        cli_capabilities: &'static [&'static str],
        lfs_version: Option<String>,
        lfs_available: bool,
        libgit2: String,
        keyring_backend: &'static str,
        keyring_availability: &'static str,
        updater: UpdaterState,
        update_key_ids: Vec<String>,
        network: &'static str,
        local_logging: &'static str,
        dropped_events: u64,
        privacy: &'static str,
    }
    let (major, minor, patch) = git2::Version::get().libgit2_version();
    let commit = option_env!("RECORDE_BUILD_COMMIT")
        .filter(|value| {
            matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .unwrap_or("not embedded");
    let ready = git.problem.is_none() && numeric_version(&git.version, "git version ").is_some();
    let lfs = git
        .lfs_version
        .as_deref()
        .and_then(|raw| numeric_version(raw, "git-lfs/"));
    let key_ids = crate::update::keys()
        .map(|keys| {
            keys.keys
                .into_iter()
                .take(16)
                .filter_map(|key| {
                    (!key.key_id.is_empty()
                        && key.key_id.len() <= 64
                        && key
                            .key_id
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')))
                    .then_some(key.key_id)
                })
                .collect()
        })
        .unwrap_or_default();
    let report = SafeReport {
        schema: 1, recorde: env!("CARGO_PKG_VERSION"), os: std::env::consts::OS, arch: std::env::consts::ARCH,
        build: if cfg!(debug_assertions) { "debug" } else { "release" }, build_commit: commit,
        git_location: git_location(git.executable.as_deref()), git_version: numeric_version(&git.version, "git version "),
        git_ready: ready, cli_minimum: "2.36.0",
        cli_capabilities: if ready { &["nul_worktree_checks", "merge_preserving_history", "nul_pathspecs", "sparse_checkout", "commit_graph"] } else { &[] },
        lfs_available: lfs.is_some(), lfs_version: lfs, libgit2: format!("{major}.{minor}.{patch}"),
        keyring_backend: if cfg!(windows) { "windows_credential_store" } else if cfg!(target_os = "macos") { "apple_keychain" } else if cfg!(target_os = "linux") { "secret_service" } else { "unsupported" },
        keyring_availability: "not probed; credential values are never read for diagnostics",
        updater, update_key_ids: key_ids, network: "not checked; offline report; no upload",
        local_logging: match STATUS.load(Ordering::Relaxed) { 1 => "starting", 2 => "active: three 64KiB event files plus empty lock file", 3 => "unavailable; application continues without disk logs", _ => "not started" },
        dropped_events: DROPPED.load(Ordering::Relaxed),
        privacy: "Only allowlisted metadata. Private paths, raw Git output, errors, panic payloads, credentials, URLs, file contents and log contents are omitted.",
    };
    Report {
        text: crate::auth::redact::text(
            &serde_json::to_string_pretty(&report)
                .unwrap_or_else(|_| "{\"report\":\"unavailable\"}".into()),
        ),
    }
}

/// Explicit offline request; call from a worker or the CLI, never an egui frame.
pub fn collect(updater: UpdaterState) -> Report {
    let git = crate::git::readiness::check();
    record(if git.problem.is_none() {
        Event::GitReady
    } else {
        Event::GitUnavailable
    });
    record(Event::ReportPreviewed);
    build_report(&git, updater)
}

pub fn save(path: &Path, report: &Report) -> Result<(), &'static str> {
    let mut file = crate::update::create_private_file(path)
        .map_err(|_| "Cannot create the report. Choose a new filename in a writable folder.")?;
    file.write_all(report.text.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|_| "Cannot finish writing the report. Check available space and permissions.")?;
    record(Event::ReportSaved);
    Ok(())
}

pub fn cli(destination: Option<&Path>) -> Result<(), &'static str> {
    let config = crate::config::Config::load();
    crate::git::readiness::configure(&config.git_executable);
    let report = collect(UpdaterState::NotRunning);
    match destination {
        Some(path) => save(path, &report),
        None => std::io::stdout()
            .write_all(format!("{}\n", report.text()).as_bytes())
            .map_err(|_| "Cannot write the diagnostics preview."),
    }
}

enum PreviewResult {
    Collected(Report),
    Saved(Result<(), &'static str>),
}
#[derive(Default)]
pub struct SupportWindow {
    pub open: bool,
    report: Option<Report>,
    pending: Option<mpsc::Receiver<PreviewResult>>,
    message: Option<&'static str>,
}
impl SupportWindow {
    fn refresh(&mut self, updater: UpdaterState, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        self.message = None;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(PreviewResult::Collected(collect(updater)));
            ctx.request_repaint();
        });
    }
    pub fn show(&mut self, ctx: &egui::Context, updater: UpdaterState) {
        if !self.open {
            return;
        }
        if self.report.is_none() && self.pending.is_none() {
            self.refresh(updater, ctx);
        }
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(PreviewResult::Collected(report)) => {
                    self.report = Some(report);
                    self.pending = None;
                }
                Ok(PreviewResult::Saved(result)) => {
                    self.message = Some(
                        result
                            .err()
                            .unwrap_or("Report saved locally. Nothing was uploaded."),
                    );
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.message = Some("Support report worker stopped. Refresh to try again.");
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100))
                }
            }
        }
        let mut open = self.open;
        egui::Window::new("Offline support report").open(&mut open).default_size([760.0, 580.0]).show(ctx, |ui| {
            ui.label("Review the exact report below before copying or saving. No upload or network check is performed. Private paths and sensitive payloads are omitted.");
            if self.pending.is_some() { ui.spinner(); }
            if let Some(message) = self.message { ui.label(message); }
            ui.horizontal(|ui| {
                if ui.add_enabled(self.pending.is_none(), egui::Button::new("Refresh offline preview")).clicked() { self.refresh(updater, ctx); }
                if let Some(report) = &self.report {
                    if ui.button("Copy preview").clicked() { ctx.copy_text(report.text.clone()); }
                    if ui.add_enabled(self.pending.is_none(), egui::Button::new("Save preview…")).clicked() {
                        if let Some(path) = rfd::FileDialog::new().set_file_name("recorde-support.json").save_file() {
                            let report = report.clone(); let (tx, rx) = mpsc::channel(); self.pending = Some(rx);
                            let ctx = ctx.clone();
                            std::thread::spawn(move || { let _ = tx.send(PreviewResult::Saved(save(&path, &report))); ctx.request_repaint(); });
                        }
                    }
                }
            });
            if let Some(report) = &self.report {
                egui::ScrollArea::vertical().show(ui, |ui| { ui.add(egui::Label::new(egui::RichText::new(report.text()).monospace()).selectable(true).wrap()); });
            }
            ui.separator();
            ui.label("Local logs contain only event codes (three files, at most 64 KiB each). Busy/full queues drop events. Rust panic events are best effort and do not capture native crashes.");
            if ui.button("Open local log folder").clicked() {
                std::thread::spawn(|| { if let Some(path) = log_directory() { let _ = crate::external::reveal_path(&path); } });
            }
        });
        self.open = open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    #[ignore = "requires RECORDE_DIAGNOSTICS_CANDIDATE pointing to the candidate executable"]
    fn candidate_cli_is_offline_private_and_survives_unwritable_logs() {
        let candidate = std::env::var_os("RECORDE_DIAGNOSTICS_CANDIDATE")
            .expect("provide the built candidate executable");
        let root = tempfile::tempdir().unwrap();
        let profile = root.path().join("profile-private-user");
        let config_dir = profile.join("recorde");
        std::fs::create_dir_all(&config_dir).unwrap();
        let git = root.path().join("private-repository-git.cmd");
        const PRIVATE: &str = "seeded-candidate-secret-991 https://alice:password@host/private-repo?token=value C:\\Users\\PrivatePerson\\private-repo file-contents-payload";
        let script = format!("@echo off\r\nif \"%~1\"==\"--version\" goto version\r\nif \"%~1\"==\"lfs\" goto lfs\r\nexit /b 91\r\n:version\r\necho git version 2.54.0.windows.1 {PRIVATE}\r\nexit /b 0\r\n:lfs\r\necho git-lfs/3.7.0 {PRIVATE}\r\nexit /b 0\r\n");
        std::fs::write(&git, script).unwrap();
        std::fs::write(
            config_dir.join("config.toml"),
            format!("git_executable = '{}'\n", git.display()),
        )
        .unwrap();
        let run = |destination: Option<&Path>| {
            let mut command = crate::git::process::background_command(&candidate);
            command.arg("--diagnostics");
            if let Some(path) = destination {
                command.arg(path);
            }
            command
                .env("APPDATA", &profile)
                .env("LOCALAPPDATA", &profile)
                .env("RUST_LOG", "trace")
                .env("GITHUB_TOKEN", PRIVATE);
            crate::git::process::run(
                &mut command,
                &AtomicBool::new(false),
                Duration::from_secs(20),
            )
            .unwrap()
        };
        let check_private = |text: &str| {
            for forbidden in [
                "seeded-candidate-secret-991",
                "alice",
                "password",
                "PrivatePerson",
                "private-repo",
                "file-contents-payload",
                "https://",
                root.path().to_str().unwrap(),
            ] {
                assert!(
                    !text.contains(forbidden),
                    "candidate output contained excluded fixture data"
                );
            }
        };
        let output = run(None);
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        check_private(&stdout);
        check_private(&stderr);
        let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(report["git_version"], "2.54.0");
        assert_eq!(report["lfs_version"], "3.7.0");
        assert_eq!(report["git_location"], "[private executable path omitted]");
        assert_eq!(report["updater"], "not_running");
        assert_eq!(report["network"], "not checked; offline report; no upload");
        assert!(report["keyring_availability"]
            .as_str()
            .unwrap()
            .contains("not probed"));
        if let Some(path) = std::env::var_os("RECORDE_DIAGNOSTICS_EVIDENCE") {
            std::fs::write(path, &stdout).unwrap();
        }
        let destination = root.path().join("private-support-output.json");
        let output = run(Some(&destination));
        assert!(output.status.success());
        check_private(&String::from_utf8(output.stdout).unwrap());
        check_private(&String::from_utf8(output.stderr).unwrap());
        check_private(&std::fs::read_to_string(destination).unwrap());
        let logs = config_dir.join("diagnostics");
        for file in std::fs::read_dir(&logs).unwrap() {
            check_private(&std::fs::read_to_string(file.unwrap().path()).unwrap());
        }
        std::fs::rename(&logs, config_dir.join("logs-from-success")).unwrap();
        std::fs::write(&logs, "blocked log directory").unwrap();
        let output = run(None);
        assert!(
            output.status.success(),
            "unwritable logging must not stop the CLI"
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        check_private(&stdout);
        check_private(&String::from_utf8(output.stderr).unwrap());
        let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert!(report["local_logging"]
            .as_str()
            .unwrap()
            .starts_with("unavailable"));
    }
    #[test]
    fn panic_hook_and_trace_logging_never_persist_sensitive_payloads() {
        const SECRET: &str = "panic-secret-991 C:\\Users\\PrivatePerson\\private-repo https://alice:password@host/file file-contents-payload";
        if std::env::var_os("RECORDE_DIAG_PANIC_CHILD").is_some() {
            let root = PathBuf::from(std::env::var_os("RECORDE_DIAG_TEST_ROOT").unwrap());
            let logging = start_logging_with(move || Some(root));
            let previous = std::panic::take_hook();
            crate::crash::install();
            log::error!("{SECRET}");
            assert!(std::panic::catch_unwind(|| panic!("{SECRET}")).is_err());
            drop(logging);
            std::panic::set_hook(previous);
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let mut command = crate::git::process::background_command(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "diagnostics::tests::panic_hook_and_trace_logging_never_persist_sensitive_payloads",
                "--nocapture",
            ])
            .env("RECORDE_DIAG_PANIC_CHILD", "1")
            .env("RECORDE_DIAG_TEST_ROOT", root.path())
            .env("RUST_LOG", "trace");
        let output = crate::git::process::run(
            &mut command,
            &AtomicBool::new(false),
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(output.status.success());
        let mut combined =
            String::from_utf8(output.stdout).unwrap() + &String::from_utf8(output.stderr).unwrap();
        for file in std::fs::read_dir(root.path()).unwrap() {
            combined.push_str(&std::fs::read_to_string(file.unwrap().path()).unwrap());
        }
        assert!(combined.contains("rust_panic_payload_omitted"));
        for forbidden in [
            "panic-secret-991",
            "PrivatePerson",
            "private-repo",
            "alice",
            "password",
            "file-contents-payload",
        ] {
            assert!(!combined.contains(forbidden), "{forbidden}");
        }
    }
    #[test]
    fn reports_and_rotated_events_omit_secrets_paths_and_payloads() {
        let secret = "seeded-secret-t7-991";
        crate::auth::redact::remember(secret);
        let sensitive = format!("{secret} https://alice:password@host/private/repo?token=value C:\\Users\\PrivatePerson\\private-repo /home/privateperson/file file-contents-payload");
        let git = crate::git::readiness::Readiness {
            executable: Some(PathBuf::from(
                "C:\\Users\\PrivatePerson\\private-repo\\git.exe",
            )),
            version: format!("git version 2.54.0.windows.1 {sensitive}"),
            problem: Some(sensitive.clone()),
            lfs_version: Some(format!("git-lfs/3.7.0 {sensitive}")),
            lfs_problem: Some(sensitive.clone()),
        };
        let report = build_report(&git, UpdaterState::NeedsAttention);
        assert!(report.text().contains("2.54.0"));
        assert!(report.text().contains("private executable path omitted"));
        for forbidden in [
            secret,
            "alice",
            "password",
            "private-repo",
            "PrivatePerson",
            "/home/",
            "file-contents-payload",
            "https://",
        ] {
            assert!(!report.text().contains(forbidden), "{forbidden}");
        }
        let root = tempfile::tempdir().unwrap();
        let mut logs = RotatingLog::new(root.path().join("logs")).unwrap();
        for _ in 0..8000 {
            logs.append(Event::OperationFailed(crate::ui::error_notice::classify(
                &sensitive,
            )))
            .unwrap();
        }
        logs.append(Event::RustPanic).unwrap();
        let files: Vec<_> = std::fs::read_dir(&logs.root)
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(files.len(), LOG_FILES + 1);
        let mut total = 0;
        for file in files {
            let bytes = std::fs::read(file.path()).unwrap();
            total += bytes.len();
            assert!(bytes.len() <= LOG_BYTES as usize);
            let text = String::from_utf8(bytes).unwrap();
            for forbidden in [
                secret,
                "PrivatePerson",
                "password",
                "private-repo",
                "file-contents-payload",
            ] {
                assert!(!text.contains(forbidden));
            }
        }
        assert!(total <= LOG_FILES * LOG_BYTES as usize);
    }

    #[test]
    fn unwritable_storage_and_existing_report_fail_without_raw_paths() {
        let root = tempfile::tempdir().unwrap();
        let blocked = root.path().join("private-path-file");
        std::fs::write(&blocked, "private file contents").unwrap();
        assert!(matches!(
            RotatingLog::new(blocked.join("logs")),
            Err(LogError::Unavailable)
        ));
        let report = build_report(&Default::default(), UpdaterState::NotRunning);
        let error = save(&blocked, &report).unwrap_err();
        assert!(!error.contains("private-path-file"));
        assert_eq!(
            std::fs::read_to_string(blocked).unwrap(),
            "private file contents"
        );
    }
}
