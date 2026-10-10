//! The privileged helper's complete process allowlist and bounded supervisor.
use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicBool, Ordering};

static CANCELLED: AtomicBool = AtomicBool::new(false);
extern "C" fn cancel(_: libc::c_int) { CANCELLED.store(true, Ordering::Relaxed); }

#[derive(Clone, Copy)]
pub enum Tool { Query, Deb, Dpkg }
impl Tool {
    pub const ALL: [Self; 3] = [Self::Query, Self::Deb, Self::Dpkg];
    pub fn path(self) -> &'static str {
        match self { Self::Query => "/usr/bin/dpkg-query", Self::Deb => "/usr/bin/dpkg-deb", Self::Dpkg => "/usr/bin/dpkg" }
    }
}
pub struct Output { pub success: bool, pub stdout: Vec<u8>, pub diagnostic: String }

pub fn enable_subreaper() -> Result<(), String> {
    // An inherited SIG_IGN would auto-reap children and lose our PID witness.
    unsafe { libc::signal(libc::SIGCHLD, libc::SIG_DFL); }
    CANCELLED.store(false, Ordering::Relaxed);
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        if unsafe { libc::signal(signal, cancel as libc::sighandler_t) } == libc::SIG_ERR { return Err("Cannot supervise installer cancellation.".into()); }
    }
    // Adopt grandchildren even if a signed package script creates a new session.
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } != 0 { return Err("Cannot supervise package-manager descendants.".into()); }
    Ok(())
}

fn nonblocking(pipe: &impl AsRawFd) -> Result<(), String> {
    let flags = unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 { return Err("Cannot supervise package-manager output.".into()); }
    Ok(())
}

fn drain(pipe: &mut impl Read, bytes: &mut Vec<u8>, keep_tail: bool) -> Result<(), String> {
    let mut buffer = [0; 8192];
    for _ in 0..8 { // Return regularly to the wall-clock deadline.
        match pipe.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                const LIMIT: usize = 1024 * 1024;
                if bytes.len() + count > LIMIT {
                    if !keep_tail { return Err("Package-manager output exceeded its limit.".into()); }
                    bytes.drain(..bytes.len() + count - LIMIT);
                }
                bytes.extend_from_slice(&buffer[..count]);
            },
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Err("Cannot read package-manager output.".into()),
        }
    }
    Ok(())
}

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        // The leader has not been reaped by another thread. Its process group
        // cannot be reassigned while it or any member is still alive.
        unsafe { libc::kill(-(self.0.id() as i32), libc::SIGKILL); }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn reap_adopted() -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        // Production is single-threaded; enumerating task children also covers
        // Rust's test-runner thread and adopted children on the main thread.
        let mut children = String::new();
        for task in std::fs::read_dir("/proc/self/task").map_err(|_| "Cannot inspect installer tasks.")? {
            let path = task.map_err(|_| "Cannot inspect installer task.")?.path().join("children");
            match std::fs::read_to_string(path) {
                Ok(text) => { children.push(' '); children.push_str(&text); },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Err(_) => return Err("Cannot inspect owned installer descendants.".into()),
            }
        }
        if children.trim().is_empty() { return Ok(()); }
        for pid in children.split_whitespace() {
            let pid: i32 = pid.parse().map_err(|_| "Invalid owned child identity.")?;
            let mut status = 0;
            // waitpid=0 establishes that this still-live PID is our child. It
            // remains alive or an unreaped zombie until our next wait, so it
            // cannot be reused for an unrelated process before this kill.
            if unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) } == 0 { unsafe { libc::kill(pid, libc::SIGKILL); } }
        }
        if Instant::now() >= deadline { return Err("An owned package-manager descendant has not terminated; inspect system package status.".into()); }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn run(tool: Tool, args: &[&OsStr], timeout: Duration) -> Result<Output, String> {
    let installing = matches!(tool, Tool::Dpkg) && args.contains(&OsStr::new("--install"));
    let mut command = Command::new(tool.path());
    command.args(args).env_clear().env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("HOME", "/root").env("LANG", "C").env("LC_ALL", "C")
        .env("DPKG_DEB_THREADS_MAX", "1").current_dir("/")
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
    supervised(command, timeout, installing)
}

