//! Verified installation handoff. Helpers have a bounded lifetime and never kill the old application.
use super::{security, witness::Witness};
use recorde_update_signing::{self as protocol, Target};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: u32,
    parent: Witness,
    target: PathBuf,
    target_kind: Target,
    previous_sha256: String,
    helper_sha256: String,
    envelope: Vec<u8>,
    nonce: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    schema: u32,
    kind: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingSwap {
    schema: u32,
    target: PathBuf,
    backup: PathBuf,
    attempt: PathBuf,
    nonce: String,
}

pub fn installation() -> Result<(PathBuf, String), String> {
    #[cfg(target_os = "linux")]
    if let Some(path) = std::env::var_os("APPIMAGE") {
        let path = PathBuf::from(path);
        let appdir = std::env::var_os("APPDIR")
            .map(PathBuf::from)
            .ok_or("Inherited APPIMAGE has no APPDIR; install manually.")?;
        let current = std::env::current_exe().map_err(|_| "Cannot locate AppImage executable.")?;
        validate_appimage_location(&path, &appdir, &current)?;
        return Ok((path, "appimage".into()));
    }
    let exe = std::env::current_exe().map_err(|_| "Cannot locate this installation.")?;
    let marker = exe
        .parent()
        .ok_or("Installation has no parent directory.")?
        .join("recorde-install.json");
    security::reject_link(&marker)?;
    let marker:Marker=serde_json::from_slice(&read(&marker,4096)?).map_err(|_|"Installation marker is unavailable or invalid. Verified downloads remain available; install manually.")?;
    if marker.schema != 1 || !matches!(marker.kind.as_str(), "portable" | "msi" | "deb") {
        return Err("Unknown installation kind. Install the verified download manually.".into());
    }
    Ok((exe, marker.kind))
}
#[cfg(target_os = "linux")]
fn validate_appimage_location(image: &Path, appdir: &Path, current: &Path) -> Result<(), String> {
    if !image.is_absolute() || !appdir.is_absolute() {
        return Err("AppImage paths must be absolute.".into());
    }
    security::reject_link(image)?;
    let mount = appdir
        .canonicalize()
        .map_err(|_| "Cannot identify AppImage mount.")?;
    let executable = current
        .canonicalize()
        .map_err(|_| "Cannot identify running AppImage executable.")?;
    if mount == Path::new("/") || !executable.starts_with(&mount) || executable == mount {
        return Err("APPIMAGE belongs to a different application. Install manually.".into());
    }
    let mountinfo = std::fs::read_to_string("/proc/self/mountinfo")
        .map_err(|_| "Cannot verify AppImage mount.")?;
    let image = image
        .canonicalize()
        .map_err(|_| "Cannot identify AppImage file.")?;
    if !appimage_mount_matches(&mountinfo, &mount, &image) {
        return Err(
            "APPDIR and APPIMAGE do not identify the same running FUSE image. Install manually."
                .into(),
        );
    }
    Ok(())
}
#[cfg(target_os = "linux")]
fn appimage_mount_matches(mountinfo: &str, mount: &Path, image: &Path) -> bool {
    let escape = |path: &Path| {
        path.to_string_lossy()
            .replace('\\', "\\134")
            .replace(' ', "\\040")
            .replace('\t', "\\011")
            .replace('\n', "\\012")
    };
    let mount = escape(mount);
    let image = escape(image);
    mountinfo.lines().any(|line| {
        line.split_whitespace().nth(4) == Some(mount.as_str())
            && line.split_once(" - ").is_some_and(|(_, tail)| {
                let mut fields = tail.split_whitespace();
                let kind = fields.next().unwrap_or_default();
                (kind == "fuse" || kind.starts_with("fuse."))
                    && fields.next() == Some(image.as_str())
            })
    })
}
fn read(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    security::reject_link(path)?;
    let mut bytes = vec![];
    security::open_read(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read update file.")?;
    if bytes.len() > limit {
        return Err("Update file exceeds its size limit.".into());
    }
    Ok(bytes)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    write_new_with_flush(path, bytes, |file| {
        file.sync_all()
            .map_err(|_| "Cannot flush update handoff file.".into())
    })
}
fn write_new_with_flush(
    path: &Path,
    bytes: &[u8],
    flush: impl FnOnce(&std::fs::File) -> Result<(), String>,
) -> Result<(), String> {
    let parent = path.parent().ok_or("Update handoff has no directory.")?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| "Cannot stage update handoff file.")?;
    temporary
        .write_all(bytes)
        .map_err(|_| "Cannot write update handoff file.")?;
    flush(temporary.as_file())?;
    temporary
        .persist_noclobber(path)
        .map_err(|_| "Cannot publish update handoff file without replacing existing data.")?;
    // Publication is the commit point. Never report a later flush error as a
    // failed signal: its recipient may already be loading new-version state.
    #[cfg(unix)]
    if std::fs::File::open(parent)
        .and_then(|file| file.sync_all())
        .is_err()
    {
        log::warn!("Published update handoff, but directory flush failed");
    }
    Ok(())
}
pub fn startup_recovery_check() -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|_| "Cannot locate application for update recovery check.")?;
    let target = installation()
        .map(|(target, _)| target)
        .unwrap_or(executable);
    check_pending_swap(&target)
}
fn check_pending_swap(target: &Path) -> Result<(), String> {
    let directory = target.parent().ok_or("Installation has no directory.")?;
    let path = directory.join(".recorde-update-pending.json");
    if matches!(std::fs::symlink_metadata(&path),Err(error) if error.kind()==std::io::ErrorKind::NotFound)
    {
        return Ok(());
    }
    let pending:PendingSwap=serde_json::from_slice(&read(&path,16384)?).map_err(|_|format!("An invalid update recovery marker remains at {}. User state was not opened; inspect the installation before retrying.",path.display()))?;
    if pending.schema != 1
        || pending.target != target
        || pending.backup.parent() != Some(directory)
        || pending.nonce.len() != 64
        || !pending.nonce.bytes().all(|b| b.is_ascii_hexdigit())
        || pending.backup.file_name().and_then(|v| v.to_str())
            != Some(format!(".recorde-backup-{}", pending.nonce).as_str())
    {
        return Err(format!(
            "Update recovery marker at {} has an invalid identity. User state was not opened.",
            path.display()
        ));
    }
    Err(format!("An application update was interrupted. User state was not opened. The previous executable may be retained at {}. Review that backup and the attempt at {} before manually completing or restoring the installation; then remove {}. No automatic restoration was attempted.",pending.backup.display(),pending.attempt.display(),path.display()))
}
/// Remove only duplicate payloads from successful attempts whose helper has
/// exited. Keep receipts and every failed/interrupted attempt for recovery.
pub fn cleanup_confirmed_attempts() {
    let Ok(root) = super::persistence::root() else {
        return;
    };
    let Ok(keys) = super::keys() else { return };
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("release-apply-")
        {
            continue;
        }
        let _ = cleanup_confirmed_attempt(&entry.path(), &keys);
    }
}
fn cleanup_confirmed_attempt(directory: &Path, keys: &protocol::KeySet) -> Result<(), String> {
    let _pin = security::pin_directory(directory)?;
    if read(&directory.join("outcome.txt"), 4096)? != b"Update applied successfully." {
        return Ok(());
    }
    let helper: Witness =
        serde_json::from_slice(&read(&directory.join("helper-identity.json"), 512)?)
            .map_err(|_| "Invalid completed helper identity.")?;
    if helper.alive()? {
        return Ok(());
    }
    let request: Request =
        serde_json::from_slice(&read(&directory.join("apply.json"), 128 * 1024)?)
            .map_err(|_| "Invalid completed update receipt.")?;
    let manifest = protocol::verify(&request.envelope, keys)?;
    for name in [
        manifest.artifact_name.as_str(),
        if cfg!(windows) {
            "recorde-update-helper.exe"
        } else {
            "recorde-update-helper"
        },
    ] {
        let path = directory.join(name);
        if !path.exists() {
            continue;
        }
        security::reject_link(&path)?;
        if !std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_file()) {
            continue;
        }
        std::fs::remove_file(&path)
            .map_err(|_| "Cannot remove completed update duplicate payload.")?;
    }
    Ok(())
}
fn digest(path: &Path) -> Result<String, String> {
    protocol::digest(security::open_read(path)?).map(|(_, hash)| hash)
}
fn stage_attempt(
    root: &Path,
    source: &Path,
    envelope: &[u8],
    manifest: &protocol::Manifest,
) -> Result<tempfile::TempDir, String> {
    let attempt = security::private_tempdir(root, "release-apply-")?;
    let mut artifact = security::create_private(&attempt.path().join(&manifest.artifact_name))?;
    protocol::copy_verified(security::open_read(source)?, &mut artifact, manifest)?;
    artifact
        .sync_all()
        .map_err(|_| "Cannot flush this update attempt.")?;
    write_new(&attempt.path().join("envelope.json"), envelope)?;
    Ok(attempt)
}
fn checked_request(path: &Path) -> Result<(PathBuf, Request, protocol::Manifest), String> {
    let root = super::persistence::root()?;
    let directory = path.parent().ok_or("Invalid update handoff path.")?;
    if directory.parent() != Some(root.as_path())
        || path.file_name().and_then(|s| s.to_str()) != Some("apply.json")
    {
        return Err("Update handoff is outside private staging storage.".into());
    }
    security::owned_directory(directory)?;
    let request: Request =
        serde_json::from_slice(&read(path, 128 * 1024)?).map_err(|_| "Invalid update handoff.")?;
    if request.schema != 1
        || request.nonce.len() != 64
        || !request.nonce.bytes().all(|b| b.is_ascii_hexdigit())
        || !request.target.is_absolute()
    {
        return Err("Invalid update handoff identity.".into());
    }
    if digest(&std::env::current_exe().map_err(|_| "Cannot identify update helper.")?)?
        != request.helper_sha256
        || (request.target_kind.package_kind != "appimage"
            && request.helper_sha256 != request.previous_sha256)
    {
        return Err("Update helper does not match the captured installation executable.".into());
    }
    let manifest = protocol::verify(&request.envelope, &super::keys()?)?;
    super::persistence::accept(
        &root,
        &manifest,
        &request.target_kind,
        env!("CARGO_PKG_VERSION"),
        super::now(),
    )?;
    Ok((directory.to_path_buf(), request, manifest))
}

