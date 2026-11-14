//! Supervise Git and its subprocesses without blocking on full stdout/stderr pipes.
use super::error::GitError;
use command_group::CommandGroup;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

thread_local! {
    static LOCAL_CANCEL: std::cell::RefCell<Option<std::sync::Arc<AtomicBool>>> = const { std::cell::RefCell::new(None) };
}

/// Local jobs can contain multiple Git commands, all sharing one cancellation token.
pub(crate) fn with_cancellation<T>(
    token: std::sync::Arc<AtomicBool>,
    work: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<std::sync::Arc<AtomicBool>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            LOCAL_CANCEL.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let _restore = Restore(LOCAL_CANCEL.with(|slot| slot.replace(Some(token))));
    work()
}

fn cancelled(explicit: &AtomicBool) -> bool {
    explicit.load(Ordering::Relaxed)
        || LOCAL_CANCEL.with(|slot| {
            slot.borrow()
                .as_ref()
                .is_some_and(|token| token.load(Ordering::Relaxed))
        })
}

pub(super) fn check_local_cancellation() -> Result<(), String> {
    if cancelled(&AtomicBool::new(false)) {
        Err("Operation cancelled.".into())
    } else {
        Ok(())
    }
}

/// All unattended programs must start here, including probes and test fixtures.
pub(crate) fn background_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    if program.as_ref() == std::ffi::OsStr::new("git") {
        let path = super::readiness::executable().unwrap_or_else(|| "git".into());
        let mut command = raw_background_command(&path);
        command
            .env(super::readiness::COMMAND_MARKER, "1")
            .env("RECORDE_GIT_EXECUTABLE", &path);
        return command;
    }
    raw_background_command(program)
}

/// Readiness probes bypass the readiness gate, but retain the same supervision.
pub(crate) fn raw_background_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let command = Command::new(program);
    #[cfg(windows)]
    let command = {
        use std::os::windows::process::CommandExt;
        let mut command = command;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        command
    };
    command
}

pub(crate) fn command(path: &Path) -> Command {
    let mut command = background_command("git");
    command
        .arg("-C")
        .arg(path)
        .env("GIT_EDITOR", "true")
        .env("GIT_TERMINAL_PROMPT", "0");
    // The executable implements Git's helper protocol before initializing the GUI.
    // Tests run as a harness, so they exercise the protocol through an injected resolver.
    if !cfg!(test) {
        if let Ok(exe) = std::env::current_exe() {
            let exe = exe.to_string_lossy().replace('\\', "/");
            let quoted = format!("'{}'", exe.replace('\'', "'\\''"));
            command
                .args([
                    "-c",
                    "credential.helper=",
                    "-c",
                    &format!("credential.helper=!{quoted} --credential-helper"),
                    "-c",
                    "credential.useHttpPath=true",
                ])
                .env("RECORDE_CREDENTIAL_REPO", path);
        }
    }
    command
}

