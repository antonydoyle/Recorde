use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const MAX_RECENT: usize = 10;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub recent_repos: Vec<PathBuf>,
    #[serde(default = "default_true")]
    pub dark_theme: bool,
    #[serde(default)]
    pub default_editor: String,
}

fn default_true() -> bool { true }

impl Config {
    pub fn load() -> Self {
        std::fs::read_to_string(config_path())
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(s) = toml::to_string_pretty(self) {
            let _ = std::fs::write(&path, s);
        }
    }

    /// Prepend `path` to the recent list, de-duplicate, cap at MAX_RECENT.
    pub fn add_recent(&mut self, path: PathBuf) {
        self.recent_repos.retain(|p| p != &path);
        self.recent_repos.insert(0, path);
        self.recent_repos.truncate(MAX_RECENT);
        self.save();
    }
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("recorde")
        .join("config.toml")
}