/// Called from the UI after operations are idle and drafts/config have been flushed.
pub fn start(directory: &Path, envelope: &[u8], target_kind: &Target) -> Result<(), String> {
    super::instance::require_only_current()?;
    let _directory_pin = security::pin_directory(directory)?;
    let root = super::persistence::root()?;
    if directory.parent() != Some(root.as_path()) {
        return Err("Update is outside private staging storage.".into());
    }
    let (target, kind) = installation()?;
    if kind != target_kind.package_kind {
        return Err("Installation kind changed since download.".into());
    }
    if matches!(kind.as_str(), "portable" | "appimage") {
        security::owned_directory(target.parent().ok_or("Missing installation directory.")?)?;
        security::reject_link(&target)?;
    }
    let manifest = protocol::verify(envelope, &super::keys()?)?;
    super::persistence::accept(
        &root,
        &manifest,
        target_kind,
        env!("CARGO_PKG_VERSION"),
        super::now(),
    )?;
    let attempt = stage_attempt(
        &root,
        &directory.join(&manifest.artifact_name),
        envelope,
        &manifest,
    )?;
    let directory = attempt.path();
    let nonce = protocol::digest(std::io::Cursor::new(
        format!(
            "{}:{}:{}",
            std::process::id(),
            super::now(),
            directory.display()
        )
        .into_bytes(),
    ))?
    .1;
    let request = Request {
        schema: 1,
        parent: Witness::current()?,
        target,
        target_kind: target_kind.clone(),
        previous_sha256: String::new(),
        helper_sha256: digest(
            &std::env::current_exe().map_err(|_| "Cannot identify running executable.")?,
        )?,
        envelope: envelope.to_vec(),
        nonce,
    };
    let request = Request {
        previous_sha256: digest(&request.target)?,
        ..request
    };
    let path = directory.join("apply.json");
    write_new(
        &path,
        &serde_json::to_vec(&request).map_err(|_| "Cannot encode update handoff.")?,
    )?;
    let current = std::env::current_exe().map_err(|_| "Cannot locate update helper source.")?;
    let helper = directory.join(if cfg!(windows) {
        "recorde-update-helper.exe"
    } else {
        "recorde-update-helper"
    });
    let mut helper_file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&helper)
        .map_err(|_| "Cannot create fresh update helper.")?;
    let mut current_file = security::open_read(&current)?;
    std::io::copy(&mut current_file, &mut helper_file)
        .map_err(|_| "Cannot stage update helper.")?;
    helper_file
        .set_permissions(
            current_file
                .metadata()
                .map_err(|_| "Cannot inspect helper permissions.")?
                .permissions(),
        )
        .and_then(|_| helper_file.sync_all())
        .map_err(|_| "Cannot flush update helper.")?;
    drop(helper_file);
    drop(current_file);
    if request.helper_sha256 != digest(&helper)? {
        return Err("Staged update helper failed verification.".into());
    }
    let mut command = crate::git::process::background_command(&helper);
    command
        .arg("--apply-update")
        .arg(&path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = command.spawn().map_err(|_| "Cannot start update helper.")?;
    let ledger=serde_json::to_vec(&serde_json::json!({"pid":child.id(),"purpose":"verified update apply; bounded 30 minute lifetime","ports":[],"executable":helper})).map_err(|_|"Cannot encode helper process ledger.")?;
    if let Err(error) = write_new(&directory.join("helper-process.json"), &ledger) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    // The helper intentionally survives application exit, supervises its own children,
    // waits at most 90 seconds for this exact parent identity, and records its outcome.
    let ready = directory.join("ready");
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if read(&ready, 128).is_ok_and(|v| v == request.nonce.as_bytes()) {
            let _ = attempt.keep();
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return Ok(());
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                return Err(
                    "Update helper exited before handoff. The application remains open.".into(),
                )
            }
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Cannot inspect update helper; the application remains open.".into());
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    Err("Update helper did not acknowledge handoff. The application remains open.".into())
}

