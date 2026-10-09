//! Exercise the actual executable without starting a window or accessing user accounts.
#![cfg(target_os = "linux")]
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn executable_helper_delegates_to_git_and_rejects_unsafe_protocols() {
    let dir = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();
    repo.config().unwrap().set_str("credential.helper",
        "!f() { test \"$1\" = get && printf 'username=fixture-user\\npassword=fixture-only\\n'; }; f").unwrap();
    for (protocol, expected) in [("https", "password=fixture-only"), ("http", "quit=true")] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_recorde"))
            .args(["--credential-helper", "get"])
            .env("XDG_CONFIG_HOME", dir.path().join("config"))
            .env("XDG_DATA_HOME", dir.path().join("data"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("RECORDE_CREDENTIAL_REPO", dir.path())
            .env_remove("RECORDE_HELPER_ACTIVE")
            // Simulate the parent's helper override; fallback must remove it.
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "credential.helper")
            .env("GIT_CONFIG_VALUE_0", "!false")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        write!(
            child.stdin.take().unwrap(),
            "protocol={protocol}\nhost=fixture.invalid\npath=org/repo.git\n\n"
        )
        .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains(expected));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("fixture-only"));
    }
}