fn supervised(mut command: Command, timeout: Duration, installing: bool) -> Result<Output, String> {
    if CANCELLED.load(Ordering::Relaxed) { return Err("Installer was cancelled before starting a package task.".into()); }
    let child = command.spawn().map_err(|_| "Cannot start the fixed package-manager tool.")?;
    let mut owned = OwnedChild(child);
    let _ = writeln!(std::io::stderr(), "Recorde package task: pid={}, purpose=verified package metadata/install, ports=none", owned.0.id());
    let result = (|| {
        let mut stdout = owned.0.stdout.take().ok_or("Missing supervised stdout.")?;
        let mut stderr = owned.0.stderr.take().ok_or("Missing supervised stderr.")?;
        nonblocking(&stdout)?; nonblocking(&stderr)?;
        let deadline = Instant::now() + timeout;
        let mut out = Vec::new(); let mut err = Vec::new();
        let mut terminating = None;
        loop {
            drain(&mut stdout, &mut out, installing)?; drain(&mut stderr, &mut err, installing)?;
            // WNOWAIT preserves our ownership witness until OwnedChild kills
            // the group and reaps it. Child::try_wait would reap too early.
            let mut status: libc::siginfo_t = unsafe { std::mem::zeroed() };
            if unsafe { libc::waitid(libc::P_PID, owned.0.id(), &mut status, libc::WEXITED | libc::WNOHANG | libc::WNOWAIT) } != 0 {
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted { continue; }
                return Err("Cannot inspect package-manager status.".into());
            }
            if unsafe { status.si_pid() } != 0 {
                drain(&mut stdout, &mut out, installing)?;
                drain(&mut stderr, &mut err, installing)?;
                if terminating.is_some() { return Err(format!("Package task was interrupted; installation may be incomplete. Inspect dpkg status before retrying. {}", diagnostic(&err))); }
                return Ok(Output { success: status.si_code == libc::CLD_EXITED && unsafe { status.si_status() } == 0, stdout: out, diagnostic: diagnostic(&err) });
            }
            if terminating.is_none() && (Instant::now() >= deadline || CANCELLED.load(Ordering::Relaxed)) {
                // Give dpkg a bounded opportunity to release locks and finish
                // state writes. Never kill an install merely for verbose output.
                unsafe { libc::kill(-(owned.0.id() as i32), libc::SIGTERM); }
                terminating = Some(Instant::now() + Duration::from_secs(if installing { 30 } else { 2 }));
            }
            if terminating.is_some_and(|deadline| Instant::now() >= deadline) { return Err(format!("Package task did not stop after SIGTERM and was forcibly terminated. Installation may be incomplete; inspect dpkg status and use your package manager's repair instructions before retrying. {}", diagnostic(&err))); }
            std::thread::sleep(Duration::from_millis(20));
        }
    })();
    drop(owned);
    let cleanup = reap_adopted();
    let _ = writeln!(std::io::stderr(), "Recorde package task cleanup complete; ports=none requested");
    cleanup?;
    result
}

fn diagnostic(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(4096)..]).chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_metadata_tool_is_supervised_without_a_shell() {
        enable_subreaper().unwrap();
        let output = run(Tool::Dpkg, &[OsStr::new("--version")], Duration::from_secs(5)).unwrap();
        assert!(output.success && String::from_utf8_lossy(&output.stdout).contains("Debian"));
    }
    #[test]
    fn verbose_installs_keep_diagnostic_tail_without_aborting() {
        enable_subreaper().unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "process::tests::verbose_fixture", "--ignored", "--nocapture"])
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
        let output = supervised(command, Duration::from_secs(10), true).unwrap();
        assert!(output.success);
        assert!(output.stdout.len() <= 1024 * 1024);
        assert!(output.diagnostic.ends_with("final diagnostic\n"));
    }
    #[test]
    fn cancellation_signal_is_supervised_and_reports_ambiguous_state() {
        enable_subreaper().unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "process::tests::signal_fixture", "--ignored", "--nocapture"])
            .env("RECORDE_INSTALLER_SIGNAL_FIXTURE", "1")
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
        let error = supervised(command, Duration::from_secs(10), true).err().unwrap();
        CANCELLED.store(false, Ordering::Relaxed);
        assert!(error.contains("interrupted") && error.contains("dpkg status"));
    }
    #[test]
    #[ignore = "only started by the bounded supervisor fixture"]
    fn signal_fixture() {
        if std::env::var_os("RECORDE_INSTALLER_SIGNAL_FIXTURE").is_none() { return; }
        unsafe { libc::kill(libc::getppid(), libc::SIGTERM); }
        loop { std::thread::sleep(Duration::from_secs(1)); }
    }
    #[test]
    #[ignore = "only started by the bounded supervisor fixture"]
    fn verbose_fixture() {
        for _ in 0..300 { std::io::stdout().write_all(&[b'x'; 8192]).unwrap(); std::io::stderr().write_all(&[b'e'; 8192]).unwrap(); }
        std::io::stderr().write_all(b"final diagnostic\n").unwrap();
    }
    #[test]
    fn timeout_reaps_a_descendant_that_created_its_own_session() {
        enable_subreaper().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let witness = temp.path().join("pid");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "process::tests::detached_fixture", "--ignored", "--nocapture"])
            .env("RECORDE_INSTALLER_TEST_PID", &witness).stdin(Stdio::null())
            .stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
        assert!(supervised(command, Duration::from_millis(500), false).is_err());
        let pid: i32 = std::fs::read_to_string(witness).unwrap().parse().unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    }
    #[test]
    #[ignore = "only started by the bounded supervisor fixture"]
    fn detached_fixture() {
        let Some(witness) = std::env::var_os("RECORDE_INSTALLER_TEST_PID") else { return; };
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0);
        if pid == 0 {
            // Only async-signal-safe C functions after fork from the test runner.
            unsafe { libc::setsid(); loop { libc::pause(); } }
        }
        std::fs::write(witness, pid.to_string()).unwrap();
        loop { std::thread::sleep(Duration::from_secs(1)); }
    }
}
