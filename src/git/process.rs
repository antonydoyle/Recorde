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

pub(crate) fn command(path: &Path) -> Command {
    let mut command = Command::new("git");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
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

fn drain(mut reader: impl Read) -> std::io::Result<Vec<u8>> {
    // Keep the last MiB for diagnostics while draining all bytes to avoid deadlock.
    const LIMIT: usize = 1024 * 1024;
    let mut output = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            return Ok(output);
        }
        output.extend_from_slice(&buffer[..count]);
        if output.len() > LIMIT {
            output.drain(..output.len() - LIMIT);
        }
    }
}

pub(crate) fn run(
    command: &mut Command,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<Output, GitError> {
    if cancelled(cancel) {
        return Err(GitError::Cancelled);
    }
    let failure = |e: std::io::Error| GitError::Command {
        cmd: "Git process".into(),
        stderr: e.to_string(),
    };
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .group_spawn()
        .map_err(failure)?;
    let stdout = child.inner().stdout.take().expect("piped stdout");
    let stderr = child.inner().stderr.take().expect("piped stderr");
    let out_reader = std::thread::spawn(move || drain(stdout));
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
                stderr: "Timed out after ten minutes; inspect repository state before retrying."
                    .into(),
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
    let stdout = out_reader
        .join()
        .map_err(|_| failure(std::io::Error::other("stdout reader stopped")))?
        .map_err(failure)?;
    let stderr = err_reader
        .join()
        .map_err(|_| failure(std::io::Error::other("stderr reader stopped")))?
        .map_err(failure)?;
    if let Some(error) = interrupted {
        return Err(error);
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
