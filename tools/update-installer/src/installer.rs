use recorde_update_signing::{self as protocol, Checkpoint, Manifest, Target};
use rustix::fs::{open, openat, mkdirat, Mode, OFlags};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const EXECUTABLE: &str = "/usr/lib/recorde/recorde-update-installer";
const STATE: &str = "/var/lib/recorde-update";
const PRIVATE: u32 = 0o700;

fn args(values: Vec<OsString>) -> Result<(PathBuf, PathBuf), String> {
    if values.len() != 4 || values[0] != "--envelope" || values[2] != "--artifact" {
        return Err("Expected exactly --envelope ABSOLUTE_PATH --artifact ABSOLUTE_PATH.".into());
    }
    let paths = (PathBuf::from(&values[1]), PathBuf::from(&values[3]));
    if !paths.0.is_absolute() || !paths.1.is_absolute() { return Err("Input paths must be absolute.".into()); }
    Ok(paths)
}

fn origin(uid: u32, effective: u32, value: Option<OsString>) -> Result<u32, String> {
    let text = value.and_then(|v| v.into_string().ok()).ok_or("Run the installed helper through pkexec.")?;
    let origin: u32 = text.parse().map_err(|_| "Invalid pkexec origin.")?;
    if uid != 0 || effective != 0 || origin == 0 || text != origin.to_string() {
        return Err("An authenticated, non-root pkexec origin is required.".into());
    }
    Ok(origin)
}

fn metadata(file: &File, owner: u32, directory: bool, private: bool) -> Result<(), String> {
    let m = file.metadata().map_err(|_| "Cannot inspect update file ownership.")?;
    if m.uid() != owner || m.is_dir() != directory || (!directory && !m.is_file())
        || (!directory && m.nlink() != 1) || m.mode() & 0o6022 != 0
        || (private && m.mode() & 0o077 != 0) {
        return Err("Update file ownership, type, links or permissions are unsafe.".into());
    }
    Ok(())
}

/// Open every component relative to the retained previous directory. Never follow
/// a symlink, including a symlink replaced concurrently in an untrusted home.
fn open_path(path: &Path, owner: u32, trusted_parents: bool) -> Result<File, String> {
    if !path.is_absolute() { return Err("Expected an absolute path.".into()); }
    let mut current = File::from(open("/", OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty()).map_err(|_| "Cannot open filesystem root.")?);
    let parts: Vec<_> = path.components().collect();
    for (index, part) in parts.iter().enumerate().skip(1) {
        let Component::Normal(name) = part else { return Err("Non-canonical path components are refused.".into()); };
        if trusted_parents { metadata(&current, 0, true, false)?; }
        let final_part = index + 1 == parts.len();
        let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK
            | if final_part { OFlags::empty() } else { OFlags::DIRECTORY };
        current = File::from(openat(&current, name, flags, Mode::empty()).map_err(|_| "Cannot safely open update path.")?);
    }
    metadata(&current, owner, false, false)?;
    Ok(current)
}

fn bounded(mut file: File, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    Read::by_ref(&mut file).take(limit as u64 + 1).read_to_end(&mut bytes).map_err(|_| "Cannot read update metadata.")?;
    if bytes.len() > limit { return Err("Update metadata exceeds its size limit.".into()); }
    Ok(bytes)
}

fn native_arch() -> Result<&'static str, String> {
    match std::env::consts::ARCH { "x86_64" => Ok("amd64"), "aarch64" => Ok("arm64"), _ => Err("Unsupported DEB architecture.".into()) }
}

fn verify(envelope: &[u8], now: u64, checkpoint: Option<&Checkpoint>) -> Result<Manifest, String> {
    let keys = serde_json::from_slice(include_bytes!("../../../packaging/update-keys.json")).map_err(|_| "Installed signing keys are invalid.")?;
    let manifest = protocol::verify(envelope, &keys)?;
    let target = Target { platform: "linux".into(), arch: std::env::consts::ARCH.into(), package_kind: "deb".into(), channel: manifest.channel.clone() };
    protocol::validate_offer(&manifest, &target, env!("RECORDE_INSTALLED_FLOOR"), now, checkpoint)?;
    if !manifest.artifact_name.ends_with(".deb") { return Err("The authenticated artifact is not a DEB.".into()); }
    Ok(manifest)
}

