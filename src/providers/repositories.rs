//! Account-scoped repository discovery and explicit personal forks.
//! Requests never follow redirects or retry writes. Listing is paged by the user.
use crate::auth::{ProviderKind, StoredAccount};
use serde_json::{json, Value};

const PAGE_SIZE: usize = 50;
const MAX_PAGES: u32 = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repository {
    pub path: String,
    pub description: String,
    pub web_url: String,
    pub https_url: String,
    pub ssh_url: String,
    pub private: bool,
    pub archived: bool,
    pub fork: bool,
}

#[derive(Debug)]
pub struct Page {
    pub repositories: Vec<Repository>,
    pub next_page: Option<u32>,
    pub limited: bool,
}

pub fn supported(provider: &ProviderKind) -> bool {
    matches!(
        provider,
        ProviderKind::GitHubCom
            | ProviderKind::GitHubEnterprise
            | ProviderKind::GitLabCom
            | ProviderKind::GitLabSelf
    )
}

fn github(provider: &ProviderKind) -> bool {
    matches!(
        provider,
        ProviderKind::GitHubCom | ProviderKind::GitHubEnterprise
    )
}

fn api_base(account: &StoredAccount) -> Result<String, String> {
    if !supported(&account.provider) {
        return Err("Repository browsing and personal forks are available for GitHub and GitLab. Clone other providers using a repository URL.".into());
    }
    let host = crate::auth::endpoint::normalize_host(&account.host)?;
    if !account.provider.needs_host() && host != account.provider.default_host() {
        return Err(
            "Account host does not match its provider. Update the account in Settings.".into(),
        );
    }
    Ok(if github(&account.provider) {
        if host == "github.com" {
            "https://api.github.com".into()
        } else {
            format!("https://{host}/api/v3")
        }
    } else {
        format!("https://{host}/api/v4")
    })
}

fn token(account: &StoredAccount) -> Result<String, String> {
    crate::auth::store::token_for_account(&account.host, &account.username)?.ok_or_else(|| {
        "No saved credential for this account. Connect it in Settings > Accounts & SSH.".into()
    })
}

/// Empty search lists repositories the account owns or participates in. A search
/// queries all repositories visible to the selected account, including public ones.
pub fn list(account: &StoredAccount, query: &str, page: u32) -> Result<Page, String> {
    let base = api_base(account)?;
    list_at(&base, account, &token(account)?, query, page)
}

fn list_url(base: &str, is_github: bool, query: &str, page: u32) -> Result<String, String> {
    let query = query.trim();
    if page == 0 || page > MAX_PAGES || query.len() > 256 || query.chars().any(char::is_control) {
        return Err("Use a search of at most 256 bytes and a valid result page.".into());
    }
    let component = super::pagination::component;
    Ok(if is_github && query.is_empty() {
        format!("{base}/user/repos?per_page={PAGE_SIZE}&page={page}&sort=updated&direction=desc")
    } else if is_github {
        format!(
            "{base}/search/repositories?q={}&per_page={PAGE_SIZE}&page={page}&sort=updated",
            component(query)
        )
    } else if query.is_empty() {
        format!("{base}/projects?membership=true&per_page={PAGE_SIZE}&page={page}&order_by=last_activity_at&sort=desc")
    } else {
        format!("{base}/projects?search={}&per_page={PAGE_SIZE}&page={page}&order_by=last_activity_at&sort=desc", component(query))
    })
}

fn list_at(
    base: &str,
    account: &StoredAccount,
    token: &str,
    query: &str,
    page: u32,
) -> Result<Page, String> {
    let url = list_url(base, github(&account.provider), query, page)?;
    let value = super::pagination::object(&url, "Authorization", &format!("Bearer {token}"))?;
    let search = github(&account.provider) && !query.trim().is_empty();
    if search && value["incomplete_results"].as_bool() == Some(true) {
        return Err("Provider search was incomplete. Narrow the search and try again.".into());
    }
    let items = if search { &value["items"] } else { &value }
        .as_array()
        .ok_or("Provider did not return a repository list.")?;
    if items.len() > PAGE_SIZE {
        return Err("Provider returned more repositories than the requested page size.".into());
    }
    let repositories = items
        .iter()
        .map(|item| parse(item, account))
        .collect::<Result<Vec<_>, _>>()?;
    let more = items.len() == PAGE_SIZE;
    Ok(Page {
        repositories,
        next_page: (more && page < MAX_PAGES).then_some(page + 1),
        limited: more && page == MAX_PAGES,
    })
}

