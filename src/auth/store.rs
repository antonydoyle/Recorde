//! OS-vault secrets with an atomic, non-secret account index.
//! Legacy plaintext entries are removed only after verified vault storage.
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static ACCESS: Mutex<()> = Mutex::new(());

#[derive(Default, Serialize, Deserialize)]
struct TokenFile {
    #[serde(default)]
    tokens: Vec<TokenEntry>,
}

#[derive(Serialize, Deserialize, Clone)]
struct TokenEntry {
    host: String,
    username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token: Option<String>,
}

trait Vault {
    fn get(&self, host: &str, username: &str) -> Result<Option<String>, String>;
    fn set(&self, host: &str, username: &str, token: &str) -> Result<(), String>;
    fn delete(&self, host: &str, username: &str) -> Result<(), String>;
}
struct NativeVault;

fn vault_error(_: keyring::Error) -> String {
    // Backend errors can contain raw secret bytes (e.g. BadEncoding).
    "Cannot access the OS credential store. Unlock it and retry; no plaintext fallback is used."
        .into()
}
fn entry(host: &str, username: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(&format!("Recorde:https://{host}"), username).map_err(vault_error)
}
impl Vault for NativeVault {
    fn get(&self, host: &str, username: &str) -> Result<Option<String>, String> {
        match entry(host, username)?.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(vault_error(error)),
        }
    }
    fn set(&self, host: &str, username: &str, token: &str) -> Result<(), String> {
        entry(host, username)?
            .set_password(token)
            .map_err(vault_error)
    }
    fn delete(&self, host: &str, username: &str) -> Result<(), String> {
        match entry(host, username)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(vault_error(error)),
        }
    }
}

struct Store<V> {
    path: PathBuf,
    vault: V,
}

impl<V: Vault> Store<V> {
    fn read(&self) -> Result<TokenFile, String> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => toml::from_str(&text)
                .map_err(|_| "Credential index is invalid; it has not been changed.".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(TokenFile::default()),
            Err(_) => Err("Cannot read credential index.".into()),
        }
    }
    fn write(&self, file: &TokenFile) -> Result<(), String> {
        if file.tokens.iter().any(|e| e.token.is_some()) {
            return Err("Refusing to write plaintext credentials.".into());
        }
        let parent = self.path.parent().ok_or("Invalid credential index path.")?;
        std::fs::create_dir_all(parent).map_err(|_| "Cannot create credential directory.")?;
        // NamedTempFile is owner-only on Unix from creation, before any writes.
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .map_err(|_| "Cannot create credential index.")?;
        let content = toml::to_string(file).map_err(|_| "Cannot serialize credential index.")?;
        temp.write_all(content.as_bytes())
            .map_err(|_| "Cannot write credential index.")?;
        temp.as_file()
            .sync_all()
            .map_err(|_| "Cannot sync credential index.")?;
        temp.persist(&self.path)
            .map_err(|_| "Cannot replace credential index.")?;
        Ok(())
    }
    fn verified_set(&self, host: &str, username: &str, token: &str) -> Result<(), String> {
        self.vault.set(host, username, token)?;
        if self.vault.get(host, username)?.as_deref() != Some(token) {
            return Err(
                "Credential store verification failed; the existing index has been retained."
                    .into(),
            );
        }
        Ok(())
    }
    fn migrate(&self) -> Result<TokenFile, String> {
        let mut file = self.read()?;
        if !file.tokens.iter().any(|e| e.token.is_some()) {
            return Ok(file);
        }
        // Validate every identity before changing the vault.
        for e in &mut file.tokens {
            e.host = super::endpoint::normalize_host(&e.host)?;
            validate_username(&e.username)?;
        }
        for e in &mut file.tokens {
            if let Some(token) = e.token.take() {
                if self.vault.get(&e.host, &e.username)?.is_none() {
                    self.verified_set(&e.host, &e.username, &token)?;
                }
                // Keep an existing, successfully read vault value: it may be
                // newer than the legacy file after an interrupted migration.
            }
        }
        // Any failure above leaves the original file byte-for-byte intact.
        file.tokens
            .dedup_by(|a, b| a.host == b.host && a.username == b.username);
        self.write(&file)?;
        Ok(file)
    }
    fn save(&self, host: &str, username: &str, token: &str) -> Result<(), String> {
        let host = super::endpoint::normalize_host(host)?;
        validate_username(username)?;
        if token.is_empty() || token.chars().any(char::is_control) {
            return Err("Token cannot be empty.".into());
        }
        let mut file = self.migrate()?;
        self.verified_set(&host, username, token)?;
        file.tokens
            .retain(|e| !(e.host == host && e.username == username));
        file.tokens.push(TokenEntry {
            host,
            username: username.into(),
            token: None,
        });
        self.write(&file)
    }
    fn remove(&self, host: &str, username: &str) -> Result<(), String> {
        let host = super::endpoint::normalize_host(host)?;
        let mut file = self.migrate()?;
        // Retire the index first. A failed vault deletion remains retryable from
        // the account UI, and the retired token can no longer be selected.
        file.tokens
            .retain(|e| !(e.host == host && e.username == username));
        self.write(&file)?;
        self.vault.delete(&host, username)
    }
    fn lookup(
        &self,
        host: &str,
        username: Option<&str>,
    ) -> Result<Option<(String, String)>, String> {
        self.lookup_with(host, username, super::oauth::resolve)
    }
    fn lookup_with(
        &self,
        host: &str,
        username: Option<&str>,
        resolve: impl FnOnce(&str, &str, &str) -> Result<(String, Option<String>), String>,
    ) -> Result<Option<(String, String)>, String> {
        let file = self.migrate()?;
        let mut candidates: Vec<_> = file
            .tokens
            .iter()
            .filter(|e| e.host == host && username.is_none_or(|u| u == e.username))
            .map(|e| e.username.as_str())
            .collect();
        candidates.sort_unstable();
        candidates.dedup();
        if candidates.len() > 1 {
            return Err("Multiple accounts match this host. Choose an account for this repository in Settings > Accounts & SSH.".into());
        }
        match candidates.first() {
            Some(user) => {
                let Some(saved) = self.vault.get(host, user)? else {
                    return Ok(None);
                };
                let (access, refreshed) = resolve(&saved, host, user)?;
                if let Some(refreshed) = refreshed {
                    self.verified_set(host, user, &refreshed)?;
                }
                Ok(Some(((*user).into(), access)))
            }
            None => Ok(None),
        }
    }
}

