//! Process identity includes creation time, preventing PID reuse from authorizing replacement.
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Witness {
    pub pid: u32,
    pub created: u64,
}
impl Witness {
    pub fn current() -> Result<Self, String> {
        let pid = std::process::id();
        Ok(Self {
            pid,
            created: creation(pid)?.ok_or("Cannot identify the running process.")?,
        })
    }
    pub fn alive(&self) -> Result<bool, String> {
        Ok(creation(self.pid)? == Some(self.created))
    }
}
#[cfg(target_os = "linux")]
fn creation(pid: u32) -> Result<Option<u64>, String> {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => {
            let tail = stat.rsplit_once(") ").ok_or("Invalid process identity.")?.1;
            let fields: Vec<_> = tail.split_whitespace().collect();
            if fields.first() == Some(&"Z") {
                return Ok(None);
            }
            Ok(Some(
                fields
                    .get(19)
                    .ok_or("Missing process creation time.")?
                    .parse()
                    .map_err(|_| "Invalid process creation time.")?,
            ))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("Cannot inspect the update parent process.".into()),
    }
}
#[cfg(windows)]
fn creation(pid: u32) -> Result<Option<u64>, String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, FILETIME, WAIT_OBJECT_0},
        System::Threading::{
            GetProcessTimes, OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
            PROCESS_SYNCHRONIZE,
        },
    };
    unsafe {
        let handle = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        );
        if handle.is_null() {
            return if GetLastError() == ERROR_INVALID_PARAMETER {
                Ok(None)
            } else {
                Err("Cannot inspect the update parent process.".into())
            };
        }
        if WaitForSingleObject(handle, 0) == WAIT_OBJECT_0 {
            CloseHandle(handle);
            return Ok(None);
        }
        let mut created: FILETIME = std::mem::zeroed();
        let mut exited: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        let ok = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) != 0;
        CloseHandle(handle);
        if !ok {
            return Err("Cannot read the update parent creation time.".into());
        }
        Ok(Some(
            ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64,
        ))
    }
}
#[cfg(not(any(windows, target_os = "linux")))]
fn creation(_: u32) -> Result<Option<u64>, String> {
    Err("Automatic update application is unavailable on this platform.".into())
}

#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests {
    use super::*;
    #[test]
    fn witness_requires_same_process_creation() {
        let witness = Witness::current().unwrap();
        assert!(witness.alive().unwrap());
        assert!(!Witness {
            pid: witness.pid,
            created: witness.created.wrapping_add(1)
        }
        .alive()
        .unwrap());
    }
}
