use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use crate::auth::StoredAccount;

const MAX_RECENT: usize = 10;

/// Per-repository persistent settings (column widths, etc.).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    #[serde(default)]
    pub navigation: Option<RepoNavigation>,
    #[serde(default)]
    pub diff_preferences: DiffPreferences,
    #[serde(default)]
    pub layout: Option<RepoLayout>,
    #[serde(default)]
    pub graph_preferences: crate::ui::commit_graph::GraphPreferences,
    /// Exact HTTPS authority to selected account username; contains no secrets.
    #[serde(default)]
    pub provider_accounts: std::collections::HashMap<String, String>,
    pub refs_w: Option<f32>,
    pub graph_w: Option<f32>,
    pub time_w: Option<f32>,
    pub hash_w: Option<f32>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffPreferences {
    #[serde(default)]
    pub whitespace: bool,
    pub split: bool,
    pub search_open: bool,
    pub search_query: String,
}

/// Navigation only: no cached content, credentials or pending mutations are restored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoNavigation {
    pub selected_commit: Option<String>,
    pub selected_file: Option<String>,
    pub diff_context: crate::state::DiffContext,
    pub search_active: bool,
    pub search_query: String,
    pub show_all_branches: bool,
    pub graph_filter: crate::git::graph::GraphFilter,
    pub history_limit: usize,
    pub focused_pane: u8,
}

impl RepoNavigation {
    pub fn from_view(view: &crate::state::RepoView) -> Self {
        Self {
            selected_commit: view.selected_commit.clone(),
            selected_file: view.selected_file.clone(),
            diff_context: view.diff_context,
            search_active: view.search_active,
            search_query: view.search_query.clone(),
            show_all_branches: view.show_all_branches,
            graph_filter: view.graph_filter.clone(),
            history_limit: view.history_limit,
            focused_pane: view.focused_pane,
        }
    }

    pub fn capture(state: &crate::state::AppState) -> Self {
        Self {
            selected_commit: state.selected_commit.clone(),
            selected_file: state.selected_file.clone(),
            diff_context: state.diff_context,
            search_active: state.search_active,
            search_query: state.search_query.clone(),
            show_all_branches: state.show_all_branches,
            graph_filter: state.graph_filter.clone(),
            history_limit: state.history_limit,
            focused_pane: state.focused_pane,
        }
    }