fn validate_username(username: &str) -> Result<(), String> {
    if username.is_empty() || username.trim() != username || username.chars().any(char::is_control)
    {
        Err("Enter a non-empty account username without control characters.".into())
    } else {
        Ok(())
    }
}

fn with_store<T>(
    operation: impl FnOnce(&Store<NativeVault>) -> Result<T, String>,
) -> Result<T, String> {
    let _guard = ACCESS
        .lock()
        .map_err(|_| "Credential store is busy; restart and retry.")?;
    let directory = dirs::data_dir()
        .ok_or("Cannot locate the application data directory.")?
        .join("recorde");
    std::fs::create_dir_all(&directory).map_err(|_| "Cannot create credential directory.")?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options
        .open(directory.join("credentials.lock"))
        .map_err(|_| "Cannot lock credential index.")?;
    fs2::FileExt::lock_exclusive(&lock).map_err(|_| "Cannot lock credential index.")?;
    operation(&Store {
        path: directory.join("tokens.toml"),
        vault: NativeVault,
    })
}

pub fn migrate() -> Result<(), String> {
    with_store(|store| store.migrate().map(|_| ()))
}
pub fn save(host: &str, username: &str, token: &str) -> Result<(), String> {
    super::redact::remember(token);
    with_store(|store| store.save(host, username, token))
}
pub(crate) fn save_oauth(session: &super::oauth::Session) -> Result<(), String> {
    let envelope = super::oauth::encode(session)?;
    save(&session.client.host, &session.username, &envelope)
}
pub fn remove(host: &str, username: &str) -> Result<(), String> {
    with_store(|store| store.remove(host, username))
}
pub fn lookup_for_url(
    remote: &str,
    repo_path: Option<&Path>,
) -> Result<Option<(String, String)>, String> {
    lookup(remote, repo_path, false)
}
pub fn lookup_for_api(remote: &str, repo_path: &Path) -> Result<Option<(String, String)>, String> {
    lookup(remote, Some(repo_path), true)
}
fn lookup(
    remote: &str,
    repo_path: Option<&Path>,
    for_api: bool,
) -> Result<Option<(String, String)>, String> {
    let endpoint = super::endpoint::parse(remote, for_api)?;
    let config = crate::config::Config::try_load()?;
    let binding = repo_path.and_then(|path| {
        config
            .get_repo_config(path)
            .provider_accounts
            .get(&endpoint.host)
            .cloned()
    });
    let username = remote_username(&endpoint, binding.as_deref(), &config.accounts)?;
    let result = with_store(|store| {
        let result = store.lookup(&endpoint.host, username)?;
        if for_api {
            return Ok(result);
        }
        match result {
            Some((user, token)) => {
                let saved = store
                    .vault
                    .get(&endpoint.host, &user)?
                    .ok_or("Saved account disappeared; reconnect it.")?;
                let git_user = transport_username(&endpoint.host, &user, &saved, false)?;
                Ok(Some((git_user, token)))
            }
            None => Ok(None),
        }
    })?;
    if result.is_none() && binding.is_some() {
        return Err("No token for this repository's selected account. Update it in Settings > Accounts & SSH.".into());
    }
    if let Some((_, token)) = &result {
        super::redact::remember(token);
    }
    Ok(result)
}

fn remote_username<'a>(
    endpoint: &'a super::endpoint::Endpoint,
    binding: Option<&'a str>,
    accounts: &[super::StoredAccount],
) -> Result<Option<&'a str>, String> {
    // Bitbucket remote usernames are clone-protocol hints, whereas API token
    // identities are Atlassian email addresses. A binding chooses the account;
    // without one the store still rejects multiple matching identities.
    let url_user = endpoint.username.as_deref().filter(|user| {
        endpoint.host != "bitbucket.org"
            && !accounts.iter().any(|account| {
                account.host == endpoint.host
                    && match account.provider {
                        super::ProviderKind::GitLabCom | super::ProviderKind::GitLabSelf => {
                            *user == "oauth2"
                        }
                        super::ProviderKind::GitHubCom | super::ProviderKind::GitHubEnterprise => {
                            *user == "x-access-token"
                        }
                        _ => false,
                    }
            })
    });
    selected_username(url_user, binding)
}

fn transport_username(
    host: &str,
    user: &str,
    saved: &str,
    for_api: bool,
) -> Result<String, String> {
    if for_api {
        return Ok(user.into());
    }
    if host == "bitbucket.org" {
        return Ok("x-bitbucket-api-token-auth".into());
    }
    Ok(super::oauth::git_username(saved)?.unwrap_or(user).into())
}

fn selected_username<'a>(
    url_user: Option<&'a str>,
    binding: Option<&'a str>,
) -> Result<Option<&'a str>, String> {
    if let (Some(url_user), Some(bound_user)) = (url_user, binding) {
        if url_user != bound_user {
            return Err(
                "Remote username conflicts with this repository's selected account.".into(),
            );
        }
    }
    Ok(binding.or(url_user))
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
