//! The privileged helper's complete process allowlist and bounded supervisor.
use std::ffi::OsStr;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub enum Tool { Query, Deb, Dpkg }
impl Tool {
    pub const ALL: [Self; 3] = [Self::Query, Self::Deb, Self::Dpkg];
    pub fn path(self) -> &'static str {
        match self { Self::Query => "/usr/bin/dpkg-query", Self::Deb => "/usr/bin/dpkg-deb", Self::Dpkg => "/usr/bin/dpkg" }
    }
}
pub struct Output { pub success: bool, pub stdout: Vec<u8> }

pub fn enable_subreaper() -> Result<(), String> {
    // An inherited SIG_IGN would auto-reap children and lose our PID witness.
    unsafe { libc::signal(libc::SIGCHLD, libc::SIG_DFL); }
    // Adopt grandchildren even if a signed package script creates a new session.
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } != 0 { return Err("Cannot supervise package-manager descendants.".into()); }
    Ok(())
}

fn nonblocking(pipe: &impl AsRawFd) -> Result<(), String> {
    let flags = unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 { return Err("Cannot supervise package-manager output.".into()); }
    Ok(())
}

fn drain(pipe: &mut impl Read, bytes: &mut Vec<u8>) -> Result<(), String> {
    let mut buffer = [0; 8192];
    for _ in 0..8 { // Return regularly to the wall-clock deadline.
        match pipe.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => { if bytes.len() + count > 1024 * 1024 { return Err("Package-manager output exceeded its limit.".into()); } bytes.extend_from_slice(&buffer[..count]); },
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
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
    let children_path = format!("/proc/self/task/{}/children", std::process::id());
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let children = std::fs::read_to_string(&children_path).map_err(|_| "Cannot inspect owned installer descendants.")?;
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
    let mut command = Command::new(tool.path());
    command.args(args).env_clear().env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("HOME", "/root").env("LANG", "C").env("LC_ALL", "C")
        .env("DPKG_DEB_THREADS_MAX", "1").current_dir("/")
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
    supervised(command, timeout)
}

fn supervised(mut command: Command, timeout: Duration) -> Result<Output, String> {
    let child = command.spawn().map_err(|_| "Cannot start the fixed package-manager tool.")?;
    let mut owned = OwnedChild(child);
    eprintln!("Recorde package task: pid={}, purpose=verified package metadata/install, ports=none", owned.0.id());
    let result = (|| {
        let mut stdout = owned.0.stdout.take().ok_or("Missing supervised stdout.")?;
        let mut stderr = owned.0.stderr.take().ok_or("Missing supervised stderr.")?;
        nonblocking(&stdout)?; nonblocking(&stderr)?;
        let deadline = Instant::now() + timeout;
        let mut out = Vec::new(); let mut err = Vec::new();
        loop {
            drain(&mut stdout, &mut out)?; drain(&mut stderr, &mut err)?;
            // WNOWAIT preserves our ownership witness until OwnedChild kills
            // the group and reaps it. Child::try_wait would reap too early.
            let mut status: libc::siginfo_t = unsafe { std::mem::zeroed() };
            if unsafe { libc::waitid(libc::P_PID, owned.0.id(), &mut status, libc::WEXITED | libc::WNOHANG | libc::WNOWAIT) } != 0 { return Err("Cannot inspect package-manager status.".into()); }
            if unsafe { status.si_pid() } != 0 {
                drain(&mut stdout, &mut out)?;
                return Ok(Output { success: status.si_code == libc::CLD_EXITED && unsafe { status.si_status() } == 0, stdout: out });
            }
            if Instant::now() >= deadline { return Err("Package-manager task timed out; inspect its installation status before retrying.".into()); }
            std::thread::sleep(Duration::from_millis(20));
        }
    })();
    drop(owned);
    let cleanup = reap_adopted();
    eprintln!("Recorde package task cleanup complete; ports=none requested");
    cleanup?;
    result
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
    fn timeout_reaps_a_descendant_that_created_its_own_session() {
        enable_subreaper().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let witness = temp.path().join("pid");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "process::tests::detached_fixture", "--ignored", "--nocapture"])
            .env("RECORDE_INSTALLER_TEST_PID", &witness).stdin(Stdio::null())
            .stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
        assert!(supervised(command, Duration::from_millis(500)).is_err());
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