fn safe_url(value: &Value, key: &str, account: &StoredAccount, ssh: bool) -> String {
    let raw = value[key].as_str().unwrap_or_default();
    let Ok(endpoint) = crate::auth::endpoint::parse(raw, ssh) else {
        return String::new();
    };
    let Ok(host) = crate::auth::endpoint::normalize_host(&account.host) else {
        return String::new();
    };
    let expected = if ssh {
        url::Url::parse(&format!("https://{host}"))
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .unwrap_or_default()
    } else {
        host
    };
    if endpoint.host != expected || (!ssh && endpoint.username.is_some()) {
        return String::new();
    }
    raw.to_string()
}

fn parse(value: &Value, account: &StoredAccount) -> Result<Repository, String> {
    let gh = github(&account.provider);
    let path = value[if gh {
        "full_name"
    } else {
        "path_with_namespace"
    }]
    .as_str()
    .ok_or("Provider repository has no name.")?
    .to_string();
    validate_path(&path, gh)?;
    let https_url = safe_url(
        value,
        if gh { "clone_url" } else { "http_url_to_repo" },
        account,
        false,
    );
    let ssh_url = safe_url(
        value,
        if gh { "ssh_url" } else { "ssh_url_to_repo" },
        account,
        true,
    );
    if https_url.is_empty() && ssh_url.is_empty() {
        return Err("Provider repository has no safe clone URL on the selected host.".into());
    }
    // Repository identity is taken from the API record, never from an arbitrary link.
    for address in [&https_url, &ssh_url].into_iter().filter(|s| !s.is_empty()) {
        let endpoint = crate::auth::endpoint::parse(address, true)?;
        if format!("{}/{}", endpoint.namespace, endpoint.project) != path {
            return Err("Provider repository name and clone URL disagree.".into());
        }
    }
    Ok(Repository {
        path,
        description: value["description"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(1000)
            .collect(),
        web_url: safe_url(
            value,
            if gh { "html_url" } else { "web_url" },
            account,
            false,
        ),
        https_url,
        ssh_url,
        private: if gh {
            value["private"].as_bool().unwrap_or(true)
        } else {
            value["visibility"].as_str() != Some("public")
        },
        archived: value["archived"].as_bool().unwrap_or(false),
        fork: if gh {
            value["fork"].as_bool().unwrap_or(false)
        } else {
            value
                .get("forked_from_project")
                .is_some_and(|v| !v.is_null())
        },
    })
}

fn validate_path(path: &str, gh: bool) -> Result<(), String> {
    let parts: Vec<_> = path.split('/').collect();
    if path.len() > 1024
        || parts.len() < 2
        || (gh && parts.len() != 2)
        || path
            .chars()
            .any(|c| c.is_control() || matches!(c, '\\' | '?' | '#'))
        || parts
            .iter()
            .any(|part| part.trim().is_empty() || matches!(*part, "." | ".."))
    {
        return Err("Invalid provider repository name.".into());
    }
    Ok(())
}

/// Create a personal fork once. Providers may provision Git objects asynchronously.
pub fn fork(account: &StoredAccount, repository: &Repository) -> Result<Repository, String> {
    let base = api_base(account)?;
    fork_at(&base, account, &token(account)?, repository)
}

fn fork_at(
    base: &str,
    account: &StoredAccount,
    token: &str,
    repository: &Repository,
) -> Result<Repository, String> {
    let gh = github(&account.provider);
    validate_path(&repository.path, gh)?;
    let component = super::pagination::component;
    let path = if gh {
        let (owner, name) = repository
            .path
            .split_once('/')
            .ok_or("Invalid repository name.")?;
        format!(
            "{base}/repos/{}/{}/forks",
            component(owner),
            component(name)
        )
    } else {
        format!("{base}/projects/{}/fork", component(&repository.path))
    };
    let value = super::write::send(
        "POST",
        &path,
        "Authorization",
        &format!("Bearer {token}"),
        token,
        Some(json!({})),
    )?;
    parse(&value, account).map_err(|_| "The provider accepted the fork but returned incomplete details. Refresh your repositories before retrying; the fork may already exist.".into())
}

/// Explicit HTTPS account selection contains a username only, never a token.
pub fn clone_url(
    repository: &Repository,
    account: &StoredAccount,
    ssh: bool,
) -> Result<String, String> {
    if ssh {
        if repository.ssh_url.is_empty() {
            return Err("No SSH URL is available for this repository.".into());
        }
        return Ok(repository.ssh_url.clone());
    }
    let mut url = url::Url::parse(&repository.https_url)
        .map_err(|_| "No HTTPS URL is available for this repository.")?;
    url.set_username(&account.username)
        .map_err(|_| "Cannot select the saved clone account.")?;
    Ok(url.into())
}

#[cfg(test)]
#[path = "repositories_tests.rs"]
mod tests;