pub fn run(path: &Path) -> Result<(), String> {
    let (directory, request, manifest) = checked_request(path)?;
    let _directory_pin = security::pin_directory(&directory)?;
    let mut exclusive_instance = None;
    let result = (|| {
        if !request.parent.alive()? {
            return Err("Update parent identity is no longer valid.".into());
        }
        write_new(
            &directory.join("helper-identity.json"),
            &serde_json::to_vec(&Witness::current()?)
                .map_err(|_| "Cannot encode helper identity.")?,
        )?;
        write_new(&directory.join("ready"), request.nonce.as_bytes())?;
        let deadline = Instant::now() + Duration::from_secs(90);
        while request.parent.alive()? {
            if Instant::now() >= deadline {
                return Err("Application did not exit; no files were changed.".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        exclusive_instance = Some(super::instance::exclusive()?);
        security::reject_link(&request.target)?;
        if digest(&request.target)? != request.previous_sha256 {
            return Err("Installation changed during handoff; no files were changed.".into());
        }
        super::persistence::accept(
            &super::persistence::root()?,
            &manifest,
            &request.target_kind,
            env!("CARGO_PKG_VERSION"),
            super::now(),
        )?;
        let artifact = directory.join(&manifest.artifact_name);
        security::reject_link(&artifact)?;
        let locked_artifact = security::open_read(&artifact)?;
        protocol::copy_verified(
            locked_artifact
                .try_clone()
                .map_err(|_| "Cannot verify staged artifact.")?,
            std::io::sink(),
            &manifest,
        )?;
        match request.target_kind.package_kind.as_str() {
            "portable" | "appimage" => {
                replace_and_restart(&directory, &request, &manifest, &artifact)
            }
            "msi" | "deb" => {
                let installation = if request.target_kind.package_kind == "msi" {
                    install_msi(&artifact)
                } else {
                    install_deb(&directory.join("envelope.json"), &artifact)
                };
                // Installers own their transactional recovery. Reopen whichever
                // installed executable remains so the outcome can be shown.
                installation?;
                let launched = crate::git::process::background_command(&request.target).spawn();
                if let Ok(child) = &launched {
                    log::info!(
                        "Update installer handoff reopened application: pid={}, ports=none",
                        child.id()
                    );
                }
                launched.map_err(|_| {
                    "Installation completed, but Recorde could not restart. Open it manually."
                })?;
                Ok(())
            }
            _ => Err("This package supports verified download only. Install it manually.".into()),
        }
    })();
    let outcome = match &result {
        Ok(()) => "Update applied successfully.".to_owned(),
        Err(error) => error.clone(),
    };
    let _ = write_new(&directory.join("outcome.txt"), outcome.as_bytes());
    // The new application loads this receipt after acquiring its shared lock.
    drop(exclusive_instance);
    if result.is_err()
        && may_relaunch_original(&request)
        && super::instance::require_only_current().is_ok()
    {
        if let Ok(child) = crate::git::process::background_command(&request.target).spawn() {
            log::info!(
                "Update failure reopened original application: pid={}, ports=none",
                child.id()
            );
        }
    }
    result
}
fn may_relaunch_original(request: &Request) -> bool {
    matches!(
        request.target_kind.package_kind.as_str(),
        "portable" | "appimage"
    ) && request.parent.alive() == Ok(false)
        && digest(&request.target).is_ok_and(|hash| hash == request.previous_sha256)
}
fn replace_and_restart(
    directory: &Path,
    request: &Request,
    manifest: &protocol::Manifest,
    artifact: &Path,
) -> Result<(), String> {
    replace_with_timeout(
        directory,
        request,
        manifest,
        artifact,
        Duration::from_secs(90),
    )
}
fn replace_with_timeout(
    directory: &Path,
    request: &Request,
    manifest: &protocol::Manifest,
    artifact: &Path,
    timeout: Duration,
) -> Result<(), String> {
    let parent = request
        .target
        .parent()
        .ok_or("Missing installation directory.")?;
    let _installation_pin = security::pin_directory(parent)?;
    security::reject_link(&request.target)?;
    if request.target_kind.package_kind == "portable" {
        let marker: Marker =
            serde_json::from_slice(&read(&parent.join("recorde-install.json"), 4096)?)
                .map_err(|_| "Installation marker changed during handoff.")?;
        if marker.schema != 1 || marker.kind != "portable" {
            return Err("Installation marker changed during handoff.".into());
        }
    }
    let metadata = security::open_read(&request.target)?
        .metadata()
        .map_err(|_| "Cannot inspect installation executable.")?;
    #[cfg(windows)]
    security::owned_file(&request.target)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o6022 != 0
            || metadata.nlink() != 1
        {
            return Err(
                "Executable ownership, permissions or hard links prevent safe replacement.".into(),
            );
        }
    }
    let mut candidate = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| "Cannot stage replacement beside the executable.")?;
    protocol::copy_verified(security::open_read(artifact)?, &mut candidate, manifest)?;
    candidate
        .as_file()
        .set_permissions(metadata.permissions())
        .and_then(|_| candidate.as_file().sync_all())
        .map_err(|_| "Cannot prepare replacement executable.")?;
    let candidate = candidate.into_temp_path();
    let backup = parent.join(format!(".recorde-backup-{}", request.nonce));
    if backup.exists() {
        return Err("An update backup already exists; it was preserved.".into());
    }
    check_pending_swap(&request.target)?;
    let pending_path = parent.join(".recorde-update-pending.json");
    let pending = PendingSwap {
        schema: 1,
        target: request.target.clone(),
        backup: backup.clone(),
        attempt: directory.to_owned(),
        nonce: request.nonce.clone(),
    };
    write_new(
        &pending_path,
        &serde_json::to_vec(&pending).map_err(|_| "Cannot encode update recovery marker.")?,
    )?;
    #[cfg(unix)]
    std::fs::File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|_| "Cannot durably record pending update; no executable was replaced.")?;
    if let Err(error) = replace(&request.target, &candidate, &backup) {
        // ReplaceFileW can fail after moving the original to its backup name.
        // Restore only the backup whose digest was captured before handoff.
        if backup.exists() && verified_backup(&backup, &request.previous_sha256)? {
            if request.target.exists() {
                rollback(&request.target, &backup)
                    .map_err(|error| format!("{error} Backup retained at {}.", backup.display()))?;
            } else {
                restore_missing_target(&backup, &request.target)?;
            }
        }
        if digest(&request.target).is_ok_and(|hash| hash == request.previous_sha256) {
            clear_pending(&pending_path)?;
        }
        return Err(format!(
            "{error} Check the preserved installation and backup before retrying."
        ));
    }
    match launch_with_timeout(&request.target, directory, &request.nonce, timeout) {
        Err(error)=>{
            if !verified_backup(&backup,&request.previous_sha256)? {
                return Err(format!("Update failed and backup verification failed. Backup retained at {} for manual recovery.",backup.display()));
            }
            rollback(&request.target, &backup).map_err(|error|format!("{error} Backup retained at {}.",backup.display()))?;
            clear_pending(&pending_path)?;
            return Err(format!("{error} The previous executable was restored."));
        }
        Ok(false)=>return Err(format!("Startup was released, but the application exited or could not be inspected. Backup retained at {}. Review the interrupted update before reopening Recorde; no automatic rollback was attempted after user-state access was released.",backup.display())),
        Ok(true)=>{},
    }
    std::fs::remove_file(&backup)
        .map_err(|_| "Update succeeded; the old executable backup could not be removed.")?;
    clear_pending(&pending_path)?;
    Ok(())
}
fn verified_backup(path: &Path, expected: &str) -> Result<bool, String> {
    digest(path).map(|hash| hash == expected).map_err(|error| {
        format!(
            "{error} The executable backup at {} was retained for manual recovery.",
            path.display()
        )
    })
}
fn clear_pending(path: &Path) -> Result<(), String> {
    std::fs::remove_file(path).map_err(|_|format!("Update recovery marker {} could not be cleared. Inspect the installation before reopening Recorde.",path.display()))
}
fn restore_missing_target(backup: &Path, target: &Path) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match std::fs::rename(backup,target){Ok(())=>return Ok(()),Err(_) if Instant::now()<deadline=>std::thread::sleep(Duration::from_millis(50)),Err(_)=>return Err(format!("Replacement failed. Verified executable backup retained at {} for manual recovery.",backup.display()))}
    }
}
#[cfg(unix)]
fn replace(target: &Path, candidate: &Path, backup: &Path) -> Result<(), String> {
    std::fs::hard_link(target, backup).map_err(|_| "Cannot preserve the old executable.")?;
    if std::fs::rename(candidate, target).is_err() {
        let _ = std::fs::remove_file(backup);
        return Err("Atomic executable replacement failed.".into());
    }
    if std::fs::File::open(target.parent().unwrap())
        .and_then(|f| f.sync_all())
        .is_err()
    {
        let _ = std::fs::rename(backup, target);
        return Err("Cannot flush executable replacement; restoration was attempted.".into());
    }
    Ok(())
}
#[cfg(windows)]
fn replace(target: &Path, candidate: &Path, backup: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    let wide = |p: &Path| {
        p.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::ReplaceFileW(
            wide(target).as_ptr(),
            wide(candidate).as_ptr(),
            wide(backup).as_ptr(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    } == 0
    {
        return Err("Atomic executable replacement failed.".into());
    }
    Ok(())
}
#[cfg(unix)]
fn rollback(target: &Path, backup: &Path) -> Result<(), String> {
    std::fs::rename(backup, target)
        .map_err(|_| "Cannot restore the executable backup; restore it manually.".into())
}
#[cfg(windows)]
fn rollback(target: &Path, backup: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    let wide = |p: &Path| {
        p.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::ReplaceFileW(
            wide(target).as_ptr(),
            wide(backup).as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    } == 0
    {
        return Err("Cannot restore executable backup; restore it manually.".into());
    }
    Ok(())
}
fn launch_with_timeout(
    target: &Path,
    directory: &Path,
    nonce: &str,
    timeout: Duration,
) -> Result<bool, String> {
    use command_group::CommandGroup;
    let mut command = crate::git::process::background_command(target);
    command
        .arg("--update-receipt")
        .arg(directory.join("apply.json"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut group = command.group();
    #[cfg(windows)]
    group.creation_flags(0x08000000).kill_on_drop(false);
    let mut child = group
        .spawn()
        .map_err(|_| "Updated application could not start.")?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if read(&directory.join("healthy"), 128).is_ok_and(|v| v == nonce.as_bytes()) {
            if let Err(error) = write_new(&directory.join("continue"), nonce.as_bytes()) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
            // No rollback is permitted after continue is published: the child
            // may already be migrating user state. Preserve its backup if its
            // process has exited or cannot be inspected at this commit point.
            return Ok(matches!(child.try_wait(), Ok(None)));
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Updated application exited before startup completed.".into());
            }
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Cannot supervise updated application.".into());
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    Err("Updated application did not complete startup in time.".into())
}
/// Acknowledge after renderer creation, before loading or migrating user state.
pub fn acknowledge(path: &Path) -> Result<(), String> {
    let root = super::persistence::root()?;
    let directory = path.parent().ok_or("Invalid update receipt.")?;
    if directory.parent() != Some(root.as_path()) {
        return Err("Update receipt is outside private storage.".into());
    }
    security::owned_directory(directory)?;
    let request: Request =
        serde_json::from_slice(&read(path, 128 * 1024)?).map_err(|_| "Invalid update receipt.")?;
    if path.file_name().and_then(|s| s.to_str()) != Some("apply.json")
        || request.schema != 1
        || request.nonce.len() != 64
        || !request.nonce.bytes().all(|b| b.is_ascii_hexdigit())
        || !request.target.is_absolute()
    {
        return Err("Invalid update receipt identity.".into());
    }
    let manifest = protocol::verify(&request.envelope, &super::keys()?)?;
    if manifest.version != env!("CARGO_PKG_VERSION") || digest(&request.target)? != manifest.sha256
    {
        return Err("Updated installation does not match its receipt.".into());
    }
    write_new(&directory.join("healthy"), request.nonce.as_bytes())?;
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        if read(&directory.join("continue"), 128)
            .is_ok_and(|bytes| bytes == request.nonce.as_bytes())
        {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err("Update helper did not release the application to load user state.".into())
}
#[cfg(windows)]
fn install_msi(artifact: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStringExt;
    let mut buffer = vec![0u16; 32768];
    let length = unsafe {
        windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW(
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        )
    } as usize;
    if length == 0 || length >= buffer.len() {
        return Err("Cannot locate Windows Installer.".into());
    }
    let system = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length]));
    let mut command = crate::git::process::background_command(system.join("msiexec.exe"));
    command
        .arg("/i")
        .arg(artifact)
        .args(["/passive", "/norestart"]);
    let output=crate::git::process::run(&mut command,&std::sync::atomic::AtomicBool::new(false),Duration::from_secs(1500)).map_err(|_|"Windows Installer may still be running. No application was restarted; wait for Windows Installer before opening Recorde or retrying.")?;
    if output.status.success() || matches!(output.status.code(), Some(3010 | 1641)) {
        Ok(())
    } else {
        Err("Windows Installer rejected or cancelled installation. The verified MSI remains available.".into())
    }
}
#[cfg(not(windows))]
fn install_msi(_: &Path) -> Result<(), String> {
    Err("MSI installation requires Windows.".into())
}
#[cfg(target_os = "linux")]
pub fn deb_helper_available() -> bool {
    validate_deb_helper(Path::new("/usr/lib/recorde/recorde-update-installer")).is_ok()
}
#[cfg(not(target_os = "linux"))]
pub fn deb_helper_available() -> bool {
    false
}
#[cfg(target_os = "linux")]
fn validate_deb_helper(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    for component in path.ancestors() {
        security::reject_link(component)?;
        let metadata=std::fs::symlink_metadata(component).map_err(|_|"The secure DEB installer helper is unavailable; install the verified download manually.")?;
        if metadata.uid() != 0
            || metadata.mode() & 0o022 != 0
            || (component != path && !metadata.is_dir())
            || (component == path && (!metadata.is_file() || metadata.mode() & 0o111 == 0))
        {
            return Err(
                "The DEB installer helper or its directory is not protected by root ownership."
                    .into(),
            );
        }
    }
    Ok(())
}
#[cfg(target_os = "linux")]
fn install_deb(envelope: &Path, artifact: &Path) -> Result<(), String> {
    let helper = Path::new("/usr/lib/recorde/recorde-update-installer");
    validate_deb_helper(helper)?;
    let mut command = crate::git::process::background_command("/usr/bin/pkexec");
    command
        .arg(helper)
        .arg("--envelope")
        .arg(envelope)
        .arg("--artifact")
        .arg(artifact);
    let output=crate::git::process::run(&mut command,&std::sync::atomic::AtomicBool::new(false),Duration::from_secs(1500)).map_err(|_|"The privileged installer may still be running. No application was restarted; wait for it to finish before reopening Recorde.")?;
    if output.status.success() {
        Ok(())
    } else {
        Err("DEB installation was refused, cancelled or failed. The verified download remains available; inspect your package manager before reopening Recorde.".into())
    }
}
#[cfg(not(target_os = "linux"))]
fn install_deb(_: &Path, _: &Path) -> Result<(), String> {
    Err("DEB installation requires Linux.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_signal_flush_never_publishes_and_existing_signal_is_preserved() {
        let directory =
            security::private_tempdir(&std::env::temp_dir(), "recorde-update-test-").unwrap();
        let signal = directory.path().join("continue");
        assert!(
            write_new_with_flush(&signal, b"nonce", |_| Err("injected flush failure".into()))
                .is_err()
        );
        assert!(!signal.exists());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        write_new(&signal, b"first").unwrap();
        assert!(write_new(&signal, b"second").is_err());
        assert_eq!(std::fs::read(signal).unwrap(), b"first");
    }
    #[test]
    fn interrupted_swap_blocks_normal_startup_and_names_retained_backup() {
        let directory =
            security::private_tempdir(&std::env::temp_dir(), "recorde-update-test-").unwrap();
        let target = directory.path().join("recorde");
        let backup = directory
            .path()
            .join(format!(".recorde-backup-{}", "a".repeat(64)));
        std::fs::write(&target, b"new binary").unwrap();
        std::fs::write(&backup, b"old binary").unwrap();
        let marker = directory.path().join(".recorde-update-pending.json");
        let pending = PendingSwap {
            schema: 1,
            target: target.clone(),
            backup: backup.clone(),
            attempt: directory.path().join("attempt"),
            nonce: "a".repeat(64),
        };
        write_new(&marker, &serde_json::to_vec(&pending).unwrap()).unwrap();
        let error = check_pending_swap(&target).unwrap_err();
        assert!(error.contains(&backup.display().to_string()));
        assert!(error.contains("User state was not opened"));
        assert_eq!(std::fs::read(&target).unwrap(), b"new binary");
        assert_eq!(std::fs::read(&backup).unwrap(), b"old binary");
        clear_pending(&marker).unwrap();
        check_pending_swap(&target).unwrap();
        let missing = directory.path().join("missing-backup");
        assert!(verified_backup(&missing, "unused")
            .unwrap_err()
            .contains(&missing.display().to_string()));
    }
    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn completed_payload_cleanup_preserves_live_helpers_failures_and_receipts() {
        let directory =
            security::private_tempdir(&std::env::temp_dir(), "recorde-update-test-").unwrap();
        let artifact = directory.path().join("recorde-fixture");
        let helper = directory.path().join(if cfg!(windows) {
            "recorde-update-helper.exe"
        } else {
            "recorde-update-helper"
        });
        std::fs::write(&artifact, b"payload").unwrap();
        std::fs::write(&helper, b"helper").unwrap();
        std::fs::write(directory.path().join("unrelated"), b"keep").unwrap();
        let (size, sha256) = protocol::digest(&b"payload"[..]).unwrap();
        let seed = [37; 32];
        let keys = protocol::KeySet {
            schema: 1,
            keys: vec![protocol::public_key(&seed, "fixture".into())],
        };
        let manifest = protocol::Manifest {
            schema: 1,
            key_id: "fixture".into(),
            channel: "stable".into(),
            version: "99.0.0".into(),
            issued_at: 100,
            expires_at: 200,
            platform: "linux".into(),
            arch: "x86_64".into(),
            package_kind: "portable".into(),
            artifact_name: "recorde-fixture".into(),
            url: protocol::release_url("99.0.0", "recorde-fixture").unwrap(),
            size,
            sha256,
        };
        let mut witness = Witness::current().unwrap();
        let request = Request {
            schema: 1,
            parent: witness.clone(),
            target: directory.path().join("installed"),
            target_kind: Target {
                platform: "linux".into(),
                arch: "x86_64".into(),
                package_kind: "portable".into(),
                channel: "stable".into(),
            },
            previous_sha256: String::new(),
            helper_sha256: String::new(),
            envelope: protocol::sign(&serde_json::to_vec(&manifest).unwrap(), &seed).unwrap(),
            nonce: "a".repeat(64),
        };
        write_new(
            &directory.path().join("apply.json"),
            &serde_json::to_vec(&request).unwrap(),
        )
        .unwrap();
        let identity = directory.path().join("helper-identity.json");
        write_new(&identity, &serde_json::to_vec(&witness).unwrap()).unwrap();
        let outcome = directory.path().join("outcome.txt");
        write_new(&outcome, b"Update applied successfully.").unwrap();
        cleanup_confirmed_attempt(directory.path(), &keys).unwrap();
        assert!(artifact.exists());
        assert!(helper.exists());
        witness.created = witness.created.wrapping_add(1);
        std::fs::write(&identity, serde_json::to_vec(&witness).unwrap()).unwrap();
        std::fs::write(&outcome, b"failure needs recovery").unwrap();
        cleanup_confirmed_attempt(directory.path(), &keys).unwrap();
        assert!(artifact.exists());
        std::fs::write(&outcome, b"Update applied successfully.").unwrap();
        cleanup_confirmed_attempt(directory.path(), &keys).unwrap();
        assert!(!artifact.exists());
        assert!(!helper.exists());
        assert!(outcome.exists());
        assert!(directory.path().join("apply.json").exists());
        assert_eq!(
            std::fs::read(directory.path().join("unrelated")).unwrap(),
            b"keep"
        );
    }
    #[test]
    fn retries_use_fresh_attempts_and_preserve_verified_download() {
        let root =
            security::private_tempdir(&std::env::temp_dir(), "recorde-update-test-").unwrap();
        let source = root.path().join("download");
        std::fs::write(&source, b"verified bytes").unwrap();
        let (size, sha256) = protocol::digest(security::open_read(&source).unwrap()).unwrap();
        let manifest = protocol::Manifest {
            schema: 1,
            key_id: "fixture".into(),
            channel: "stable".into(),
            version: "99.0.0".into(),
            issued_at: 100,
            expires_at: 200,
            platform: "linux".into(),
            arch: "x86_64".into(),
            package_kind: "portable".into(),
            artifact_name: "recorde-fixture".into(),
            url: protocol::release_url("99.0.0", "recorde-fixture").unwrap(),
            size,
            sha256,
        };
        let first = stage_attempt(root.path(), &source, b"envelope", &manifest).unwrap();
        let first_path = first.path().to_owned();
        write_new(&first.path().join("ready"), b"old attempt").unwrap();
        let second = stage_attempt(root.path(), &source, b"envelope", &manifest).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for name in [&manifest.artifact_name, &"envelope.json".to_owned()] {
                assert_eq!(
                    std::fs::metadata(second.path().join(name))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
        }
        assert_ne!(first.path(), second.path());
        assert!(!second.path().join("ready").exists());
        assert_eq!(
            std::fs::read(second.path().join("envelope.json")).unwrap(),
            b"envelope"
        );
        drop(first);
        assert!(!first_path.exists());
        assert_eq!(std::fs::read(&source).unwrap(), b"verified bytes");
        std::fs::write(&source, b"tampered").unwrap();
        assert!(stage_attempt(root.path(), &source, b"envelope", &manifest).is_err());
        assert_eq!(
            std::fs::read(second.path().join(&manifest.artifact_name)).unwrap(),
            b"verified bytes"
        );
    }
    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn living_parent_and_installer_failures_never_trigger_fallback_relaunch() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut request = Request {
            schema: 1,
            parent: Witness::current().unwrap(),
            target: file.path().to_owned(),
            target_kind: Target {
                platform: "linux".into(),
                arch: "x86_64".into(),
                package_kind: "portable".into(),
                channel: "stable".into(),
            },
            previous_sha256: digest(file.path()).unwrap(),
            helper_sha256: String::new(),
            envelope: vec![],
            nonce: "a".repeat(64),
        };
        assert!(!may_relaunch_original(&request));
        request.parent.created = request.parent.created.wrapping_add(1);
        assert!(may_relaunch_original(&request));
        for kind in ["msi", "deb"] {
            request.target_kind.package_kind = kind.into();
            assert!(!may_relaunch_original(&request));
        }
    }
    #[test]
    fn atomic_replacement_keeps_old_bytes_and_can_roll_back() {
        let directory =
            security::private_tempdir(&std::env::temp_dir(), "recorde-update-test-").unwrap();
        let target = directory.path().join("recorde");
        let candidate = directory.path().join("candidate");
        let backup = directory.path().join("backup");
        std::fs::write(&target, b"old executable").unwrap();
        std::fs::write(&candidate, b"new executable").unwrap();
        replace(&target, &candidate, &backup).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new executable");
        assert_eq!(std::fs::read(&backup).unwrap(), b"old executable");
        rollback(&target, &backup).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"old executable");
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn failed_application_launch_restores_verified_original() {
        use std::os::unix::fs::PermissionsExt;
        let directory =
            security::private_tempdir(&std::env::temp_dir(), "recorde-update-test-").unwrap();
        let target = directory.path().join("recorde");
        let artifact = directory.path().join("artifact");
        std::fs::write(
            directory.path().join("recorde-install.json"),
            br#"{"schema":1,"kind":"portable"}"#,
        )
        .unwrap();
        std::fs::write(&target, b"original invalid executable fixture").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(&artifact, b"new invalid executable fixture").unwrap();
        let (size, sha256) = protocol::digest(std::fs::File::open(&artifact).unwrap()).unwrap();
        let request = Request {
            schema: 1,
            parent: Witness::current().unwrap(),
            target: target.clone(),
            target_kind: Target {
                platform: "linux".into(),
                arch: "x86_64".into(),
                package_kind: "portable".into(),
                channel: "stable".into(),
            },
            previous_sha256: digest(&target).unwrap(),
            helper_sha256: String::new(),
            envelope: vec![],
            nonce: "a".repeat(64),
        };
        let manifest = protocol::Manifest {
            schema: 1,
            key_id: "fixture".into(),
            channel: "stable".into(),
            version: "99.0.0".into(),
            issued_at: 100,
            expires_at: 200,
            platform: "linux".into(),
            arch: "x86_64".into(),
            package_kind: "portable".into(),
            artifact_name: "recorde".into(),
            url: protocol::release_url("99.0.0", "recorde").unwrap(),
            size,
            sha256,
        };
        let error =
            replace_and_restart(directory.path(), &request, &manifest, &artifact).unwrap_err();
        assert!(error.contains("restored"), "{error}");
        assert_eq!(digest(&target).unwrap(), request.previous_sha256);
    }
    #[cfg(unix)]
    #[test]
    fn replacement_rejects_writable_installation_and_symlinks() {
        use std::os::unix::fs::PermissionsExt;
        let directory =
            security::private_tempdir(&std::env::temp_dir(), "recorde-update-test-").unwrap();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(security::owned_directory(directory.path()).is_err());
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = directory.path().join("link");
        std::os::unix::fs::symlink("somewhere", &path).unwrap();
        assert!(security::reject_link(&path).is_err());
    }
    #[cfg(unix)]
    fn script(directory: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = directory.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
    #[cfg(unix)]
    #[test]
    fn launch_gate_accepts_nonce_before_state_writes_and_reaps_exited_probe() {
        let directory =
            security::private_tempdir(&std::env::temp_dir(), "recorde-update-test-").unwrap();
        let nonce = "a".repeat(64);
        let path = script(
            directory.path(),
            "healthy-probe",
            &format!("dir=${{2%/*}}\nprintf '%s' '{nonce}' > \"$dir/healthy\"\n/bin/sleep 0.3"),
        );
        assert!(
            launch_with_timeout(&path, directory.path(), &nonce, Duration::from_secs(2)).unwrap()
        );
        assert_eq!(
            std::fs::read(directory.path().join("continue")).unwrap(),
            nonce.as_bytes()
        );
    }
    #[cfg(unix)]
    #[test]
    fn no_ack_timeout_stops_owned_descendants_and_never_releases_state_gate() {
        let directory =
            security::private_tempdir(&std::env::temp_dir(), "recorde-update-test-").unwrap();
        let nonce = "a".repeat(64);
        let path = script(directory.path(), "no-ack-probe", "/bin/sleep 30");
        assert!(
            launch_with_timeout(&path, directory.path(), &nonce, Duration::from_millis(150))
                .unwrap_err()
                .contains("startup")
        );
        assert!(!directory.path().join("continue").exists());
        let exited = script(directory.path(), "exited-probe", "exit 7");
        assert!(
            launch_with_timeout(&exited, directory.path(), &nonce, Duration::from_secs(2))
                .unwrap_err()
                .contains("exited")
        );
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn inherited_appimage_cannot_select_another_application() {
        let directory =
            security::private_tempdir(&std::env::temp_dir(), "recorde-update-test-").unwrap();
        let image = directory.path().join("other.AppImage");
        std::fs::write(&image, b"other app").unwrap();
        assert!(validate_appimage_location(
            &image,
            directory.path(),
            &std::env::current_exe().unwrap()
        )
        .is_err());
        assert_eq!(std::fs::read(image).unwrap(), b"other app");
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn appimage_mount_requires_exact_image_source_and_mountpoint() {
        let mountinfo="123 1 0:42 / /tmp/.mount_recorde ro - fuse.Recorde /home/user/Recorde\\040app.AppImage ro\n";
        assert!(appimage_mount_matches(
            mountinfo,
            Path::new("/tmp/.mount_recorde"),
            Path::new("/home/user/Recorde app.AppImage")
        ));
        assert!(!appimage_mount_matches(
            mountinfo,
            Path::new("/tmp/.mount_recorde"),
            Path::new("/home/user/Other.AppImage")
        ));
        assert!(!appimage_mount_matches(
            mountinfo,
            Path::new("/tmp/.mount_other"),
            Path::new("/home/user/Recorde app.AppImage")
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn successful_swap_and_no_ack_rollback_preserve_adjacent_files() {
        for acknowledge in [true, false] {
            let directory =
                security::private_tempdir(&std::env::temp_dir(), "recorde-update-test-").unwrap();
            let nonce = "b".repeat(64);
            let target = script(directory.path(), "recorde", "exit 0");
            std::fs::write(
                directory.path().join("recorde-install.json"),
                br#"{"schema":1,"kind":"portable"}"#,
            )
            .unwrap();
            std::fs::write(directory.path().join("adjacent.txt"), b"keep this").unwrap();
            std::fs::write(
                directory.path().join(".recorde-backup-unrelated"),
                b"prior backup",
            )
            .unwrap();
            let body = if acknowledge {
                format!("dir=${{2%/*}}\nprintf '%s' '{nonce}' > \"$dir/healthy\"\n/bin/sleep 0.3")
            } else {
                "/bin/sleep 30".into()
            };
            let artifact = script(directory.path(), "artifact", &body);
            let (size, sha256) = protocol::digest(security::open_read(&artifact).unwrap()).unwrap();
            let request = Request {
                schema: 1,
                parent: Witness::current().unwrap(),
                target: target.clone(),
                target_kind: Target {
                    platform: "linux".into(),
                    arch: "x86_64".into(),
                    package_kind: "portable".into(),
                    channel: "stable".into(),
                },
                previous_sha256: digest(&target).unwrap(),
                helper_sha256: String::new(),
                envelope: vec![],
                nonce,
            };
            let manifest = protocol::Manifest {
                schema: 1,
                key_id: "fixture".into(),
                channel: "stable".into(),
                version: "99.0.0".into(),
                issued_at: 100,
                expires_at: 200,
                platform: "linux".into(),
                arch: "x86_64".into(),
                package_kind: "portable".into(),
                artifact_name: "recorde-fixture".into(),
                url: protocol::release_url("99.0.0", "recorde-fixture").unwrap(),
                size,
                sha256,
            };
            let result = replace_with_timeout(
                directory.path(),
                &request,
                &manifest,
                &artifact,
                Duration::from_millis(350),
            );
            assert_eq!(result.is_ok(), acknowledge, "{result:?}");
            assert_eq!(
                digest(&target).unwrap(),
                if acknowledge {
                    manifest.sha256
                } else {
                    request.previous_sha256
                }
            );
            assert_eq!(
                std::fs::read(directory.path().join("adjacent.txt")).unwrap(),
                b"keep this"
            );
            assert_eq!(
                std::fs::read(directory.path().join(".recorde-backup-unrelated")).unwrap(),
                b"prior backup"
            );
        }
    }
}