fn private_state() -> Result<File, String> {
    let mut directory = File::from(open("/", OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty()).map_err(|_| "Cannot open filesystem root.")?);
    for part in ["var", "lib"] {
        metadata(&directory, 0, true, false)?;
        directory = File::from(openat(&directory, part, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty()).map_err(|_| "System state parent is unavailable.")?);
    }
    metadata(&directory, 0, true, false)?;
    match mkdirat(&directory, "recorde-update", Mode::from_bits_truncate(PRIVATE)) {
        Ok(()) => (), Err(rustix::io::Errno::EXIST) => (), Err(_) => return Err("Cannot create protected update state.".into()),
    }
    let private = File::from(openat(&directory, "recorde-update", OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty()).map_err(|_| "Cannot open protected update state.")?);
    metadata(&private, 0, true, true)?;
    Ok(private)
}

fn lock_state(directory: &File) -> Result<File, String> {
    let file = File::from(openat(directory, "lock", OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK, Mode::from_bits_truncate(0o600)).map_err(|_| "Cannot open protected update lock.")?);
    metadata(&file, 0, false, true)?;
    fs2::FileExt::try_lock_exclusive(&file).map_err(|_| "Another package update is already running.")?;
    Ok(file)
}

fn checkpoint(directory: &File) -> Result<Option<Checkpoint>, String> {
    let file = match openat(directory, "checkpoint.json", OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK, Mode::empty()) {
        Ok(file) => File::from(file), Err(rustix::io::Errno::NOENT) => return Ok(None), Err(_) => return Err("Cannot read protected rollback checkpoint; it was preserved.".into()),
    };
    metadata(&file, 0, false, true)?;
    serde_json::from_slice(&bounded(file, 4096)?).map(Some).map_err(|_| "Protected rollback checkpoint is invalid; it was preserved.".into())
}

fn save_checkpoint(directory: &File, manifest: &Manifest) -> Result<(), String> {
    let bytes = serde_json::to_vec(&Checkpoint { version: manifest.version.clone(), issued_at: manifest.issued_at }).map_err(|_| "Cannot serialize update checkpoint.")?;
    let mut temp = tempfile::NamedTempFile::new_in(STATE).map_err(|_| "Cannot prepare protected checkpoint.")?;
    temp.write_all(&bytes).and_then(|_| temp.as_file().sync_all()).map_err(|_| "Cannot flush protected checkpoint.")?;
    temp.persist(Path::new(STATE).join("checkpoint.json")).map_err(|_| "Cannot replace protected checkpoint.")?;
    directory.sync_all().map_err(|_| "Cannot flush protected update state.")
}

struct BoundedReader(File, Instant);
impl Read for BoundedReader {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if Instant::now() >= self.1 { return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "Package copy expired")); }
        self.0.read(bytes)
    }
}
fn staged_copy(input: File, directory: &Path, manifest: &Manifest) -> Result<PathBuf, String> {
    let path = directory.join("recorde.deb");
    let mut file = std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path).map_err(|_| "Cannot stage protected package.")?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600)).map_err(|_| "Cannot protect staged package.")?;
    protocol::copy_verified(BoundedReader(input, Instant::now() + Duration::from_secs(120)), &mut file, manifest)?;
    file.sync_all().map_err(|_| "Cannot flush protected package.")?;
    file.seek(SeekFrom::Start(0)).map_err(|_| "Cannot recheck protected package.")?;
    protocol::copy_verified(file, std::io::sink(), manifest)?;
    Ok(path)
}

fn upstream(version: &str) -> Result<String, String> {
    // Debian revision is permitted; epochs and upstream substitutions are not.
    if version.len() > 256 || !version.as_bytes().first().is_some_and(u8::is_ascii_digit) || version.contains(':') || !version.bytes().all(|c| c.is_ascii_alphanumeric() || b".+~-".contains(&c)) { return Err("Invalid Debian version.".into()); }
    if version.is_empty() || version.ends_with('-') { return Err("Invalid Debian revision.".into()); }
    Ok(version.rsplit_once('-').map_or(version, |(upstream, _)| upstream).into())
}

fn expected_upstream(version: &str) -> Result<String, String> {
    let version = semver::Version::parse(version).map_err(|_| "Invalid authenticated version.")?;
    Ok(format!("{}.{}.{}{}{}", version.major, version.minor, version.patch,
        if version.pre.is_empty() { String::new() } else { format!("~{}", version.pre) },
        if version.build.is_empty() { String::new() } else { format!("+{}", version.build) }))
}

