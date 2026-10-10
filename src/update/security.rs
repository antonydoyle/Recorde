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
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|_| "Cannot create private update file.".into())
}
pub fn private_tempdir(parent: &Path, prefix: &str) -> Result<tempfile::TempDir, String> {
    let mut builder = tempfile::Builder::new();
    builder.prefix(prefix);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    builder
        .tempdir_in(parent)
        .map_err(|_| "Cannot create private update staging directory.".into())
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