pub(crate) fn git(path: &Path, args: &[&str], cancel: &AtomicBool) -> Result<Output, GitError> {
    let mut command = command(path);
    command.args(args);
    let output = run(&mut command, cancel, Duration::from_secs(600))?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(GitError::Command {
            cmd: format!("git {}", args.join(" ")),
            stderr: crate::auth::redact::text(&String::from_utf8_lossy(&output.stderr)),
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Deadline {
    Total(Duration),
    Idle(Duration),
    None,
}

/// Network transfers have no total deadline, but must produce output every five minutes.
pub(crate) fn git_network(
    path: &Path,
    args: &[&str],
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(&str, Option<u8>),
) -> Result<Output, GitError> {
    let mut command = command(path);
    if let Some((verb, rest)) = args.split_first() {
        command.arg(verb);
        if matches!(*verb, "clone" | "fetch" | "push") {
            command.arg("--progress");
        }
        command.args(rest);
    }
    let output = run_network(&mut command, cancel, on_progress)?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(GitError::Command {
            cmd: "Git network operation".into(),
            stderr: crate::auth::redact::text(&String::from_utf8_lossy(&output.stderr)),
        })
    }
}

/// Prepared commands must add --progress themselves, preserving native path arguments.
pub(crate) fn run_network(
    command: &mut Command,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(&str, Option<u8>),
) -> Result<Output, GitError> {
    run_inner(
        command,
        cancel,
        Deadline::Idle(Duration::from_secs(300)),
        Stdio::null(),
        None,
        None,
        Some(on_progress),
        None,
    )
}

/// Return recognized short labels, never arbitrary remote diagnostics or URLs.
fn parse_progress(input: &str) -> Vec<(String, Option<u8>)> {
    input
        .split(['\r', '\n'])
        .filter_map(|line| {
            let labels = [
                "Receiving objects",
                "Resolving deltas",
                "Writing objects",
                "Counting objects",
                "Compressing objects",
                "Enumerating objects",
                "Filtering content",
                "Updating files",
            ];
            let label = labels.iter().find(|label| line.contains(**label)).copied();
            let label = label.or_else(|| {
                line.to_ascii_lowercase()
                    .contains("lfs")
                    .then_some("Transferring LFS objects")
            })?;
            let pct = line.find('%').and_then(|end| {
                let digits = line[..end]
                    .trim_end()
                    .rsplit(|c: char| !c.is_ascii_digit())
                    .next()?;
                digits.parse::<u8>().ok().filter(|pct| *pct <= 100)
            });
            Some((label.to_string(), pct))
        })
        .collect()
}

type Progress = (String, Option<u8>);

/// Captured after the owned process group has stopped. Streams are separate;
/// elapsed time covers the command, not individual hooks.
pub(crate) struct DiagnosticOutput<'a> {
    pub stdout: &'a [u8],
    pub stderr: &'a [u8],
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub elapsed: Duration,
    pub success: Option<bool>,
    pub cancelled: bool,
}

type DiagnosticObserver<'a> = &'a mut dyn FnMut(DiagnosticOutput<'_>);

fn send_progress(tx: &mpsc::SyncSender<Progress>, line: &[u8]) {
    let line = crate::auth::redact::text(&String::from_utf8_lossy(line));
    for update in parse_progress(&line) {
        let _ = tx.try_send(update);
    }
}

#[cfg(test)]
fn drain(reader: impl Read) -> std::io::Result<(Vec<u8>, bool)> {
    drain_active(reader, Instant::now(), Arc::new(AtomicU64::new(0)), None)
}

fn drain_active(
    mut reader: impl Read,
    start: Instant,
    activity: Arc<AtomicU64>,
    progress: Option<mpsc::SyncSender<Progress>>,
) -> std::io::Result<(Vec<u8>, bool)> {
    // Keep the last MiB for diagnostics while draining all bytes to avoid deadlock.
    const LIMIT: usize = 1024 * 1024;
    let mut output = Vec::new();
    let mut truncated = false;
    let mut buffer = [0; 8192];
    let mut line = Vec::new();
    let mut overlong = false;
    loop {
        let count = reader.read(&mut buffer)?;
        if count > 0 {
            activity.fetch_max(start.elapsed().as_millis() as u64, Ordering::Relaxed);
        }
        if let Some(tx) = &progress {
            for &byte in &buffer[..count] {
                if byte == b'\r' || byte == b'\n' {
                    if !overlong {
                        send_progress(tx, &line);
                    }
                    line.clear();
                    overlong = false;
                } else if line.len() < 4096 && !overlong {
                    line.push(byte);
                } else {
                    line.clear();
                    overlong = true;
                }
            }
        }
        if count == 0 {
            if let Some(tx) = &progress {
                if !overlong {
                    send_progress(tx, &line);
                }
            }
            return Ok((output, truncated));
        }
        output.extend_from_slice(&buffer[..count]);
        if output.len() > LIMIT {
            truncated = true;
            output.drain(..output.len() - LIMIT);
        }
    }
}

pub(crate) fn run(
    command: &mut Command,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<Output, GitError> {
    run_with_stdin(command, cancel, timeout, Stdio::null())
}

pub(crate) fn run_with_stdin(
    command: &mut Command,
    cancel: &AtomicBool,
    timeout: Duration,
    stdin: Stdio,
) -> Result<Output, GitError> {
    run_inner(
        command,
        cancel,
        Deadline::Total(timeout),
        stdin,
        None,
        None,
        None,
        None,
    )
}

/// Stream machine-readable listings into a private file without applying the
/// diagnostic buffer limit. Cancellation and descendant cleanup remain shared.
pub(crate) fn run_to_file(
    command: &mut Command,
    output: std::fs::File,
    timeout: Duration,
) -> Result<Output, GitError> {
    run_inner(
        command,
        &AtomicBool::new(false),
        Deadline::Total(timeout),
        Stdio::null(),
        None,
        Some(output),
        None,
        None,
    )
}

pub(crate) fn run_with_input(
    command: &mut Command,
    input: &[u8],
    timeout: Duration,
) -> Result<Output, GitError> {
    run_inner(
        command,
        &AtomicBool::new(false),
        Deadline::Total(timeout),
        Stdio::piped(),
        Some(input.to_vec()),
        None,
        None,
        None,
    )
}

#[cfg(test)]
pub(crate) fn run_with_deadline(
    command: &mut Command,
    cancel: &AtomicBool,
    deadline: Deadline,
) -> Result<Output, GitError> {
    run_inner(
        command,
        cancel,
        deadline,
        Stdio::null(),
        None,
        None,
        None,
        None,
    )
}

/// Human-readable command output may be truncated while its actual exit status
/// remains authoritative. Machine-readable callers keep the strict runner.
pub(crate) fn run_diagnostic(
    command: &mut Command,
    cancel: &AtomicBool,
    observer: DiagnosticObserver<'_>,
) -> Result<Output, GitError> {
    let start = Instant::now();
    let mut delivered = false;
    let result = {
        let mut capture = |output: DiagnosticOutput<'_>| {
            delivered = true;
            observer(output);
        };
        run_inner(
            command,
            cancel,
            Deadline::None,
            Stdio::null(),
            None,
            None,
            None,
            Some(&mut capture),
        )
    };
    if !delivered {
        observer(DiagnosticOutput {
            stdout: &[],
            stderr: &[],
            stdout_truncated: false,
            stderr_truncated: false,
            elapsed: start.elapsed(),
            success: None,
            cancelled: matches!(result, Err(GitError::Cancelled)),
        });
    }
    result
}

fn run_inner(
    command: &mut Command,
    cancel: &AtomicBool,
    deadline: Deadline,
    stdin: Stdio,
    input: Option<Vec<u8>>,
    output_file: Option<std::fs::File>,
    mut on_progress: Option<&mut dyn FnMut(&str, Option<u8>)>,
    mut observer: Option<DiagnosticObserver<'_>>,
) -> Result<Output, GitError> {
    if cancelled(cancel) {
        return Err(GitError::Cancelled);
    }
    let system_git = command
        .get_envs()
        .any(|(name, value)| name == super::readiness::COMMAND_MARKER && value.is_some());
    if system_git {
        let readiness = super::readiness::check();
        readiness
            .require_ready()
            .map_err(|stderr| GitError::Command {
                cmd: "System Git readiness".into(),
                stderr,
            })?;
        if readiness.executable.as_deref().map(Path::as_os_str) != Some(command.get_program()) {
            return Err(GitError::Command {
                cmd: "System Git readiness".into(),
                stderr: "The selected Git executable changed. Retry the operation using the current Git check in Settings > Git.".into(),
            });
        }
    }
    let program = command.get_program().to_string_lossy().into_owned();
    let failure = |e: std::io::Error| GitError::Command {
        cmd: if system_git {
            "System Git readiness"
        } else {
            "Background process"
        }
        .into(),
        stderr: if system_git {
            super::readiness::invalidate();
            format!("Git at {program} stopped: {e}. Check this executable, then choose Retry Git check in Settings > Git.")
        } else {
            e.to_string()
        },
    };
    command
        .stdin(stdin)
        .stdout(output_file.map_or_else(Stdio::piped, Stdio::from))
        .stderr(Stdio::piped());
    let mut group = command.group();
    // command-group replaces Command's creation flags when it creates the job.
    // Set this on its builder too, or every Git command allocates a console.
    #[cfg(windows)]
    group.creation_flags(0x08000000).kill_on_drop(true); // CREATE_NO_WINDOW
    let mut child = group.spawn().map_err(failure)?;
    // Never log arguments, stdin or environment: credential helpers carry secrets.
    log::debug!(
        "Background process started: pid={}, program={:?}, deadline={:?}, ports=none requested",
        child.id(),
        command.get_program(),
        deadline
    );
    let writer = input.map(|bytes| {
        let mut stdin = child.inner().stdin.take().expect("piped stdin");
        std::thread::spawn(move || {
            use std::io::Write;
            stdin.write_all(&bytes)
        })
    });
    let start = Instant::now();
    let activity = Arc::new(AtomicU64::new(0));
    let (progress_tx, progress_rx) = mpsc::sync_channel(32);
    let stdout = child.inner().stdout.take();
    let stderr = child.inner().stderr.take().expect("piped stderr");
    let out_activity = activity.clone();
    let out_reader = stdout
        .map(|stdout| std::thread::spawn(move || drain_active(stdout, start, out_activity, None)));
    let err_activity = activity.clone();
    let progress_tx = on_progress.as_ref().map(|_| progress_tx);
    let err_reader =
        std::thread::spawn(move || drain_active(stderr, start, err_activity, progress_tx));
    let mut last_progress = None;
    let mut pending_progress = None;
    let mut interrupted = None;
    let status = loop {
        for update in progress_rx.try_iter().take(32) {
            pending_progress = Some(update);
        }
        if last_progress.is_none_or(|last: Instant| last.elapsed() >= Duration::from_millis(100)) {
            if let (Some(callback), Some((message, pct))) =
                (on_progress.as_mut(), pending_progress.take())
            {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    callback(&message, pct);
                }));
                if result.is_err() {
                    interrupted = Some(GitError::Command {
                        cmd: "Git process".into(),
                        stderr:
                            "Progress reporting stopped unexpectedly; the transfer was stopped."
                                .into(),
                    });
                    break None;
                }
                last_progress = Some(Instant::now());
            }
        }
        if cancelled(cancel) {
            interrupted = Some(GitError::Cancelled);
            break None;
        }
        let expired = match deadline {
            Deadline::Total(timeout) => start.elapsed() >= timeout,
            Deadline::Idle(timeout) => {
                start
                    .elapsed()
                    .saturating_sub(Duration::from_millis(activity.load(Ordering::Relaxed)))
                    >= timeout
            }
            Deadline::None => false,
        };
        if expired {
            let message = match deadline {
                Deadline::Idle(timeout) if timeout == Duration::from_secs(300) => concat!(
                    "No output from Git for 5 minutes; the transfer was stopped. ",
                    "Check your connection and retry."
                )
                .into(),
                Deadline::Idle(timeout) => format!(
                    concat!(
                        "Inactivity timeout: no output from Git for {} milliseconds; ",
                        "the transfer was stopped. Check your connection and retry."
                    ),
                    timeout.as_millis()
                ),
                Deadline::Total(timeout) => format!(
                    "Timed out after {} seconds; inspect repository state before retrying.",
                    timeout.as_secs()
                ),
                Deadline::None => unreachable!(),
            };
            interrupted = Some(GitError::Command {
                cmd: "Git process".into(),
                stderr: message,
            });
            break None;
        }
        match child.inner().try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                interrupted = Some(failure(error));
                break None;
            }
        }
    };
    // Terminate the group/job even if its leader exited, closing inherited pipes.
    let _ = child.kill();
    let _ = child.wait();
    log::debug!("Background process stopped: pid={}", child.id());
    let input_result = writer.map(|writer| {
        writer
            .join()
            .unwrap_or_else(|_| Err(std::io::Error::other("stdin writer stopped")))
    });
    let (stdout, stdout_truncated) = match out_reader {
        Some(reader) => reader
            .join()
            .map_err(|_| failure(std::io::Error::other("stdout reader stopped")))?
            .map_err(failure)?,
        None => (Vec::new(), false),
    };
    let (stderr, stderr_truncated) = err_reader
        .join()
        .map_err(|_| failure(std::io::Error::other("stderr reader stopped")))?
        .map_err(failure)?;
    if let Some(observer) = observer.as_mut() {
        observer(DiagnosticOutput {
            stdout: &stdout,
            stderr: &stderr,
            stdout_truncated,
            stderr_truncated,
            elapsed: start.elapsed(),
            success: status.map(|s| s.success()),
            cancelled: matches!(interrupted, Some(GitError::Cancelled)),
        });
    }
    if let Some(error) = interrupted {
        return Err(error);
    }
    if let Some(result) = input_result {
        result.map_err(failure)?;
    }
    if stdout_truncated && observer.is_none() {
        return Err(failure(std::io::Error::other(
            "Command output exceeded 1 MiB; refusing an incomplete response.",
        )));
    }
    Ok(Output {
        status: status.expect("completed process"),
        stdout,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t11_diagnostic_observer_runs_once_for_spawn_failure_and_early_cancellation() {
        for cancel in [false, true] {
            let mut command = background_command("recorde-does-not-exist-output-fixture");
            let mut observations = 0;
            let result = run_diagnostic(&mut command, &AtomicBool::new(cancel), &mut |output| {
                observations += 1;
                assert!(output.stdout.is_empty() && output.stderr.is_empty());
                assert_eq!(output.cancelled, cancel);
                assert_eq!(output.success, None);
            });
            assert!(result.is_err());
            assert_eq!(observations, 1);
        }
    }

    #[test]
    fn t11_diagnostic_output_bounds_both_streams_without_overriding_success() {
        const TEST: &str = "git::process::tests::t11_diagnostic_output_bounds_both_streams_without_overriding_success";
        if std::env::var_os("RECORDE_T11_DIAGNOSTIC_CHILD").is_some() {
            for _ in 0..1200 {
                println!("{}", "x".repeat(1024));
                eprintln!("{}", "y".repeat(1024));
            }
            println!("stdout-tail");
            eprintln!("stderr-tail");
            return;
        }
        let mut command = background_command(std::env::current_exe().unwrap());
        command
            .args(["--exact", TEST, "--nocapture"])
            .env("RECORDE_T11_DIAGNOSTIC_CHILD", "1");
        let mut observations = 0;
        let result = run_diagnostic(&mut command, &AtomicBool::new(false), &mut |output| {
            observations += 1;
            assert!(output.stdout_truncated && output.stderr_truncated);
            assert!(output.stdout.len() <= 1024 * 1024 && output.stderr.len() <= 1024 * 1024);
            assert!(String::from_utf8_lossy(output.stdout).contains("stdout-tail"));
            assert!(String::from_utf8_lossy(output.stderr).contains("stderr-tail"));
            assert_eq!(output.success, Some(true));
        })
        .unwrap();
        assert!(result.status.success());
        assert_eq!(observations, 1);
    }

    fn deadline_fixture() -> bool {
        let Ok(mode) = std::env::var("RECORDE_IDLE_FIXTURE") else {
            return false;
        };
        use std::io::Write;
        // Writes every 100 ms for 3 s: well inside, then well past, a 1.5 s idle window.
        for _ in 0..30 {
            match mode.as_str() {
                "stdout" => {
                    std::io::stdout().write_all(b"x").unwrap();
                    std::io::stdout().flush().unwrap();
                }
                "stderr" => {
                    std::io::stderr().write_all(b"x").unwrap();
                    std::io::stderr().flush().unwrap();
                }
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        true
    }

    fn idle_fixture_command(test: &str, mode: &str) -> Command {
        let mut command = background_command(std::env::current_exe().unwrap());
        command
            .args(["--exact", test, "--nocapture"])
            .env("RECORDE_IDLE_FIXTURE", mode);
        command
    }

    #[test]
    fn idle_activity_extends_deadline_on_either_pipe() {
        if deadline_fixture() {
            return;
        }
        for mode in ["stdout", "stderr"] {
            let mut command = idle_fixture_command(
                "git::process::tests::idle_activity_extends_deadline_on_either_pipe",
                mode,
            );
            let output = run_with_deadline(
                &mut command,
                &AtomicBool::new(false),
                Deadline::Idle(Duration::from_millis(1500)),
            )
            .unwrap();
            assert!(output.status.success());
        }
    }

    #[test]
    fn idle_silence_stops_child_with_inactivity_error() {
        if deadline_fixture() {
            return;
        }
        let mut command = idle_fixture_command(
            "git::process::tests::idle_silence_stops_child_with_inactivity_error",
            "silent",
        );
        let error = run_with_deadline(
            &mut command,
            &AtomicBool::new(false),
            Deadline::Idle(Duration::from_millis(1500)),
        )
        .unwrap_err();
        let message = error.user_message();
        assert!(message.contains("Inactivity timeout"), "{message}");
        assert!(message.contains("1500 milliseconds"), "{message}");
    }

    #[test]
    fn unlimited_deadline_still_observes_local_cancellation() {
        with_cancellation(Arc::new(AtomicBool::new(true)), || {
            let mut command = background_command("does-not-exist-recorde-test");
            assert!(matches!(
                run_with_deadline(&mut command, &AtomicBool::new(false), Deadline::None),
                Err(GitError::Cancelled)
            ));
        });
        assert!(!cancelled(&AtomicBool::new(false)));
    }

    #[test]
    fn progress_parser_handles_git_lfs_percentages_and_carriage_returns() {
        let updates = parse_progress(concat!(
            "Receiving objects:  45% (450/1000)\r",
            "remote: Resolving deltas: 100% (10/10)\r\n",
            "Writing objects:  12% (3/25)\n",
            "Counting objects: 10, done.\r",
            "Compressing objects: 50% (5/10)\r",
            "Filtering content: 20% (2/10)\r",
            "Uploading LFS objects: 70% (7/10)\r",
            "Git LFS: transferring files\n"
        ));
        assert_eq!(
            updates,
            vec![
                ("Receiving objects".into(), Some(45)),
                ("Resolving deltas".into(), Some(100)),
                ("Writing objects".into(), Some(12)),
                ("Counting objects".into(), None),
                ("Compressing objects".into(), Some(50)),
                ("Filtering content".into(), Some(20)),
                ("Transferring LFS objects".into(), Some(70)),
                ("Transferring LFS objects".into(), None),
            ]
        );
        assert!(parse_progress("fatal: unavailable repository").is_empty());
        assert_eq!(parse_progress("Counting objects: 999%")[0].1, None);
        assert_eq!(
            parse_progress("Receiving objects: https://user:secret@example.com/repo 45%"),
            vec![("Receiving objects".into(), Some(45))]
        );
    }

    #[test]
    fn stderr_progress_retains_diagnostics_and_unterminated_last_line() {
        let bytes = b"Counting objects\rReceiving objects: 45%\rdiagnostic\nWriting objects: 12%";
        let (tx, rx) = mpsc::sync_channel(32);
        let (tail, truncated) = drain_active(
            &bytes[..],
            Instant::now(),
            Arc::new(AtomicU64::new(0)),
            Some(tx),
        )
        .unwrap();
        assert_eq!(tail, bytes);
        assert!(!truncated);
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            vec![
                ("Counting objects".into(), None),
                ("Receiving objects".into(), Some(45)),
                ("Writing objects".into(), Some(12)),
            ]
        );
    }

    #[test]
    fn bounded_output_reports_truncation() {
        let (bytes, truncated) = drain(&b"small response"[..]).unwrap();
        assert_eq!(bytes, b"small response");
        assert!(!truncated);
        let (bytes, truncated) = drain(std::io::repeat(b'x').take(2 * 1024 * 1024)).unwrap();
        assert_eq!(bytes.len(), 1024 * 1024);
        assert!(truncated);
    }

    #[test]
    fn incomplete_stdin_is_not_reported_as_success() {
        if std::env::var_os("RECORDE_TEST_CLOSED_INPUT").is_some() {
            return;
        }
        let mut command = background_command(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "git::process::tests::incomplete_stdin_is_not_reported_as_success",
            ])
            .env("RECORDE_TEST_CLOSED_INPUT", "1");
        let result = run_with_input(
            &mut command,
            &vec![b'x'; 2 * 1024 * 1024],
            Duration::from_secs(10),
        );
        assert!(
            result.is_err(),
            "A child which never reads its input must not report success"
        );
    }

    #[test]
    fn supervised_input_closes_stdin_and_collects_response() {
        const FIXTURE: &str = "protocol=https\nhost=example.invalid\n\n";
        if std::env::var_os("RECORDE_TEST_INPUT_CHILD").is_some() {
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input).unwrap();
            assert_eq!(input, FIXTURE);
            println!("input pipe reached EOF");
            return;
        }
        let mut command = background_command(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "git::process::tests::supervised_input_closes_stdin_and_collects_response",
                "--nocapture",
            ])
            .env("RECORDE_TEST_INPUT_CHILD", "1");
        let output =
            run_with_input(&mut command, FIXTURE.as_bytes(), Duration::from_secs(10)).unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("input pipe reached EOF"));
    }
    #[cfg(windows)]
    #[test]
    fn windows_cancellation_terminates_the_grandchild() {
        use std::os::windows::process::CommandExt;
        const TEST: &str = "git::process::tests::windows_cancellation_terminates_the_grandchild";
        if let Some(marker) = std::env::var_os("RECORDE_TEST_JOB_MARKER") {
            if std::env::var_os("RECORDE_TEST_JOB_GRANDCHILD").is_some() {
                std::fs::write(marker, std::process::id().to_string()).unwrap();
                // A watchdog also bounds the fixture if job inheritance regresses.
                let start = Instant::now();
                while start.elapsed() < Duration::from_secs(15) {
                    println!("{}", "stdout fixture ".repeat(1024));
                    eprintln!("{}", "stderr fixture ".repeat(1024));
                    std::thread::sleep(Duration::from_millis(2));
                }
            } else {
                let mut child = Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", TEST, "--nocapture"])
                    .env("RECORDE_TEST_JOB_GRANDCHILD", "1")
                    .creation_flags(0x08000000)
                    .spawn()
                    .unwrap();
                let _ = child.wait();
            }
            return;
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn OpenProcess(access: u32, inherit: i32, id: u32) -> isize;
            fn WaitForSingleObject(handle: isize, milliseconds: u32) -> u32;
            fn CloseHandle(handle: isize) -> i32;
        }
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("grandchild-pid");
        let child_marker = marker.clone();
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let child_cancel = cancel.clone();
        let worker = std::thread::spawn(move || {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", TEST, "--nocapture"])
                .env("RECORDE_TEST_JOB_MARKER", child_marker);
            run(&mut command, &child_cancel, Duration::from_secs(20))
        });
        let start = Instant::now();
        let id: u32 = loop {
            if let Some(id) = std::fs::read_to_string(&marker)
                .ok()
                .and_then(|s| s.parse().ok())
            {
                break id;
            }
            if start.elapsed() > Duration::from_secs(10) {
                cancel.store(true, Ordering::Relaxed);
                let _ = worker.join();
                panic!("Grandchild did not start");
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        // Own a synchronization handle before cancellation so PID reuse cannot
        // make the assertion observe an unrelated process.
        let handle = unsafe { OpenProcess(0x00100000, 0, id) };
        cancel.store(true, Ordering::Relaxed);
        let result = worker.join().unwrap();
        assert_ne!(handle, 0);
        let waited = unsafe { WaitForSingleObject(handle, 5000) };
        unsafe {
            CloseHandle(handle);
        }
        assert!(matches!(result, Err(GitError::Cancelled)));
        assert_eq!(waited, 0, "Grandchild survived cancellation");
    }

    #[cfg(windows)]
    #[test]
    fn windows_supervised_child_has_no_console() {
        if std::env::var_os("RECORDE_TEST_CONSOLE_CHILD").is_some() {
            #[link(name = "kernel32")]
            extern "system" {
                fn GetConsoleWindow() -> isize;
            }
            // This query does not own or dereference the returned window handle.
            assert_eq!(
                unsafe { GetConsoleWindow() },
                0,
                "Git worker allocated a console"
            );
            return;
        }
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "git::process::tests::windows_supervised_child_has_no_console",
            ])
            .env("RECORDE_TEST_CONSOLE_CHILD", "1");
        let result = run(
            &mut command,
            &AtomicBool::new(false),
            Duration::from_secs(30),
        )
        .unwrap();
        assert!(
            result.status.success(),
            "{} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
    #[test]
    fn local_job_cancellation_propagates_to_nested_commands_and_is_scoped() {
        let token = std::sync::Arc::new(AtomicBool::new(true));
        with_cancellation(token, || {
            let mut command = Command::new("does-not-exist-recorde-test");
            assert!(matches!(
                run(
                    &mut command,
                    &AtomicBool::new(false),
                    Duration::from_secs(1)
                ),
                Err(GitError::Cancelled)
            ));
        });
        assert!(!cancelled(&AtomicBool::new(false)));
    }

    #[test]
    fn pre_cancelled_command_never_starts() {
        let mut command = Command::new("does-not-exist-recorde-test");
        assert!(matches!(
            run(&mut command, &AtomicBool::new(true), Duration::from_secs(1)),
            Err(GitError::Cancelled)
        ));
    }
    #[cfg(unix)]
    #[test]
    fn cancellation_kills_children_and_drains_both_pipes() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("started");
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let marker2 = marker.clone();
        let worker = std::thread::spawn(move || {
            let mut cmd = Command::new("sh");
            cmd.args([
                "-c",
                "printf ready > \"$1\"; while :; do printf output; printf error >&2; done & wait",
                "test",
            ])
            .arg(marker2);
            run(&mut cmd, &signal, Duration::from_secs(10))
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while !marker.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(marker.exists());
        cancel.store(true, Ordering::Relaxed);
        assert!(matches!(worker.join().unwrap(), Err(GitError::Cancelled)));
    }
}
