//! Per-user update preferences and monotonic offer checkpoints.
use recorde_update_signing::{Checkpoint, Manifest, Target};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub automatic_checks: bool,
    pub account: Option<String>,
    pub channel: String,
    pub last_check: u64,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            automatic_checks: false,
            account: None,
            channel: "stable".into(),
            last_check: 0,
        }
    }
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct State {
    preferences: Preferences,
    checkpoints: BTreeMap<String, Checkpoint>,
}

pub fn root() -> Result<PathBuf, String> {
    let path = dirs::data_local_dir()
        .ok_or("Cannot locate private update storage.")?
        .join("recorde/updates");
    for ancestor in path.ancestors() {
        super::security::reject_link(ancestor)?;
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(&path)
        .map_err(|_| "Cannot create private update storage.")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "Cannot protect update storage.")?;
    }
    super::security::owned_directory(&path)?;
    Ok(path)
}
fn transact<T>(
    root: &Path,
    mutate: bool,
    operation: impl FnOnce(&mut State) -> Result<T, String>,
) -> Result<T, String> {
    use fs2::FileExt;
    let _directory_pin = super::security::pin_directory(root)?;
    let lock_path = root.join("state.lock");
    super::security::reject_link(&lock_path)?;
    let lock = super::security::open_lock(&lock_path)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    loop {
        let result = if mutate {
            FileExt::try_lock_exclusive(&lock)
        } else {
            FileExt::try_lock_shared(&lock)
        };
        match result {
            Ok(()) => break,
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            Err(_) => {
                return Err(
                    "Another Recorde instance is updating its state. Try again shortly.".into(),
                )
            }
        }
    }
    let path = root.join("state.json");
    super::security::reject_link(&path)?;
    let mut state = match std::fs::symlink_metadata(&path) {
        Ok(_) => {
            let file = super::security::open_read(&path)?;
            let mut bytes = vec![];
            file.take(64 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "Cannot read update state.")?;
            if bytes.len() > 64 * 1024 {
                return Err("Update state exceeds its size limit; it was preserved.".into());
            }
            serde_json::from_slice(&bytes)
                .map_err(|_| "Update state is invalid; it was preserved.")?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => State::default(),
        Err(_) => return Err("Cannot read update state; it was preserved.".into()),
    };
    let result = operation(&mut state)?;
    if mutate {
        let bytes = serde_json::to_vec(&state).map_err(|_| "Cannot serialize update state.")?;
        let mut staged =
            tempfile::NamedTempFile::new_in(root).map_err(|_| "Cannot stage update state.")?;
        staged
            .write_all(&bytes)
            .and_then(|_| staged.as_file().sync_all())
            .map_err(|_| "Cannot save update state.")?;
        staged
            .persist(&path)
            .map_err(|_| "Cannot atomically save update state.")?;
        #[cfg(unix)]
        std::fs::File::open(root)
            .and_then(|f| f.sync_all())
            .map_err(|_| "Cannot flush update state directory.")?;
    }
    Ok(result)
}
pub fn preferences() -> Result<Preferences, String> {
    transact(&root()?, false, |s| Ok(s.preferences.clone()))
}
pub fn save_preferences(preferences: &Preferences) -> Result<(), String> {
    if !matches!(preferences.channel.as_str(), "stable" | "preview") {
        return Err("Invalid update channel.".into());
    }
    transact(&root()?, true, |s| {
        s.preferences = preferences.clone();
        Ok(())
    })
}
pub fn accept(
    root: &Path,
    manifest: &Manifest,
    target: &Target,
    current: &str,
    now: u64,
) -> Result<(), String> {
    transact(root, true, |state| {
        recorde_update_signing::validate_offer(
            manifest,
            target,
            current,
            now,
            state.checkpoints.get(&target.checkpoint_key()),
        )?;
        state.checkpoints.insert(
            target.checkpoint_key(),
            Checkpoint {
                version: manifest.version.clone(),
                issued_at: manifest.issued_at,
            },
        );
        Ok(())
    })
}
/// Keep install results available after the old process has exited. Only inspect
/// private staging directories; no updater subprocess is started by this read.
pub fn latest_outcome() -> Option<String> {
    let root = root().ok()?;
    let mut newest = None;
    for entry in std::fs::read_dir(&root).ok()?.flatten() {
        if !entry.file_name().to_string_lossy().starts_with("release-") {
            continue;
        }
        let directory = entry.path();
        if super::security::owned_directory(&directory).is_err() {
            continue;
        }
        let path = directory.join("outcome.txt");
        if super::security::reject_link(&path).is_err() {
            continue;
        }
        let Ok(file) = super::security::open_read(&path) else {
            continue;
        };
        let Ok(modified) = file.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        let mut text = String::new();
        if file.take(4097).read_to_string(&mut text).is_err() || text.len() > 4096 {
            continue;
        }
        if newest.as_ref().is_none_or(|(time, _)| modified > *time) {
            newest = Some((modified, text));
        }
    }
    newest.map(|(_, text)| text)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest(version: &str, issued_at: u64) -> Manifest {
        Manifest {
            schema: 1,
            key_id: "fixture".into(),
            channel: "stable".into(),
            version: version.into(),
            issued_at,
            expires_at: issued_at + 1000,
            platform: "linux".into(),
            arch: "x86_64".into(),
            package_kind: "portable".into(),
            artifact_name: "recorde".into(),
            url: recorde_update_signing::release_url(version, "recorde").unwrap(),
            size: 1,
            sha256: "a".repeat(64),
        }
    }
    fn target() -> Target {
        Target {
            platform: "linux".into(),
            arch: "x86_64".into(),
            package_kind: "portable".into(),
            channel: "stable".into(),
        }
    }
    #[test]
    fn checkpoint_survives_reopen_and_rejects_rollback_older_issuance_and_corruption() {
        let directory =
            super::super::security::private_tempdir(&std::env::temp_dir(), "recorde-update-test-")
                .unwrap();
        accept(
            directory.path(),
            &manifest("2.0.0", 100),
            &target(),
            "1.0.0",
            200,
        )
        .unwrap();
        assert!(accept(
            directory.path(),
            &manifest("1.9.0", 150),
            &target(),
            "1.0.0",
            200
        )
        .unwrap_err()
        .contains("rollback"));
        assert!(accept(
            directory.path(),
            &manifest("2.1.0", 99),
            &target(),
            "1.0.0",
            200
        )
        .unwrap_err()
        .contains("rollback"));
        accept(
            directory.path(),
            &manifest("2.0.0", 101),
            &target(),
            "1.0.0",
            200,
        )
        .unwrap();
        std::fs::write(directory.path().join("state.json"), "broken").unwrap();
        assert!(accept(
            directory.path(),
            &manifest("3.0.0", 200),
            &target(),
            "1.0.0",
            200
        )
        .unwrap_err()
        .contains("preserved"));
        assert_eq!(
            std::fs::read(directory.path().join("state.json")).unwrap(),
            b"broken"
        );
    }
    #[test]
    fn held_state_lock_blocks_mutation() {
        use fs2::FileExt;
        let directory =
            super::super::security::private_tempdir(&std::env::temp_dir(), "recorde-update-test-")
                .unwrap();
        let lock =
            super::super::security::create_private(&directory.path().join("state.lock")).unwrap();
        lock.lock_exclusive().unwrap();
        assert!(accept(
            directory.path(),
            &manifest("2.0.0", 100),
            &target(),
            "1.0.0",
            200
        )
        .unwrap_err()
        .contains("Another Recorde"));
    }
    #[cfg(unix)]
    #[test]
    fn symlinked_state_is_preserved() {
        let directory =
            super::super::security::private_tempdir(&std::env::temp_dir(), "recorde-update-test-")
                .unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::os::unix::fs::symlink(outside.path(), directory.path().join("state.json")).unwrap();
        assert!(accept(
            directory.path(),
            &manifest("2.0.0", 100),
            &target(),
            "1.0.0",
            200
        )
        .is_err());
        assert_eq!(std::fs::metadata(outside.path()).unwrap().len(), 0);
    }
}
