pub mod ssh;
pub mod store;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind")]
pub enum ProviderKind {
    GitHubCom,
    GitHubEnterprise,
    GitLabCom,
    GitLabSelf,
}

impl ProviderKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::GitHubCom        => "GitHub.com",
            Self::GitHubEnterprise => "GitHub Enterprise",
            Self::GitLabCom        => "GitLab.com",
            Self::GitLabSelf       => "GitLab (self-hosted)",
        }
    }

    pub fn default_host(&self) -> &'static str {
        match self {
            Self::GitHubCom        => "github.com",
            Self::GitHubEnterprise => "",
            Self::GitLabCom        => "gitlab.com",
            Self::GitLabSelf       => "",
        }
    }

    /// Whether the user must enter a custom host.
    pub fn needs_host(&self) -> bool {
        matches!(self, Self::GitHubEnterprise | Self::GitLabSelf)
    }

    /// Return the effective host: provided value or the default.
    pub fn effective_host<'a>(&'a self, custom: &'a str) -> &'a str {
        if self.needs_host() { custom } else { self.default_host() }
    }
}

/// Non-secret account record persisted in config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAccount {
    pub provider: ProviderKind,
    /// "github.com", "gitlab.com", or an enterprise hostname.
    pub host: String,
    pub username: String,
}

impl StoredAccount {
    pub fn label(&self) -> String {
        format!("{} — {}", self.provider.label(), self.username)
    }
}
