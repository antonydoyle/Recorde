use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use crate::auth::StoredAccount;

const MAX_RECENT: usize = 10;

/// Per-repository persistent settings (column widths, etc.).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    pub refs_w:  Option<f32>,
    pub graph_w: Option<f32>,
    pub time_w:  Option<f32>,
    pub hash_w:  Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum PullStrategy {
    #[default]
    Merge,
    Rebase,
    FfOnly,
}

impl PullStrategy {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Merge   => "Merge",
            Self::Rebase  => "Rebase",
            Self::FfOnly  => "Fast-forward only",
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub recent_repos: Vec<PathBuf>,
    #[serde(default = "default_true")]
    pub dark_theme: bool,
    #[serde(default)]
    pub default_editor: String,
    #[serde(default)]
    pub per_repo: std::collections::HashMap<String, RepoConfig>,
    #[serde(default)]
    pub signoff_default: bool,
    #[serde(default = "default_protected")]
    pub protected_branches: Vec<String>,
    /// Provider accounts (no tokens — tokens live in the token store).
    #[serde(default)]
    pub accounts: Vec<StoredAccount>,
    #[serde(default)]
    pub pull_strategy: PullStrategy,
    #[serde(default)]
    pub fetch_prune: bool,
    #[serde(default)]
    pub auto_fetch: bool,
    #[serde(default = "default_300")]
    pub auto_fetch_interval_secs: u32,
    #[serde(default)]
    pub terminal: String,
    #[serde(default)]
    pub diff_tool: String,
    #[serde(default)]
    pub merge_tool: String,
    #[serde(default = "default_font_scale")]
    pub font_scale: f32,
    #[serde(default)]
    pub colorblind_graph: bool,
    /// Persisted right-panel (Changes/Details) width.
    #[serde(default = "default_right_panel_w")]
    pub right_panel_width: f32,
}

fn default_true() -> bool { true }
fn default_protected() -> Vec<String> { vec!["main".into(), "master".into()] }
fn default_300() -> u32 { 300 }
fn default_font_scale() -> f32 { 1.0 }
fn default_right_panel_w() -> f32 { 360.0 }

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

    pub fn get_repo_config(&self, path: &Path) -> RepoConfig {
        self.per_repo.get(&path.display().to_string()).cloned().unwrap_or_default()
    }

    pub fn set_repo_column_widths(&mut self, path: &Path, refs_w: f32, graph_w: f32, time_w: f32, hash_w: f32) {
        let key = path.display().to_string();
        let cfg = self.per_repo.entry(key).or_default();
        cfg.refs_w  = Some(refs_w);
        cfg.graph_w = Some(graph_w);
        cfg.time_w  = Some(time_w);
        cfg.hash_w  = Some(hash_w);
        self.save();
    }

    /// Remove a specific path from the recent list.
    pub fn remove_recent(&mut self, path: &std::path::Path) {
        self.recent_repos.retain(|p| p.as_path() != path);
        self.save();
    }
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("recorde")
        .join("config.toml")
}
