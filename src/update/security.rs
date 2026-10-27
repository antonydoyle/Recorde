//! Filesystem checks for update storage and executable replacement.
use std::path::Path;
pub fn reject_link(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err("Update paths must not be symbolic links.".into());
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err("Update paths must not be reparse points.".into());
                }
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("Cannot inspect update path.".into()),
    }
}
pub fn owned_directory(path: &Path) -> Result<(), String> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| "Cannot inspect update directory.")?;
    if !metadata.is_dir() {
        return Err("Update directory is not a directory.".into());
    }
    for ancestor in path.ancestors() {
        reject_link(ancestor)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o022 != 0 {
            return Err(
                "Update directory must be owned by this user and not writable by other users."
                    .into(),
            );
        }
    }
    #[cfg(windows)]
    windows_owner(path)?;
    Ok(())
}
#[cfg(windows)]
fn windows_owner(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree},
        Security::{
            Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT},
            CreateWellKnownSid, EqualSid, GetAce, GetTokenInformation, TokenUser,
            WinBuiltinAdministratorsSid, WinLocalSystemSid, ACCESS_ALLOWED_ACE, ACE_HEADER,
            DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let mut owner = std::ptr::null_mut();
        let mut descriptor = std::ptr::null_mut();
        let mut dacl = std::ptr::null_mut();
        if GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        ) != 0
        {
            return Err("Cannot verify update directory owner.".into());
        }
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            LocalFree(descriptor);
            return Err("Cannot inspect current user token.".into());
        }
        let mut size = 0;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut size);
        let mut data = vec![
            0usize;
            (size as usize + std::mem::size_of::<usize>() - 1)
                / std::mem::size_of::<usize>()
        ];
        let success =
            GetTokenInformation(token, TokenUser, data.as_mut_ptr().cast(), size, &mut size) != 0;
        let same =
            success && EqualSid(owner, (*(data.as_ptr().cast::<TOKEN_USER>())).User.Sid) != 0;
        let mut private_acl = !dacl.is_null();
        let mut admin = [0u32; 17];
        let mut system = [0u32; 17];
        let mut admin_size = 68;
        let mut system_size = 68;
        if CreateWellKnownSid(
            WinBuiltinAdministratorsSid,
            std::ptr::null_mut(),
            admin.as_mut_ptr().cast(),
            &mut admin_size,
        ) == 0
            || CreateWellKnownSid(
                WinLocalSystemSid,
                std::ptr::null_mut(),
                system.as_mut_ptr().cast(),
                &mut system_size,
            ) == 0
        {
            private_acl = false;
        }
        if private_acl {
            for index in 0..(*dacl).AceCount {
                let mut ace = std::ptr::null_mut();
                if GetAce(dacl, index as u32, &mut ace) == 0 {
                    private_acl = false;
                    break;
                }
                let header = &*ace.cast::<ACE_HEADER>();
                if header.AceFlags & 0x08 != 0 {
                    continue;
                } // INHERIT_ONLY_ACE does not apply here.
                match header.AceType {
                    0 => {
                        // ACCESS_ALLOWED_ACE
                        let allowed = &*ace.cast::<ACCESS_ALLOWED_ACE>();
                        let sid = (&allowed.SidStart as *const u32).cast_mut().cast();
                        // File writes, child deletion, DELETE, WRITE_DAC, WRITE_OWNER,
                        // GENERIC_WRITE or GENERIC_ALL granted to other principals.
                        let writable = allowed.Mask
                            & (0x2
                                | 0x4
                                | 0x10
                                | 0x40
                                | 0x100
                                | 0x10000
                                | 0x40000
                                | 0x80000
                                | 0x40000000
                                | 0x10000000)
                            != 0;
                        if writable
                            && EqualSid(sid, owner) == 0
                            && EqualSid(sid, admin.as_mut_ptr().cast()) == 0
                            && EqualSid(sid, system.as_mut_ptr().cast()) == 0
                        {
                            private_acl = false;
                            break;
                        }
                    }
                    1 => {} // A deny ACE can only restrict access.
                    _ => {
                        private_acl = false;
                        break;
                    }
                }
            }
        }
        CloseHandle(token);
        LocalFree(descriptor);
        if !private_acl {
            return Err("Update directory permits other principals to write, or has an unsupported ACL. Choose a private installation location.".into());
        }
        if !same {
            return Err("Update directory must be owned by the current user. Use the full installer for this location.".into());
        }
    }
    Ok(())
}

