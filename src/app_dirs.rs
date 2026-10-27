//! Application profile locations, including Windows roaming-data overrides.
//!
//! `dirs` uses Known Folder APIs on Windows, which ignore APPDATA. Respect an
//! absolute override for profile files. Cross-process locks and updater state
//! keep their native locations because they coordinate shared resources.
use std::path::PathBuf;

#[cfg(windows)]
fn override_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

pub fn config_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    if let Some(path) = override_path("APPDATA") {
        return Some(path);
    }
    dirs::config_dir()
}

pub fn data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    if let Some(path) = override_path("APPDATA") {
        return Some(path);
    }
    dirs::data_dir()
}

/// Disposable downloads such as author pictures.
pub fn cache_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    if let Some(path) = override_path("LOCALAPPDATA") {
        return Some(path);
    }
    dirs::cache_dir()
}
