use super::witness::Witness;
use fs2::FileExt;
use std::path::{Path, PathBuf};

pub struct InstanceGuard {
    _file: std::fs::File,
    _pin: super::security::DirectoryPin,
    registration: Option<PathBuf>,
}
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if let Some(path) = &self.registration {
            let _ = std::fs::remove_file(path);
        }
    }
}
fn lock_file(root: &Path) -> Result<std::fs::File, String> {
    let path = root.join("instances.lock");
    super::security::reject_link(&path)?;
    super::security::open_lock(&path)
}
pub fn register() -> Result<InstanceGuard, String> {
    let root = super::persistence::root()?;
    register_at(&root)
}
fn register_at(root: &Path) -> Result<InstanceGuard, String> {
    let pin = super::security::pin_directory(root)?;
    let file = lock_file(root)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match FileExt::try_lock_shared(&file) {
            Ok(()) => break,
            Err(_) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50))
            }
            Err(_) => {
                return Err(
                    "An application update is in progress. Open Recorde after it finishes.".into(),
                )
            }
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    return Ok(InstanceGuard {
        _file: file,
        _pin: pin,
        registration: None,
    });
    #[cfg(any(windows, target_os = "linux"))]
    {
        let witness = Witness::current()?;
        let path = root.join(format!("instance-{}-{}.json", witness.pid, witness.created));
        let mut registration = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| "Cannot register application instance.")?;
        use std::io::Write;
        registration
            .write_all(
                &serde_json::to_vec(&witness).map_err(|_| "Cannot encode application identity.")?,
            )
            .and_then(|_| registration.sync_all())
            .map_err(|_| "Cannot save application identity.")?;
        Ok(InstanceGuard {
            _file: file,
            _pin: pin,
            registration: Some(path),
        })
    }
}
pub fn require_only_current() -> Result<(), String> {
    let root = super::persistence::root()?;
    let current = Witness::current()?;
    let mut count = 0;
    for entry in
        std::fs::read_dir(&root).map_err(|_| "Cannot inspect running Recorde instances.")?
    {
        let entry = entry.map_err(|_| "Cannot inspect running Recorde instances.")?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("instance-") || !name.ends_with(".json") {
            continue;
        }
        count += 1;
        if count > 10000 {
            return Err(
                "Too many application instance records; close other instances before updating."
                    .into(),
            );
        }
        use std::io::Read;
        let mut bytes = vec![];
        super::security::open_read(&entry.path())?
            .take(512)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read application instance identity.")?;
        let witness: Witness = serde_json::from_slice(&bytes)
            .map_err(|_| "Invalid application instance record; update refused.")?;
        if name != format!("instance-{}-{}.json", witness.pid, witness.created) {
            return Err("Application instance record does not match its identity.".into());
        }
        if witness.pid == current.pid && witness.created == current.created {
            continue;
        }
        if witness.alive()? {
            return Err("Close other Recorde instances before installing this update.".into());
        }
        let _ = std::fs::remove_file(entry.path());
    }
    Ok(())
}
pub fn exclusive() -> Result<InstanceGuard, String> {
    let root = super::persistence::root()?;
    let pin = super::security::pin_directory(&root)?;
    let file = lock_file(&root)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match FileExt::try_lock_exclusive(&file){Ok(())=>break,Err(error) if error.kind()==std::io::ErrorKind::WouldBlock && std::time::Instant::now()<deadline=>std::thread::sleep(std::time::Duration::from_millis(50)),Err(_)=>return Err("Another Recorde instance is running or its guard cannot be acquired; no executable was replaced.".into())}
    }
    Ok(InstanceGuard {
        _file: file,
        _pin: pin,
        registration: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn another_instance_shared_lock_prevents_exclusive_apply() {
        let directory = super::super::security::fixture_dir();
        let first = lock_file(directory.path()).unwrap();
        let second = lock_file(directory.path()).unwrap();
        FileExt::try_lock_shared(&first).unwrap();
        FileExt::try_lock_shared(&second).unwrap();
        let helper = lock_file(directory.path()).unwrap();
        assert!(FileExt::try_lock_exclusive(&helper).is_err());
        drop(first);
        assert!(FileExt::try_lock_exclusive(&helper).is_err());
        drop(second);
        FileExt::try_lock_exclusive(&helper).unwrap();
    }
}

#[cfg(all(test, windows))]
#[test]
fn instance_retains_parent_identity_until_guard_is_released() {
    let directory = super::security::fixture_dir();
    let parent = directory.path().join("recorde");
    let root = parent.join("updates");
    super::security::create_private_directory(&root).unwrap();
    let guard = register_at(&root).unwrap();
    let renamed_parent = directory.path().join("renamed");
    assert!(std::fs::rename(&parent, &renamed_parent).is_err());
    assert!(std::fs::rename(&root, parent.join("renamed-updates")).is_err());
    drop(guard);
    std::fs::rename(&parent, &renamed_parent).unwrap();
    std::fs::rename(&renamed_parent, &parent).unwrap();
}