/// Windows directory handles deny delete sharing and retain every ancestor while
/// path-based replacement runs. Unix rejects ancestors writable by other users;
/// privileged administrators and this same user remain outside the threat model.
pub struct DirectoryPin {
    #[cfg(windows)]
    _ancestors: Vec<std::fs::File>,
}
pub fn pin_directory(path: &Path) -> Result<DirectoryPin, String> {
    if !path.is_absolute() {
        return Err("Update directory must be absolute.".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        let mut ancestors = Vec::new();
        for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
            let handle = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(3)
                .custom_flags(0x02000000 | 0x00200000)
                .open(ancestor)
                .map_err(|_| "Cannot retain update directory identity.")?;
            let metadata = handle
                .metadata()
                .map_err(|_| "Cannot inspect retained update directory.")?;
            if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
                return Err(
                    "Update directory chain contains a reparse point or non-directory.".into(),
                );
            }
            ancestors.push(handle);
        }
        owned_directory(path)?;
        return Ok(DirectoryPin {
            _ancestors: ancestors,
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let uid = unsafe { libc::geteuid() };
        for ancestor in path.ancestors() {
            reject_link(ancestor)?;
            let metadata = std::fs::metadata(ancestor)
                .map_err(|_| "Cannot inspect update directory ancestry.")?;
            if metadata.uid() != uid && metadata.uid() != 0 {
                return Err("Update directory ancestry belongs to another user.".into());
            }
            if metadata.mode() & 0o022 != 0
                && !(metadata.uid() == 0 && metadata.mode() & 0o1000 != 0)
            {
                return Err(format!(
                    "Update directory {} is writable by other users.",
                    ancestor.display()
                ));
            }
        }
        owned_directory(path)?;
        Ok(DirectoryPin {})
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err("Secure update directories are unsupported on this platform.".into())
    }
}

pub fn open_read(path: &Path) -> Result<std::fs::File, String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000).share_mode(1);
    }
    let file = options
        .open(path)
        .map_err(|_| "Cannot safely open update file.")?;
    let metadata = file
        .metadata()
        .map_err(|_| "Cannot inspect opened update file.")?;
    if !metadata.is_file() {
        return Err("Update path is not a regular file.".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("Update file is a reparse point.".into());
        }
    }
    Ok(file)
}

pub fn create_private(path: &Path) -> Result<std::fs::File, String> {
    create_private_file(path)
}