    pub fn restored_view(&self, path: &Path) -> crate::state::RepoView {
        let repo = git2::Repository::open(path).ok();
        let selected_commit = self.selected_commit.clone().filter(|oid| {
            repo.as_ref().is_some_and(|r| {
                git2::Oid::from_str(oid)
                    .ok()
                    .is_some_and(|oid| r.find_commit(oid).is_ok())
            })
        });
        let mut filter = self.graph_filter.clone();
        if filter.solo_ref.as_ref().is_some_and(|name| {
            !repo
                .as_ref()
                .is_some_and(|r| r.find_reference(name).is_ok())
        }) {
            filter.solo_ref = None;
        }
        if filter.range.as_ref().is_some_and(|(a, b)| {
            !repo.as_ref().is_some_and(|r| {
                r.revparse_single(a)
                    .and_then(|o| o.peel_to_commit())
                    .is_ok()
                    && r.revparse_single(b)
                        .and_then(|o| o.peel_to_commit())
                        .is_ok()
            })
        }) {
            filter.range = None;
        }
        let selected_file = self
            .selected_file
            .clone()
            .filter(|file| crate::git::safe_path::relative(file).is_ok())
            .filter(|_| {
                self.diff_context != crate::state::DiffContext::CommitDiff
                    || selected_commit.is_some()
            });
        crate::state::RepoView {
            selected_commit,
            selected_file,
            diff_context: self.diff_context,
            search_active: self.search_active,
            search_query: self.search_query.clone(),
            show_all_branches: self.show_all_branches,
            graph_filter: filter,
            history_limit: self.history_limit.clamp(2000, 100000),
            focused_pane: self.focused_pane.min(4),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoLayout {
    pub show_sidebar: bool,
    pub show_right_panel: bool,
    pub show_diff_panel: bool,
    pub sidebar_width: Option<f32>,
    pub right_panel_width: Option<f32>,
    pub diff_panel_height: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum PullStrategy {
    #[default]
    GitConfig,
    Merge,
    Rebase,
    FfOnly,
}

impl PullStrategy {
    pub fn label(&self) -> &'static str {
        match self {
            Self::GitConfig => "Use Git configuration",
            Self::Merge => "Merge",
            Self::Rebase => "Rebase",
            Self::FfOnly => "Fast-forward only",
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    /// A failed load must never turn into an overwrite with default settings.
    #[serde(skip)]
    pub load_error: Option<String>,
    /// Public registration metadata only. Tokens and refresh tokens live in the vault.
    #[serde(default)]
    pub oauth_clients: std::collections::BTreeMap<String, crate::auth::oauth::Client>,
    #[serde(default)]
    pub shortcuts: crate::commands::Shortcuts,
    #[serde(default)]
    pub favorites: Vec<PathBuf>,
    #[serde(default)]
    pub workspaces: std::collections::BTreeMap<String, Vec<PathBuf>>,
    #[serde(default)]
    pub open_repos: Vec<PathBuf>,
    #[serde(default)]
    pub active_repo: Option<PathBuf>,
    #[serde(default)]
    pub recent_repos: Vec<PathBuf>,
    #[serde(default = "default_true")]
    pub dark_theme: bool,
    #[serde(default)]
    pub theme_palette: crate::theme::Palette,
    #[serde(default)]
    pub high_contrast: bool,
    /// Interface animation; `System` follows the OS "show animations" setting.
    #[serde(default)]
    pub motion: crate::ui::motion::MotionPreference,
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
    /// Default account selections, replaced when applying a global profile.
    #[serde(default)]
    pub provider_accounts: std::collections::HashMap<String, String>,
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
    #[serde(default = "default_true")]
    pub show_sidebar: bool,
    #[serde(default = "default_true")]
    pub show_right_panel: bool,
    #[serde(default = "default_true")]
    pub show_diff_panel: bool,
    #[serde(default)]
    pub sidebar_width: Option<f32>,
    #[serde(default)]
    pub right_panel_width: Option<f32>,
    #[serde(default)]
    pub diff_panel_height: Option<f32>,
    /// Named commit identities; see Settings > Accounts & SSH.
    #[serde(default)]
    pub profiles: Vec<crate::profiles::Profile>,
    /// Off by default: showing pictures sends a hash of each author's email
    /// address to Gravatar.
    #[serde(default)]
    pub gravatar: bool,
    #[serde(default)]
    pub show_terminal: bool,
}

fn default_true() -> bool {
    true
}
fn default_protected() -> Vec<String> {
    vec!["main".into(), "master".into()]
}
fn default_300() -> u32 {
    300
}
fn default_font_scale() -> f32 {
    1.0
}

impl Config {
    /// Repository selections override the active global profile's defaults.
    pub fn account_binding(&self, repo_path: Option<&Path>, host: &str) -> Option<String> {
        repo_path
            .and_then(|path| self.per_repo.get(&path.display().to_string()))
            .and_then(|repo| repo.provider_accounts.get(host))
            .or_else(|| self.provider_accounts.get(host))
            .cloned()
    }

    pub fn remember_layout(&mut self, path: &Path) {
        let layout = RepoLayout {
            show_sidebar: self.show_sidebar,
            show_right_panel: self.show_right_panel,
            show_diff_panel: self.show_diff_panel,
            sidebar_width: self.sidebar_width,
            right_panel_width: self.right_panel_width,
            diff_panel_height: self.diff_panel_height,
        };
        self.per_repo
            .entry(path.display().to_string())
            .or_default()
            .layout = Some(layout);
    }

    pub fn restore_layout(&mut self, path: &Path) {
        if let Some(layout) = self.get_repo_config(path).layout {
            self.show_sidebar = layout.show_sidebar;
            self.show_right_panel = layout.show_right_panel;
            self.show_diff_panel = layout.show_diff_panel;
            self.sidebar_width = layout.sidebar_width;
            self.right_panel_width = layout.right_panel_width;
            self.diff_panel_height = layout.diff_panel_height;
        }
    }

    pub fn load() -> Self {
        Self::load_from(&config_path())
    }

    fn load_from(path: &Path) -> Self {
        Self::try_load_from(path).unwrap_or_else(|error| {
            let mut defaults: Self = toml::from_str("").unwrap_or_default();
            defaults.load_error = Some(format!(
                "{error} Your existing file is preserved; settings will not be saved. Repair {} and restart Recorde.",
                path.display()
            ));
            defaults
        })
    }

    /// Authentication must not silently ignore a corrupt account binding.
    pub fn try_load() -> Result<Self, String> {
        Self::try_load_from(&config_path())
    }

    fn try_load_from(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(content) => toml::from_str(&content).map_err(|_| {
                "Application configuration is invalid; account selection cannot be resolved.".into()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(toml::from_str("").unwrap_or_default())
            }
            Err(_) => Err("Cannot read application configuration.".into()),
        }
    }

    pub fn save(&self) {
        if let Err(error) = self.try_save() {
            log::error!("{}", error);
        }
    }

    pub fn try_save(&self) -> Result<(), String> {
        self.try_save_to(&config_path())
    }

    fn try_save_to(&self, path: &Path) -> Result<(), String> {
        use std::io::Write;
        if let Some(error) = &self.load_error {
            return Err(error.clone());
        }
        // Also protect a file edited into an invalid state after startup.
        Self::try_load_from(path)?;
        let parent = path.parent().ok_or("Invalid configuration path.")?;
        std::fs::create_dir_all(parent).map_err(|_| "Cannot create configuration directory.")?;
        let content =
            toml::to_string_pretty(self).map_err(|_| "Cannot serialize configuration.")?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .map_err(|_| "Cannot create configuration file.")?;
        temp.write_all(content.as_bytes())
            .map_err(|_| "Cannot write configuration.")?;
        temp.as_file()
            .sync_all()
            .map_err(|_| "Cannot sync configuration.")?;
        temp.persist(path)
            .map_err(|_| "Cannot replace configuration.")?;
        Ok(())
    }

    /// Prepend `path` to the recent list, de-duplicate, cap at MAX_RECENT.
    pub fn add_recent(&mut self, path: PathBuf) {
        self.recent_repos.retain(|p| p != &path);
        self.recent_repos.insert(0, path);
        self.recent_repos.truncate(MAX_RECENT);
        self.save();
    }

    pub fn get_repo_config(&self, path: &Path) -> RepoConfig {
        self.per_repo
            .get(&path.display().to_string())
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_repo_column_widths(
        &mut self,
        path: &Path,
        refs_w: f32,
        graph_w: f32,
        time_w: f32,
        hash_w: f32,
    ) {
        let key = path.display().to_string();
        let cfg = self.per_repo.entry(key).or_default();
        cfg.refs_w = Some(refs_w);
        cfg.graph_w = Some(graph_w);
        cfg.time_w = Some(time_w);
        cfg.hash_w = Some(hash_w);
        self.save();
    }

    pub fn reset_repo_column_widths(&mut self, path: &Path) {
        let key = path.display().to_string();
        if let Some(cfg) = self.per_repo.get_mut(&key) {
            cfg.refs_w = None;
            cfg.graph_w = None;
            cfg.time_w = None;
            cfg.hash_w = None;
        }
        self.save();
    }

    /// Remove a specific path from the recent list.
    pub fn remove_recent(&mut self, path: &std::path::Path) {
        self.recent_repos.retain(|p| p.as_path() != path);
        self.save();
    }
}

fn config_path() -> PathBuf {
    crate::app_dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("recorde")
        .join("config.toml")
}

#[cfg(test)]
mod session_tests {
    use super::*;

    #[test]
    fn pull_defaults_to_git_config_and_preserves_saved_choices() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.pull_strategy, PullStrategy::GitConfig);
        for (saved, expected) in [
            ("GitConfig", PullStrategy::GitConfig),
            ("Merge", PullStrategy::Merge),
            ("Rebase", PullStrategy::Rebase),
            ("FfOnly", PullStrategy::FfOnly),
        ] {
            let config: Config = toml::from_str(&format!("pull_strategy = \"{saved}\"\n")).unwrap();
            assert_eq!(config.pull_strategy, expected);
        }
    }

    #[test]
    fn corrupt_configuration_is_preserved_across_load_and_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original = "favorites = [\"important\"]\n[shortcuts\n";
        std::fs::write(&path, original).unwrap();
        let mut config = Config::load_from(&path);
        assert!(config.load_error.is_some());
        config.dark_theme = false;
        assert!(config.try_save_to(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        std::fs::write(&path, "favorites = [\"important\"]\n").unwrap();
        // Restart/reload is required to avoid overwriting repaired data with defaults.
        assert!(config.try_save_to(&path).is_err());
        let config = Config::load_from(&path);
        config.try_save_to(&path).unwrap();
        assert_eq!(
            Config::try_load_from(&path).unwrap().favorites,
            vec![PathBuf::from("important")]
        );
        std::fs::write(&path, original).unwrap();
        assert!(config.try_save_to(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }
    #[test]
    fn navigation_round_trip_restores_selection_and_drops_stale_refs() {
        let repo = crate::git::test_support::TestRepo::new();
        let oid = repo.commit("file", b"base").to_string();
        let state = crate::state::AppState {
            selected_commit: Some(oid.clone()),
            selected_file: Some("file".into()),
            show_all_branches: true,
            history_limit: 4000,
            graph_filter: crate::git::graph::GraphFilter {
                solo_ref: Some("refs/heads/deleted".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let settings = RepoConfig {
            navigation: Some(RepoNavigation::capture(&state)),
            diff_preferences: DiffPreferences {
                split: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let decoded: RepoConfig = toml::from_str(&toml::to_string(&settings).unwrap()).unwrap();
        let view = decoded.navigation.unwrap().restored_view(repo.path());
        assert_eq!(view.selected_commit.as_deref(), Some(oid.as_str()));
        assert_eq!(view.selected_file.as_deref(), Some("file"));
        assert_eq!(view.history_limit, 4000);
        assert!(view.graph_filter.solo_ref.is_none());
        assert!(view.current_diff.is_none() && view.selected_commit_detail.is_none());
        assert!(decoded.diff_preferences.split);
        assert_eq!(repo.head().to_string(), oid);
    }
}
