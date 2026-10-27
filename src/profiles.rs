//! Named commit identities (for example work and personal) that can be
//! switched globally or applied to one repository.
use serde::{Deserialize, Serialize};

/// Select the global file Git gives precedence to when both locations exist.
/// libgit2's open_global prefers XDG unconditionally, but Git reads ~/.gitconfig
/// after XDG. Keep XDG-only installations there; create ~/.gitconfig if neither
/// file exists. Do not modify process environment or repository-local settings.
pub fn global_config_for_write() -> Result<git2::Config, git2::Error> {
    let mut config = git2::Config::open_default()?;
    match git2::Config::find_global() {
        Ok(_) => config.open_level(git2::ConfigLevel::Global),
        Err(error) if error.code() == git2::ErrorCode::NotFound => config.open_global(),
        Err(error) => Err(error),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    pub git_name: String,
    pub git_email: String,
    /// Optional `user.signingkey`; an empty value leaves the setting unchanged.
    pub signing_key: String,
    /// Saved account usernames by exact authority. Tokens stay in the OS vault.
    pub provider_accounts: std::collections::HashMap<String, String>,
    pub preferences: Option<ProfilePreferences>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfilePreferences {
    pub dark_theme: bool,
    pub theme_palette: crate::theme::Palette,
    pub high_contrast: bool,
    pub default_editor: String,
    pub terminal: String,
    pub diff_tool: String,
    pub merge_tool: String,
}

impl ProfilePreferences {
    pub fn capture(config: &crate::config::Config) -> Self {
        Self {
            dark_theme: config.dark_theme,
            theme_palette: config.theme_palette,
            high_contrast: config.high_contrast,
            default_editor: config.default_editor.clone(),
            terminal: config.terminal.clone(),
            diff_tool: config.diff_tool.clone(),
            merge_tool: config.merge_tool.clone(),
        }
    }

    pub fn apply(&self, config: &mut crate::config::Config) {
        config.dark_theme = self.dark_theme;
        config.theme_palette = self.theme_palette;
        config.high_contrast = self.high_contrast;
        config.default_editor.clone_from(&self.default_editor);
        config.terminal.clone_from(&self.terminal);
        config.diff_tool.clone_from(&self.diff_tool);
        config.merge_tool.clone_from(&self.merge_tool);
    }
}

impl Profile {
    pub fn validate_accounts(&self, accounts: &[crate::auth::StoredAccount]) -> Result<(), String> {
        for (host, username) in &self.provider_accounts {
            if !accounts
                .iter()
                .any(|account| account.host == *host && account.username == *username)
            {
                return Err(format!("Reconnect the saved account {username} at {host} before applying this profile."));
            }
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("Enter a profile name.".into());
        }
        if self.git_name.trim().is_empty() || self.git_email.trim().is_empty() {
            return Err("A profile needs a commit name and email.".into());
        }
        crate::git::commit_settings::validate_identity(&self.git_name, &self.git_email)?;
        if self.signing_key.chars().any(char::is_control) {
            return Err("The signing key cannot contain control characters.".into());
        }
        Ok(())
    }

    /// Write the identity into `config` (a global or repository config file).
    pub fn apply_to(&self, config: &mut git2::Config) -> Result<(), String> {
        self.validate()?;
        let set = |config: &mut git2::Config, key: &str, value: &str| {
            config
                .set_str(key, value.trim())
                .map_err(|e| e.message().to_string())
        };
        set(config, "user.name", &self.git_name)?;
        set(config, "user.email", &self.git_email)?;
        if !self.signing_key.trim().is_empty() {
            set(config, "user.signingkey", &self.signing_key)?;
        }
        Ok(())
    }

    /// Apply globally, or to one repository when `repo` is given.
    pub fn apply(&self, repo: Option<&std::path::Path>) -> Result<(), String> {
        let mut config = match repo {
            Some(path) => git2::Repository::open(path)
                .and_then(|r| r.config())
                .and_then(|c| c.open_level(git2::ConfigLevel::Local)),
            None => global_config_for_write(),
        }
        .map_err(|e| e.message().to_string())?;
        self.apply_to(&mut config)
    }

    /// Whether `config` currently uses this identity.
    pub fn matches(&self, name: &str, email: &str) -> bool {
        self.git_name.trim() == name.trim() && self.git_email.trim() == email.trim()
    }
}

/// Add or replace a profile by name, keeping names unique.
pub fn upsert(profiles: &mut Vec<Profile>, profile: Profile) -> Result<(), String> {
    profile.validate()?;
    let name = profile.name.trim().to_string();
    let profile = Profile { name, ..profile };
    match profiles
        .iter_mut()
        .find(|p| p.name.eq_ignore_ascii_case(&profile.name))
    {
        Some(existing) => *existing = profile,
        None => profiles.push(profile),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_profile_writes_effective_identity_when_xdg_and_home_configs_coexist() {
        const CHILD: &str = "RECORDE_GLOBAL_PROFILE_CHILD";
        const TEST: &str = "profiles::tests::global_profile_writes_effective_identity_when_xdg_and_home_configs_coexist";
        if let Some(root) = std::env::var_os(CHILD) {
            let root = std::path::PathBuf::from(root);
            let home_file = root.join("home/.gitconfig");
            let xdg_file = root.join("xdg/git/config");
            // Verify the library behavior that originally caused the bug, in
            // a fresh child so its cached home paths cannot affect other tests.
            let mut old_writer = git2::Config::open_default().unwrap().open_global().unwrap();
            old_writer
                .set_str("user.name", "Lower priority write")
                .unwrap();
            assert_eq!(
                git2::Config::open_default()
                    .unwrap()
                    .get_string("user.name")
                    .unwrap(),
                "Home identity"
            );
            assert_eq!(
                git2::Config::open(&xdg_file)
                    .unwrap()
                    .get_string("user.name")
                    .unwrap(),
                "Lower priority write"
            );

            work().apply(None).unwrap();
            let effective = git2::Config::open_default().unwrap();
            assert_eq!(effective.get_string("user.name").unwrap(), "Ada Work");
            assert_eq!(
                effective.get_string("user.email").unwrap(),
                "ada@work.example"
            );
            assert_eq!(effective.get_string("user.signingkey").unwrap(), "ABC123");
            assert_eq!(
                git2::Config::open(&home_file)
                    .unwrap()
                    .get_string("user.name")
                    .unwrap(),
                "Ada Work"
            );
            assert_eq!(
                git2::Config::open(&xdg_file)
                    .unwrap()
                    .get_string("user.name")
                    .unwrap(),
                "Lower priority write"
            );
            drop(effective);
            drop(old_writer);

            // With only XDG present, retain that existing location.
            std::fs::remove_file(&home_file).unwrap();
            work().apply(None).unwrap();
            assert!(!home_file.exists());
            assert_eq!(
                git2::Config::open(&xdg_file)
                    .unwrap()
                    .get_string("user.name")
                    .unwrap(),
                "Ada Work"
            );
            std::fs::remove_file(&xdg_file).unwrap();
            work().apply(None).unwrap();
            assert!(home_file.exists());
            assert_eq!(
                git2::Config::open_default()
                    .unwrap()
                    .get_string("user.email")
                    .unwrap(),
                "ada@work.example"
            );
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let xdg = root.path().join("xdg");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(xdg.join("git")).unwrap();
        std::fs::write(
            home.join(".gitconfig"),
            "[user]\n name = Home identity\n email = home@example.invalid\n",
        )
        .unwrap();
        std::fs::write(
            xdg.join("git/config"),
            "[user]\n name = XDG identity\n email = xdg@example.invalid\n",
        )
        .unwrap();
        let mut command = crate::git::process::background_command(std::env::current_exe().unwrap());
        command
            .args(["--exact", TEST, "--nocapture"])
            .env(CHILD, root.path())
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("XDG_CONFIG_HOME", &xdg)
            .env("APPDATA", root.path())
            .env("LOCALAPPDATA", root.path())
            .env_remove("HOMEDRIVE")
            .env_remove("HOMEPATH")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_CONFIG_GLOBAL")
            .env_remove("GIT_CONFIG_COUNT")
            .current_dir(root.path());
        let output = crate::git::process::run(
            &mut command,
            &std::sync::atomic::AtomicBool::new(false),
            std::time::Duration::from_secs(30),
        )
        .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn work() -> Profile {
        Profile {
            name: "Work".into(),
            git_name: "Ada Work".into(),
            git_email: "ada@work.example".into(),
            signing_key: "ABC123".into(),
            ..Default::default()
        }
    }

    #[test]
    fn upsert_validates_and_replaces_by_name() {
        let mut profiles = Vec::new();
        assert!(upsert(&mut profiles, Profile::default()).is_err());
        upsert(&mut profiles, work()).unwrap();
        let mut changed = work();
        changed.name = "work ".into();
        changed.git_email = "ada@new.example".into();
        upsert(&mut profiles, changed).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].git_email, "ada@new.example");
        assert_eq!(profiles[0].name, "work");
    }

    #[test]
    fn profile_preferences_and_accounts_round_trip_without_credentials() {
        let mut config = crate::config::Config::default();
        config.dark_theme = true;
        config.theme_palette = crate::theme::Palette::Siege;
        config.default_editor = "editor --wait".into();
        let preferences = ProfilePreferences::capture(&config);
        let mut profile = work();
        profile.preferences = Some(preferences);
        profile
            .provider_accounts
            .insert("github.com".into(), "work-user".into());
        assert!(profile.validate_accounts(&[]).is_err());
        config.profiles.push(profile);
        let serialized = toml::to_string(&config).unwrap();
        let restored: crate::config::Config = toml::from_str(&serialized).unwrap();
        let profile = &restored.profiles[0];
        config.dark_theme = false;
        config.default_editor.clear();
        profile.preferences.as_ref().unwrap().apply(&mut config);
        assert!(config.dark_theme);
        assert_eq!(config.theme_palette, crate::theme::Palette::Siege);
        assert_eq!(config.default_editor, "editor --wait");
        config
            .provider_accounts
            .clone_from(&profile.provider_accounts);
        let path = std::path::Path::new("fixture/repo");
        assert_eq!(
            config.account_binding(None, "github.com").as_deref(),
            Some("work-user")
        );
        config
            .per_repo
            .entry(path.display().to_string())
            .or_default()
            .provider_accounts
            .insert("github.com".into(), "personal".into());
        assert_eq!(
            config.account_binding(Some(path), "github.com").as_deref(),
            Some("personal")
        );
        assert_eq!(config.account_binding(Some(path), "github.com:8443"), None);
        let legacy: Profile =
            toml::from_str("name='Old'\ngit_name='Ada'\ngit_email='ada@example.com'").unwrap();
        assert!(legacy.provider_accounts.is_empty());
        assert!(legacy.preferences.is_none());
    }

    #[test]
    fn applying_to_a_repository_changes_only_its_identity() {
        let repo = crate::git::test_support::TestRepo::new();
        work().apply(Some(repo.path())).unwrap();
        let config = repo.open().config().unwrap().snapshot().unwrap();
        assert_eq!(config.get_str("user.name").unwrap(), "Ada Work");
        assert_eq!(config.get_str("user.email").unwrap(), "ada@work.example");
        assert_eq!(config.get_str("user.signingkey").unwrap(), "ABC123");
        assert!(work().matches(" Ada Work", "ada@work.example "));
        let mut personal = work();
        personal.name = "Personal".into();
        personal.git_email = "ada@home.example".into();
        personal.signing_key.clear();
        personal.apply(Some(repo.path())).unwrap();
        let config = repo.open().config().unwrap().snapshot().unwrap();
        assert_eq!(config.get_str("user.email").unwrap(), "ada@home.example");
        assert_eq!(
            config.get_str("user.signingkey").unwrap(),
            "ABC123",
            "an empty key leaves the setting alone"
        );
    }
}