/// Create a new file with private permissions in the creation syscall. This
/// also protects SSH key material when the parent has a broadly inherited ACL.
pub(crate) fn create_private_file(path: &Path) -> Result<std::fs::File, String> {
    #[cfg(windows)]
    {
        use std::os::windows::{ffi::OsStrExt, io::FromRawHandle};
        use std::path::{Component, Prefix};
        use windows_sys::Win32::{
            Foundation::{GENERIC_WRITE, INVALID_HANDLE_VALUE},
            Security::SECURITY_ATTRIBUTES,
            Storage::FileSystem::{
                CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE,
            },
        };
        // Alternate streams share the base file's ACL and device paths are not
        // regular files. A supplied creation descriptor cannot make them private.
        for component in path.components() {
            match component {
                Component::Prefix(prefix)
                    if !matches!(
                        prefix.kind(),
                        Prefix::Disk(_)
                            | Prefix::VerbatimDisk(_)
                            | Prefix::UNC(_, _)
                            | Prefix::VerbatimUNC(_, _)
                    ) =>
                {
                    return Err("Private files require a regular filesystem path.".into());
                }
                Component::Normal(name) => {
                    let name = name.to_string_lossy();
                    let stem = name
                        .split('.')
                        .next()
                        .unwrap_or_default()
                        .trim_end_matches(' ')
                        .to_ascii_uppercase();
                    let numbered_device = ["COM", "LPT"].into_iter().any(|prefix| {
                        stem.strip_prefix(prefix).is_some_and(|suffix| {
                            matches!(
                                suffix,
                                "1" | "2"
                                    | "3"
                                    | "4"
                                    | "5"
                                    | "6"
                                    | "7"
                                    | "8"
                                    | "9"
                                    | "¹"
                                    | "²"
                                    | "³"
                            )
                        })
                    });
                    if name.contains(':')
                        || matches!(
                            stem.as_str(),
                            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
                        )
                        || numbered_device
                    {
                        return Err("Private files cannot use alternate data streams or Windows device names.".into());
                    }
                }
                _ => {}
            }
        }
        let descriptor = windows_descriptor("", true)?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: 0,
        };
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if wide[..wide.len() - 1].contains(&0) {
            return Err("Private file path contains a null character.".into());
        }
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                &attributes,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(format!(
                "Cannot create a new private file: {}",
                std::io::Error::last_os_error()
            ));
        }
        let file = unsafe { std::fs::File::from_raw_handle(handle) };
        if !file
            .metadata()
            .map_err(|_| "Cannot verify the new private file.")?
            .is_file()
        {
            return Err("Private storage is not a regular file.".into());
        }
        return Ok(file);
    }
    #[cfg(not(windows))]
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(path)
            .map_err(|_| "Cannot create a new private file.")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|_| "Cannot set private file permissions.")?;
        }
        Ok(file)
    }
}
pub fn private_tempdir(parent: &Path, prefix: &str) -> Result<tempfile::TempDir, String> {
    let mut builder = tempfile::Builder::new();
    builder.prefix(prefix);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    let directory = builder
        .tempdir_in(parent)
        .map_err(|_| "Cannot create private update staging directory.")?;
    // In particular, refuse an inherited writable Windows ACL before any
    // download, handoff, or executable bytes are placed in this directory.
    owned_directory(directory.path())?;
    Ok(directory)
}

/// Resolve OS temporary-directory aliases (for example macOS /var) only in tests.
/// Production update paths must continue to reject symbolic-link ancestors.
#[cfg(test)]
pub(super) fn fixture_dir() -> tempfile::TempDir {
    let parent = std::env::temp_dir()
        .canonicalize()
        .expect("Cannot resolve the test temporary directory.");
    #[cfg(windows)]
    {
        let directory = tempfile::Builder::new()
            .prefix("recorde-update-test-")
            .tempdir_in(parent)
            .expect("Cannot create an update test directory.");
        fixture_acl(directory.path(), false);
        owned_directory(directory.path()).expect("Update test directory must be private.");
        directory
    }
    #[cfg(not(windows))]
    {
        private_tempdir(&parent, "recorde-update-test-")
            .expect("Cannot create a private update test directory.")
    }
}

#[cfg(windows)]
struct WindowsDescriptor(windows_sys::Win32::Security::PSECURITY_DESCRIPTOR);
#[cfg(windows)]
impl Drop for WindowsDescriptor {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::LocalFree(self.0) };
    }
}

#[cfg(windows)]
fn windows_directory_descriptor(extra_aces: &str) -> Result<WindowsDescriptor, String> {
    windows_descriptor(extra_aces, false)
}

#[cfg(windows)]
fn windows_descriptor(extra_aces: &str, owner_only: bool) -> Result<WindowsDescriptor, String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            },
            GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err("Cannot identify the update storage owner.".into());
        }
        let mut size = 0;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut size);
        let mut data = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
        let read = GetTokenInformation(token, TokenUser, data.as_mut_ptr().cast(), size, &mut size);
        CloseHandle(token);
        if read == 0 {
            return Err("Cannot read the update storage owner.".into());
        }
        let sid = (*(data.as_ptr().cast::<TOKEN_USER>())).User.Sid;
        let mut sid_text = std::ptr::null_mut();
        if ConvertSidToStringSidW(sid, &mut sid_text) == 0 {
            return Err("Cannot encode the update storage owner.".into());
        }
        let mut len = 0;
        while *sid_text.add(len) != 0 {
            len += 1;
        }
        let sid_string = String::from_utf16_lossy(std::slice::from_raw_parts(sid_text, len));
        LocalFree(sid_text.cast());
        let text = if owner_only {
            format!("O:{sid_string}D:P(A;;FA;;;{sid_string})")
        } else {
            format!("O:{sid_string}D:P(A;OICI;FA;;;{sid_string})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA){extra_aces}")
        };
        let sddl: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let mut descriptor = std::ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err("Cannot construct private update storage permissions.".into());
        }
        Ok(WindowsDescriptor(descriptor))
    }
}

