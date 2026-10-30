//! Public-client browser sign-in. Device codes never enter URLs, logs or config.
//! Tokens (including rotating refresh tokens) are stored only in the OS vault.
use super::ProviderKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Read;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const LIMIT: u64 = 256 * 1024;
pub(crate) const ENVELOPE: &str = "recorde-oauth-v1:";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Client {
    pub provider: ProviderKind,
    pub host: String,
    pub client_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Session {
    pub client: Client,
    pub username: String,
    pub identity: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<u64>,
    #[serde(default)]
    pub refresh_expires_at: Option<u64>,
}

struct Endpoints {
    device: String,
    token: String,
    identity: String,
    verification: String,
    scope: &'static str,
}

/// Public client IDs of Recorde's own OAuth apps, injected by `build.rs` from
/// the environment (release secrets) or the untracked `recorde-oauth.env`.
/// Builds without them fall back to a user-registered app.
const GITHUB_CLIENT_ID: Option<&str> = option_env!("RECORDE_GITHUB_OAUTH_CLIENT_ID");
const GITLAB_CLIENT_ID: Option<&str> = option_env!("RECORDE_GITLAB_OAUTH_CLIENT_ID");

/// Recorde's registered app for github.com or gitlab.com, when this build has
/// one. Enterprise and self-managed servers always need their own app.
pub fn built_in(provider: &ProviderKind, host: &str) -> Option<Client> {
    built_in_from(provider, host, GITHUB_CLIENT_ID, GITLAB_CLIENT_ID)
}

fn built_in_from(
    provider: &ProviderKind,
    host: &str,
    github: Option<&str>,
    gitlab: Option<&str>,
) -> Option<Client> {
    let client_id = match (provider, host) {
        (ProviderKind::GitHubCom, "github.com") => github,
        (ProviderKind::GitLabCom, "gitlab.com") => gitlab,
        _ => None,
    }?;
    let client = Client {
        provider: provider.clone(),
        host: host.into(),
        client_id: client_id.into(),
    };
    client.validate().ok().map(|()| client)
}

impl Client {
    /// True when this is the build's built-in app rather than one the user registered.
    pub fn is_built_in(&self) -> bool {
        built_in(&self.provider, &self.host).as_ref() == Some(self)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.endpoints().map(|_| ())
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
    fn endpoints(&self) -> Result<Endpoints, String> {
        let host = super::endpoint::normalize_host(&self.host)?;
        if host != self.host
            || self.client_id.is_empty()
            || self.client_id.len() > 256
            || self
                .client_id
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(
                "Enter the registered application's public client ID and a valid host.".into(),
            );
        }
        let root = format!("https://{host}");
        match self.provider {
            ProviderKind::BitbucketCloud | ProviderKind::AzureDevOps => {
                return Err(
                    "Browser sign-in is currently supported for GitHub and GitLab only.".into(),
                )
            }
            ProviderKind::GitHubCom | ProviderKind::GitHubEnterprise => {
                if self.provider == ProviderKind::GitHubCom && host != "github.com" {
                    return Err("GitHub.com sign-in requires github.com.".into());
                }
                Ok(Endpoints {
                    device: format!("{root}/login/device/code"),
                    token: format!("{root}/login/oauth/access_token"),
                    identity: if host == "github.com" {
                        "https://api.github.com/user".into()
                    } else {
                        format!("{root}/api/v3/user")
                    },
                    verification: format!("{root}/login/device"),
                    // `workflow` lets HTTPS pushes change .github/workflows files.
                    scope: "repo read:user workflow offline_access",
                })
            }
            ProviderKind::GitLabCom | ProviderKind::GitLabSelf => {
                if self.provider == ProviderKind::GitLabCom && host != "gitlab.com" {
                    return Err("GitLab.com sign-in requires gitlab.com.".into());
                }
                Ok(Endpoints {
                    device: format!("{root}/oauth/authorize_device"),
                    token: format!("{root}/oauth/token"),
                    identity: format!("{root}/api/v4/user"),
                    verification: format!("{root}/oauth/device"),
                    scope: "api read_user write_repository",
                })
            }
        }
    }
}

pub(crate) enum Event {
    Verification {
        code: String,
        url: String,
        expires_in: u64,
    },
    Complete(Result<Session, String>),
}

pub(crate) struct Login {
    pub events: mpsc::Receiver<Event>,
    cancel: Arc<AtomicBool>,
}
impl Login {
    #[cfg(test)]
    pub(crate) fn test_channel() -> (Self, mpsc::Sender<Event>) {
        let (tx, events) = mpsc::channel();
        (
            Self {
                events,
                cancel: Arc::new(AtomicBool::new(false)),
            },
            tx,
        )
    }
    pub fn start(client: Client) -> Self {
        let (tx, events) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        std::thread::spawn(move || {
            let result = authorize_with(
                &client,
                &stop,
                &Http,
                &WallClock(Instant::now()),
                |code, url, expires_in| {
                    tx.send(Event::Verification {
                        code,
                        url,
                        expires_in,
                    })
                    .map_err(|_| "Sign-in closed.".to_string())
                },
            );
            // The worker never writes credentials. Only the active UI session
            // can choose to save the returned, verified account.
            let _ = tx.send(Event::Complete(result));
        });
        Self { events, cancel }
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl Drop for Login {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn check_cancel(stop: &AtomicBool) -> Result<(), String> {
    if stop.load(Ordering::Relaxed) {
        Err("Sign-in cancelled.".into())
    } else {
        Ok(())
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

trait Transport {
    fn request(
        &self,
        url: &str,
        form: Option<&[(&str, &str)]>,
        token: Option<&str>,
    ) -> Result<Value, String>;
}
struct Http;
impl Transport for Http {
    fn request(
        &self,
        url: &str,
        form: Option<&[(&str, &str)]>,
        token: Option<&str>,
    ) -> Result<Value, String> {
        request(url, form, token)
    }
}
trait Clock {
    fn elapsed(&self) -> Duration;
    fn wait(&self, duration: Duration, deadline: Duration, stop: &AtomicBool)
        -> Result<(), String>;
}
struct WallClock(Instant);
impl Clock for WallClock {
    fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
    fn wait(
        &self,
        duration: Duration,
        deadline: Duration,
        stop: &AtomicBool,
    ) -> Result<(), String> {
        let target = self.elapsed().saturating_add(duration).min(deadline);
        while self.elapsed() < target {
            check_cancel(stop)?;
            std::thread::sleep(
                (target - self.elapsed().min(target)).min(Duration::from_millis(100)),
            );
        }
        check_deadline(self, deadline, stop)
    }
}
fn check_deadline(clock: &impl Clock, deadline: Duration, stop: &AtomicBool) -> Result<(), String> {
    check_cancel(stop)?;
    if clock.elapsed() >= deadline {
        return Err("Sign-in expired. Start again to get a new code.".into());
    }
    Ok(())
}

fn request(url: &str, form: Option<&[(&str, &str)]>, token: Option<&str>) -> Result<Value, String> {
    let parsed = url::Url::parse(url).map_err(|_| "Invalid sign-in endpoint.")?;
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err("Sign-in endpoints require HTTPS without embedded credentials.".into());
    }
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(Duration::from_secs(15))
        .build();
    let mut request = if form.is_some() {
        agent.post(url)
    } else {
        agent.get(url)
    }
    .set("Accept", "application/json")
    .set("User-Agent", "Recorde-browser-login");
    if let Some(token) = token {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    let result = match form {
        Some(form) => request.send_form(form),
        None => request.call(),
    };
    let response = match result {
        Ok(response) => response,
        Err(ureq::Error::Status(400, response)) if form.is_some() => response,
        Err(ureq::Error::Status(status, _)) => return Err(format!("Sign-in request failed (HTTP {status}). Check the application registration and permissions.")),
        Err(_) => return Err("Cannot contact the sign-in provider. Check the network and try again.".into()),
    };
    parse_response(response.status(), response.into_reader())
}
fn parse_response(status: u16, reader: impl Read) -> Result<Value, String> {
    if (300..400).contains(&status) {
        return Err("Sign-in redirects are not accepted.".into());
    }
    if status != 200 && status != 400 {
        return Err(format!("Sign-in request failed (HTTP {status})."));
    }
    let mut bytes = Vec::new();
    reader
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read sign-in response.")?;
    if bytes.len() as u64 > LIMIT {
        return Err("Sign-in response exceeded the size limit.".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid sign-in response.".into())
}

fn secret(value: &Value, name: &str) -> Result<String, String> {
    let text = value[name]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 16 * 1024 && !s.chars().any(char::is_control))
        .ok_or("The provider returned an invalid credential.")?;
    super::redact::remember(text);
    Ok(text.into())
}

fn identity(
    http: &impl Transport,
    endpoint: &str,
    access: &str,
) -> Result<(String, String), String> {
    let user = http.request(endpoint, None, Some(access))?;
    let username = user["login"]
        .as_str()
        .or_else(|| user["username"].as_str())
        .filter(|s| {
            !s.is_empty() && s.len() <= 256 && s.trim() == *s && !s.chars().any(char::is_control)
        })
        .ok_or("The provider did not return an account username.")?
        .to_string();
    let id = user["id"]
        .as_u64()
        .filter(|id| *id > 0)
        .ok_or("The provider did not return a stable account identity.")?;
    Ok((username, id.to_string()))
}

fn expiration(value: &Value, field: &str, timestamp: u64) -> Result<Option<u64>, String> {
    value
        .get(field)
        .map(|expiry| {
            timestamp
                .checked_add(
                    expiry
                        .as_u64()
                        .filter(|n| *n > 0 && *n <= 366 * 86400)
                        .ok_or("Invalid token expiry.")?,
                )
                .ok_or_else(|| "Invalid token expiry.".into())
        })
        .transpose()
}
fn token_fields(
    value: &Value,
    timestamp: u64,
) -> Result<(String, Option<String>, Option<u64>), String> {
    if !value["token_type"]
        .as_str()
        .is_some_and(|t| t.eq_ignore_ascii_case("bearer"))
    {
        return Err("The provider returned an unsupported token type.".into());
    }
    let access = secret(value, "access_token")?;
    let refresh = if value.get("refresh_token").is_some() {
        Some(secret(value, "refresh_token")?)
    } else {
        None
    };
    Ok((access, refresh, expiration(value, "expires_in", timestamp)?))
}

fn authorize_with(
    client: &Client,
    stop: &AtomicBool,
    http: &impl Transport,
    clock: &impl Clock,
    notify: impl FnOnce(String, String, u64) -> Result<(), String>,
) -> Result<Session, String> {
    let endpoints = client.endpoints()?;
    let start = clock.elapsed();
    check_cancel(stop)?;
    let response = http.request(
        &endpoints.device,
        Some(&[("client_id", &client.client_id), ("scope", endpoints.scope)]),
        None,
    )?;
    check_cancel(stop)?;
    let device = secret(&response, "device_code")?;
    let code = response["user_code"]
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 64
                && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        .ok_or("Invalid browser verification code.")?
        .to_string();
    let url = response["verification_uri"]
        .as_str()
        .ok_or("Missing browser verification address.")?;
    if url != endpoints.verification {
        return Err("The provider returned an unexpected browser verification address.".into());
    }
    let expiry = response["expires_in"]
        .as_u64()
        .filter(|n| *n > 0 && *n <= 1800)
        .ok_or("Invalid sign-in expiry.")?;
    let deadline = start.saturating_add(Duration::from_secs(expiry));
    let mut interval = match response.get("interval") {
        Some(value) => value
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or("Invalid sign-in polling interval.")?,
        None => 5,
    };
    check_deadline(clock, deadline, stop)?;
    notify(
        code,
        url.to_string(),
        deadline.saturating_sub(clock.elapsed()).as_secs(),
    )?;
    loop {
        clock.wait(Duration::from_secs(interval), deadline, stop)?;
        check_deadline(clock, deadline, stop)?;
        let response = http.request(
            &endpoints.token,
            Some(&[
                ("client_id", &client.client_id),
                ("device_code", &device),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ]),
            None,
        )?;
        check_cancel(stop)?;
        match response["error"].as_str() {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                interval = interval
                    .saturating_add(5)
                    .max(response["interval"].as_u64().unwrap_or(0));
                continue;
            }
            Some("access_denied") => return Err("Browser sign-in was declined.".into()),
            Some("expired_token" | "token_expired") => {
                return Err("Sign-in expired. Start again to get a new code.".into())
            }
            Some(_) => return Err(
                "Provider rejected sign-in. Check that device login is enabled for this client ID."
                    .into(),
            ),
            None => {}
        }
        let timestamp = now();
        let (access_token, refresh_token, expires_at) = token_fields(&response, timestamp)?;
        let refresh_expires_at = expiration(&response, "refresh_token_expires_in", timestamp)?;
        let (username, identity) = identity(http, &endpoints.identity, &access_token)?;
        check_cancel(stop)?;
        return Ok(Session {
            client: client.clone(),
            username,
            identity,
            access_token,
            refresh_token,
            expires_at,
            refresh_expires_at,
        });
    }
}

/// Called under the credential store's cross-process lock to serialize rotation.
pub(crate) fn resolve(
    envelope: &str,
    host: &str,
    username: &str,
) -> Result<(String, Option<String>), String> {
    resolve_with(envelope, host, username, &Http, now())
}
fn resolve_with(
    envelope: &str,
    host: &str,
    username: &str,
    http: &impl Transport,
    timestamp: u64,
) -> Result<(String, Option<String>), String> {
    let Some(json) = envelope.strip_prefix(ENVELOPE) else {
        return Ok((envelope.to_string(), None));
    };
    let mut session: Session = serde_json::from_str(json)
        .map_err(|_| "Saved browser sign-in is invalid; reconnect the account.")?;
    if session.client.host != host || session.username != username {
        return Err("Saved browser sign-in does not match the selected account.".into());
    }
    super::redact::remember(&session.access_token);
    if let Some(refresh) = &session.refresh_token {
        super::redact::remember(refresh);
    }
    let endpoints = session.client.endpoints()?;
    if session
        .expires_at
        .is_none_or(|expiry| expiry > timestamp.saturating_add(60))
    {
        return Ok((session.access_token, None));
    }
    if session
        .refresh_expires_at
        .is_some_and(|expiry| expiry <= timestamp)
    {
        return Err("Browser sign-in expired; reconnect the account.".into());
    }
    let refresh = session
        .refresh_token
        .as_ref()
        .ok_or("Browser sign-in expired; reconnect the account.")?;
    let response = http.request(
        &endpoints.token,
        Some(&[
            ("client_id", &session.client.client_id),
            ("refresh_token", refresh),
            ("grant_type", "refresh_token"),
        ]),
        None,
    )?;
    if response.get("error").is_some() {
        return Err("Browser sign-in could not be renewed; reconnect the account.".into());
    }
    let (access, next_refresh, expiry) = token_fields(&response, timestamp)?;
    if expiry.is_none() {
        return Err("Renewed browser sign-in omitted its expiry; reconnect the account.".into());
    }
    let (account, id) = identity(http, &endpoints.identity, &access)?;
    if id != session.identity {
        return Err("Renewed browser sign-in changed identity; reconnect the account.".into());
    }
    if account != username {
        return Err(
            "The provider account was renamed; reconnect it to update the saved username.".into(),
        );
    }
    session.access_token = access.clone();
    session.refresh_token = next_refresh.or(session.refresh_token);
    session.expires_at = expiry;
    if let Some(expiry) = expiration(&response, "refresh_token_expires_in", timestamp)? {
        session.refresh_expires_at = Some(expiry);
    }
    let updated = encode(&session)?;
    Ok((access, Some(updated)))
}

pub(crate) fn encode(session: &Session) -> Result<String, String> {
    serde_json::to_string(session)
        .map(|json| format!("{ENVELOPE}{json}"))
        .map_err(|_| "Cannot save browser sign-in.".into())
}

pub(crate) fn git_username(envelope: &str) -> Result<Option<&'static str>, String> {
    let Some(json) = envelope.strip_prefix(ENVELOPE) else {
        return Ok(None);
    };
    let session: Session = serde_json::from_str(json)
        .map_err(|_| "Saved browser sign-in is invalid; reconnect the account.")?;
    Ok(matches!(
        session.client.provider,
        ProviderKind::GitLabCom | ProviderKind::GitLabSelf
    )
    .then_some("oauth2"))
}

#[cfg(test)]
#[path = "oauth_tests.rs"]
mod tests;