fn package_record(bytes: &[u8], installed: bool) -> Result<(String, String), String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "Invalid package metadata.")?;
    let parts: Vec<_> = text.trim_end_matches('\n').split('\t').collect();
    if parts.len() != if installed { 4 } else { 3 } || parts[0] != "recorde" || (installed && parts[3] != "installed") {
        return Err("Expected the installed Recorde package and exact package identity.".into());
    }
    upstream(parts[1])?;
    if parts[2] != native_arch()? { return Err("Package architecture does not match this installation.".into()); }
    Ok((parts[1].into(), parts[2].into()))
}

pub fn run(values: Vec<OsString>) -> Result<(), String> {
    let (envelope_path, artifact_path) = args(values)?;
    // pkexec supplies this after authorization. An unprivileged caller cannot set
    // the helper's real/effective root credentials by choosing environment values.
    let uid = origin(unsafe { libc::getuid() }, unsafe { libc::geteuid() }, std::env::var_os("PKEXEC_UID"))?;
    if std::env::current_exe().map_err(|_| "Cannot locate installed helper.")? != Path::new(EXECUTABLE) { return Err("Only the packaged helper at its fixed system path may install updates.".into()); }
    open_path(Path::new(EXECUTABLE), 0, true)?;
    for tool in crate::process::Tool::ALL { open_path(Path::new(tool.path()), 0, true)?; }
    native_arch()?;
    let envelope = bounded(open_path(&envelope_path, uid, false)?, protocol::MAX_ENVELOPE)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| "System clock is invalid.")?.as_secs();
    verify(&envelope, now, None)?; // Before creating any privileged state.
    let input = open_path(&artifact_path, uid, false)?;
    let state = private_state()?;
    let _lock = lock_state(&state)?;
    let manifest = verify(&envelope, now, checkpoint(&state)?.as_ref())?;
    crate::process::enable_subreaper()?;
    let installed = crate::process::run(crate::process::Tool::Query, &[OsStr::new("--show"), OsStr::new("--showformat=${Package}\t${Version}\t${Architecture}\t${db:Status-Status}\n"), OsStr::new("recorde")], Duration::from_secs(20))?;
    if !installed.success { return Err("Recorde is not installed through dpkg; use verified download only.".into()); }
    let (installed_version, _) = package_record(&installed.stdout, true)?;
    let temp = tempfile::Builder::new().prefix("install-").tempdir_in(STATE).map_err(|_| "Cannot create protected package directory.")?;
    std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(PRIVATE)).map_err(|_| "Cannot protect package directory.")?;
    let package = staged_copy(input, temp.path(), &manifest)?;
    let control = crate::process::run(crate::process::Tool::Deb, &[OsStr::new("--show"), OsStr::new("--showformat=${Package}\t${Version}\t${Architecture}\n"), package.as_os_str()], Duration::from_secs(30))?;
    if !control.success { return Err("Authenticated artifact is not a readable Debian package.".into()); }
    let (version, _) = package_record(&control.stdout, false)?;
    if upstream(&version)? != expected_upstream(&manifest.version)? { return Err("Debian upstream version does not match the authenticated manifest.".into()); }
    let newer = crate::process::run(crate::process::Tool::Dpkg, &[OsStr::new("--compare-versions"), OsStr::new(&version), OsStr::new("gt"), OsStr::new(&installed_version)], Duration::from_secs(10))?;
    if !newer.success { return Err("Package downgrade or reinstall was refused.".into()); }
    // Root-only bytes are independently checked again immediately before dpkg.
    protocol::copy_verified(open_path(&package, 0, true)?, std::io::sink(), &manifest)?;
    let install_time = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| "System clock is invalid.")?.as_secs();
    verify(&envelope, install_time, checkpoint(&state)?.as_ref())?;
    save_checkpoint(&state, &manifest)?;
    // The dpkg lock is independent of ours. Refuse downgrades again inside dpkg
    // in case another package manager upgraded Recorde after our comparison.
    let outcome = crate::process::run(crate::process::Tool::Dpkg, &[OsStr::new("--refuse-downgrade"), OsStr::new("--skip-same-version"), OsStr::new("--force-confold"), OsStr::new("--install"), package.as_os_str()], Duration::from_secs(900))?;
    if !outcome.success { return Err("dpkg did not complete installation. Inspect package-manager status before retrying; the rollback checkpoint was retained.".into()); }
    let result = crate::process::run(crate::process::Tool::Query, &[OsStr::new("--show"), OsStr::new("--showformat=${Package}\t${Version}\t${Architecture}\t${db:Status-Status}\n"), OsStr::new("recorde")], Duration::from_secs(20))?;
    if !result.success || package_record(&result.stdout, true)?.0 != version { return Err("Installed package version did not confirm the requested update; refresh package-manager status.".into()); }
    Ok(())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