/// Create with a protected ACL in the creation syscall. Never repair permissions
/// on an existing directory or treat its untrusted state as a fresh checkpoint.
#[cfg(windows)]
pub fn create_private_directory(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::{GetLastError, ERROR_ALREADY_EXISTS},
        Security::SECURITY_ATTRIBUTES,
        Storage::FileSystem::CreateDirectoryW,
    };
    if !path.is_absolute() {
        return Err("Update storage path must be absolute.".into());
    }
    for ancestor in path.ancestors() {
        reject_link(ancestor)?;
    }
    if let Some(parent) = path.parent() {
        if !parent
            .try_exists()
            .map_err(|_| "Cannot inspect update storage parent.")?
        {
            create_private_directory(parent)?;
        }
    }
    let descriptor = windows_directory_descriptor("")?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe { CreateDirectoryW(wide.as_ptr(), &attributes) } == 0
        && unsafe { GetLastError() } != ERROR_ALREADY_EXISTS
    {
        return Err("Cannot create private update storage.".into());
    }
    owned_directory(path)
        .map_err(|error| format!("{error} Existing update storage was left unchanged."))
}

/// Only changes fresh, empty test directories; never production paths.
#[cfg(all(test, windows))]
fn fixture_acl(path: &Path, allow_everyone: bool) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Security::{
        SetFileSecurityW, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION,
    };
    let descriptor = windows_directory_descriptor(if allow_everyone {
        "(A;OICI;FA;;;WD)"
    } else {
        ""
    })
    .unwrap();
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let applied = unsafe {
        SetFileSecurityW(
            wide.as_ptr(),
            OWNER_SECURITY_INFORMATION
                | DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor.0,
        )
    };
    assert_ne!(applied, 0, "Cannot set the fixture directory ACL.");
}

#[cfg(all(test, windows))]
#[test]
fn private_creation_excludes_writable_parent_acl_and_preserves_existing_unsafe_state() {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Security::{
        GetFileSecurityW, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
    };
    fn snapshot(path: &Path) -> Vec<u8> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let flags = DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION;
        let mut size = 0;
        unsafe { GetFileSecurityW(wide.as_ptr(), flags, std::ptr::null_mut(), 0, &mut size) };
        let mut bytes = vec![0u8; size as usize];
        assert_ne!(
            unsafe {
                GetFileSecurityW(
                    wide.as_ptr(),
                    flags,
                    bytes.as_mut_ptr().cast(),
                    size,
                    &mut size,
                )
            },
            0
        );
        bytes
    }
    let directory = fixture_dir();
    fixture_acl(directory.path(), true);
    let parent_acl = snapshot(directory.path());
    let private = directory.path().join("recorde/updates");
    create_private_directory(&private).unwrap();
    owned_directory(&private).unwrap();
    owned_directory(private.parent().unwrap()).unwrap();
    let private_acl = snapshot(&private);
    create_private_directory(&private).unwrap();
    assert_eq!(snapshot(&private), private_acl);
    assert_eq!(snapshot(directory.path()), parent_acl);
    let staging = private_tempdir(&private, "release-").unwrap();
    owned_directory(staging.path()).unwrap();
    let unsafe_path = directory.path().join("existing-unsafe");
    std::fs::create_dir(&unsafe_path).unwrap();
    std::fs::write(unsafe_path.join("state.json"), b"untrusted checkpoint").unwrap();
    let unsafe_acl = snapshot(&unsafe_path);
    assert!(create_private_directory(&unsafe_path).is_err());
    assert_eq!(snapshot(&unsafe_path), unsafe_acl);
    assert_eq!(
        std::fs::read(unsafe_path.join("state.json")).unwrap(),
        b"untrusted checkpoint"
    );
}

