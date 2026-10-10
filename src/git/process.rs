//! Supervise Git and its subprocesses without blocking on full stdout/stderr pipes.
use super::error::GitError;
use command_group::CommandGroup;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
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

/// All unattended programs must start here, including probes and test fixtures.
pub(crate) fn background_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
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

fn drain(mut reader: impl Read) -> std::io::Result<(Vec<u8>, bool)> {
    // Keep the last MiB for diagnostics while draining all bytes to avoid deadlock.
    const LIMIT: usize = 1024 * 1024;
    let mut output = Vec::new();
    let mut truncated = false;
    let mut buffer = [0; 8192];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
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
    run_inner(command, cancel, timeout, stdin, None, None)
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
        timeout,
        Stdio::null(),
        None,
        Some(output),
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
        timeout,
        Stdio::piped(),
        Some(input.to_vec()),
        None,
    )
}

fn run_inner(
    command: &mut Command,
    cancel: &AtomicBool,
    timeout: Duration,
    stdin: Stdio,
    input: Option<Vec<u8>>,
    output_file: Option<std::fs::File>,
) -> Result<Output, GitError> {
    if cancelled(cancel) {
        return Err(GitError::Cancelled);
    }
    let failure = |e: std::io::Error| GitError::Command {
        cmd: "Git process".into(),
        stderr: e.to_string(),
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
        "Background process started: pid={}, program={:?}, timeout={}s, ports=none requested",
        child.id(),
        command.get_program(),
        timeout.as_secs()
    );
    let writer = input.map(|bytes| {
        let mut stdin = child.inner().stdin.take().expect("piped stdin");
        std::thread::spawn(move || {
            use std::io::Write;
            stdin.write_all(&bytes)
        })
    });
    let stdout = child.inner().stdout.take();
    let stderr = child.inner().stderr.take().expect("piped stderr");
    let out_reader = stdout.map(|stdout| std::thread::spawn(move || drain(stdout)));
    let err_reader = std::thread::spawn(move || drain(stderr));
    let start = Instant::now();
    let mut interrupted = None;
    let status = loop {
        if cancelled(cancel) {
            interrupted = Some(GitError::Cancelled);
            break None;
        }
        if start.elapsed() >= timeout {
            interrupted = Some(GitError::Command {
                cmd: "Git process".into(),
                stderr: format!(
                    "Timed out after {} seconds; inspect repository state before retrying.",
                    timeout.as_secs()
                ),
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
    let (stderr, _) = err_reader
        .join()
        .map_err(|_| failure(std::io::Error::other("stderr reader stopped")))?
        .map_err(failure)?;
    if let Some(error) = interrupted {
        return Err(error);
    }
    if let Some(result) = input_result {
        result.map_err(failure)?;
    }
    if stdout_truncated {
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
