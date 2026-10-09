use std::path::PathBuf;

pub struct SshDiagnostics {
    pub agent_running: bool,
    pub agent_keys: Vec<String>,        // lines from ssh-add -l
    pub key_files: Vec<(String, bool)>, // (name, exists)
}

pub fn diagnose() -> SshDiagnostics {
    let mut command = crate::git::process::background_command("ssh-add");
    command.arg("-l");
    let agent_output = crate::git::process::run(
        &mut command,
        &std::sync::atomic::AtomicBool::new(false),
        std::time::Duration::from_secs(10),
    );

    let (agent_running, agent_keys) = match agent_output {
        Ok(out) if out.status.success() => {
            let keys = String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(str::to_string)
                .collect();
            (true, keys)
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            // Exit 1 with "The agent has no identities" = agent running but empty
            if out.status.code() == Some(1) && !stderr.contains("Could not") {
                (true, Vec::new())
            } else {
                (false, Vec::new())
            }
        }
        Err(_) => (false, Vec::new()),
    };

    let key_files = key_file_status();

    SshDiagnostics {
        agent_running,
        agent_keys,
        key_files,
    }
}

fn key_file_status() -> Vec<(String, bool)> {
    let home = std::env::var("HOME").unwrap_or_default();
    let ssh_dir = PathBuf::from(&home).join(".ssh");
    ["id_ed25519", "id_ecdsa", "id_rsa", "id_ed25519_sk"]
        .iter()
        .map(|name| (name.to_string(), ssh_dir.join(name).exists()))
        .collect()
}