#[cfg(all(test, windows))]
#[test]
fn private_staging_refuses_writable_inherited_acl_before_creating_payloads() {
    let directory = fixture_dir();
    let safe = private_tempdir(directory.path(), "safe-").unwrap();
    owned_directory(safe.path()).unwrap();
    drop(safe);
    fixture_acl(directory.path(), true);
    assert!(owned_directory(directory.path()).is_err());
    assert!(private_tempdir(directory.path(), "unsafe-").is_err());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[cfg(all(test, windows))]
#[test]
fn generated_ssh_keys_have_owner_only_acl_under_a_permissive_parent() {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT},
            EqualSid, GetAce, GetSecurityDescriptorControl, ACCESS_ALLOWED_ACE, ACE_HEADER,
            DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, SE_DACL_PROTECTED,
        },
    };
    let directory = fixture_dir();
    fixture_acl(directory.path(), true);
    assert!(windows_owner(directory.path()).is_err());
    let private = directory.path().join("id_acl_fixture");
    crate::auth::ssh::generate_key(&private, "ACL fixture", "").unwrap();
    assert!(
        windows_owner(directory.path()).is_err(),
        "Do not change the user's parent ACL"
    );
    for path in [&private, &private.with_extension("pub")] {
        windows_owner(path).unwrap();
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            let mut owner = std::ptr::null_mut();
            let mut descriptor = std::ptr::null_mut();
            let mut dacl = std::ptr::null_mut();
            assert_eq!(
                GetNamedSecurityInfoW(
                    wide.as_ptr(),
                    SE_FILE_OBJECT,
                    OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                    &mut owner,
                    std::ptr::null_mut(),
                    &mut dacl,
                    std::ptr::null_mut(),
                    &mut descriptor
                ),
                0
            );
            assert!(!dacl.is_null());
            let mut control = 0;
            let mut revision = 0;
            assert_ne!(
                GetSecurityDescriptorControl(descriptor, &mut control, &mut revision),
                0
            );
            assert_ne!(
                control & SE_DACL_PROTECTED,
                0,
                "Private keys must not inherit parent grants"
            );
            assert_eq!(
                (*dacl).AceCount,
                1,
                "Only the owner may access the private file"
            );
            let mut ace = std::ptr::null_mut();
            assert_ne!(GetAce(dacl, 0, &mut ace), 0);
            let header = &*ace.cast::<ACE_HEADER>();
            assert_eq!(header.AceType, 0);
            assert_eq!(header.AceFlags & 0x10, 0, "Grant must not be inherited");
            let grant = &*ace.cast::<ACCESS_ALLOWED_ACE>();
            let sid = (&grant.SidStart as *const u32).cast_mut().cast();
            assert_ne!(EqualSid(sid, owner), 0);
            LocalFree(descriptor);
        }
    }
    let shared = directory.path().join("shared.txt");
    std::fs::write(&shared, "keep shared file unchanged").unwrap();
    assert!(create_private_file(&directory.path().join("shared.txt:secret")).is_err());
    assert!(create_private_file(&directory.path().join("NUL")).is_err());
    assert_eq!(
        std::fs::read_to_string(shared).unwrap(),
        "keep shared file unchanged"
    );
}

pub fn open_lock(path: &Path) -> Result<std::fs::File, String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW).mode(0o600);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000).share_mode(3);
    }
    let file = options
        .open(path)
        .map_err(|_| "Cannot safely open update lock.")?;
    let metadata = file.metadata().map_err(|_| "Cannot inspect update lock.")?;
    if !metadata.is_file() {
        return Err("Update lock is not a regular file.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
            || metadata.mode() & 0o022 != 0
        {
            return Err("Update lock ownership or permissions are unsafe.".into());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("Update lock is a reparse point.".into());
        }
    }
    Ok(file)
}

#[cfg(windows)]
pub fn owned_file(path: &Path) -> Result<(), String> {
    let _file = open_read(path)?;
    windows_owner(path)
}
