//! Public-client browser sign-in. Device codes never enter URLs, logs or config.
//! Tokens (including rotating refresh tokens) are stored only in the OS vault.
use super::{ProviderKind, StoredAccount};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Read;
use std::sync::{atomic::{AtomicBool, Ordering}, mpsc, Arc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const LIMIT: u64 = 256 * 1024;
pub(crate) const ENVELOPE: &str = "recorde-oauth-v1:";

#[derive(Clone, Serialize, Deserialize)]
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
}

struct Endpoints { device: String, token: String, identity: String, verification: String, scope: &'static str }

impl Client {
    fn endpoints(&self) -> Result<Endpoints, String> {
        let host = super::endpoint::normalize_host(&self.host)?;
        if host != self.host || self.client_id.is_empty() || self.client_id.len() > 256
            || self.client_id.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err("Enter the registered application's public client ID and a valid host.".into());
        }
        let root = format!("https://{host}");
        match self.provider {
            ProviderKind::GitHubCom | ProviderKind::GitHubEnterprise => {
                if self.provider == ProviderKind::GitHubCom && host != "github.com" {
                    return Err("GitHub.com sign-in requires github.com.".into());
                }
                Ok(Endpoints {
                    device: format!("{root}/login/device/code"), token: format!("{root}/login/oauth/access_token"),
                    identity: if host == "github.com" { "https://api.github.com/user".into() } else { format!("{root}/api/v3/user") },
                    verification: format!("{root}/login/device"), scope: "repo read:user",
                })
            }
            ProviderKind::GitLabCom | ProviderKind::GitLabSelf => {
                if self.provider == ProviderKind::GitLabCom && host != "gitlab.com" {
                    return Err("GitLab.com sign-in requires gitlab.com.".into());
                }
                Ok(Endpoints { device: format!("{root}/oauth/authorize_device"), token: format!("{root}/oauth/token"),
                    identity: format!("{root}/api/v4/user"), verification: format!("{root}/oauth/device"), scope: "api read_user write_repository" })
            }
        }
    }
}

pub enum Event {
    Verification { code: String, url: String, expires_in: u64 },
    Complete(Result<StoredAccount, String>),
}

pub struct Login {
    pub events: mpsc::Receiver<Event>,
    cancel: Arc<AtomicBool>,
}
impl Login {
    pub fn start(client: Client) -> Self {
        let (tx, events) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        std::thread::spawn(move || {
            let result = authorize(&client, &stop, |code, url, expires_in| {
                tx.send(Event::Verification { code, url, expires_in }).map_err(|_| "Sign-in closed.".to_string())
            }).and_then(|session| {
                check_cancel(&stop)?;
                super::store::save_oauth(&session)?;
                Ok(StoredAccount { provider: client.provider, host: client.host, username: session.username })
            });
            let _ = tx.send(Event::Complete(result));
        });
        Self { events, cancel }
    }
    pub fn cancel(&self) { self.cancel.store(true, Ordering::Relaxed); }
}
impl Drop for Login { fn drop(&mut self) { self.cancel(); } }

fn check_cancel(stop: &AtomicBool) -> Result<(), String> {
    if stop.load(Ordering::Relaxed) { Err("Sign-in cancelled.".into()) } else { Ok(()) }
}

fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() }

fn request(url: &str, form: Option<&[(&str, &str)]>, token: Option<&str>) -> Result<Value, String> {
    let agent = ureq::AgentBuilder::new().redirects(0).timeout(Duration::from_secs(15)).build();
    let mut request = if form.is_some() { agent.post(url) } else { agent.get(url) }
        .set("Accept", "application/json").set("User-Agent", "Recorde-browser-login");
    if let Some(token) = token { request = request.set("Authorization", &format!("Bearer {token}")); }
    let result = match form { Some(form) => request.send_form(form), None => request.call() };
    let response = match result {
        Ok(response) => response,
        Err(ureq::Error::Status(400, response)) if form.is_some() => response,
        Err(ureq::Error::Status(status, _)) => return Err(format!("Sign-in request failed (HTTP {status}). Check the application registration and permissions.")),
        Err(_) => return Err("Cannot contact the sign-in provider. Check the network and try again.".into()),
    };
    if (300..400).contains(&response.status()) { return Err("Sign-in redirects are not accepted.".into()); }
    let mut bytes = Vec::new();
    response.into_reader().take(LIMIT + 1).read_to_end(&mut bytes).map_err(|_| "Cannot read sign-in response.")?;
    if bytes.len() as u64 > LIMIT { return Err("Sign-in response exceeded the size limit.".into()); }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid sign-in response.".into())
}

fn secret(value: &Value, name: &str) -> Result<String, String> {
    let text = value[name].as_str().filter(|s| !s.is_empty() && s.len() <= 16 * 1024 && !s.chars().any(char::is_control))
        .ok_or("The provider returned an invalid credential.")?;
    super::redact::remember(text);
    Ok(text.into())
}

fn identity(endpoint: &str, access: &str) -> Result<(String, String), String> {
    let user = request(endpoint, None, Some(access))?;
    let username = user["login"].as_str().or_else(|| user["username"].as_str())
        .filter(|s| !s.is_empty() && s.len() <= 256 && s.trim() == *s && !s.chars().any(char::is_control))
        .ok_or("The provider did not return an account username.")?.to_string();
    let id = user["id"].as_u64().filter(|id| *id > 0).ok_or("The provider did not return a stable account identity.")?;
    Ok((username, id.to_string()))
}

fn token_fields(value: &Value) -> Result<(String, Option<String>, Option<u64>), String> {
    if !value["token_type"].as_str().is_some_and(|t| t.eq_ignore_ascii_case("bearer")) {
        return Err("The provider returned an unsupported token type.".into());
    }
    let access = secret(value, "access_token")?;
    let refresh = if value.get("refresh_token").is_some() { Some(secret(value, "refresh_token")?) } else { None };
    let expires = match value.get("expires_in") {
        Some(expires) => Some(now().checked_add(expires.as_u64().filter(|n| *n > 0 && *n <= 366 * 86400)
            .ok_or("Invalid access-token expiry.")?).ok_or("Invalid access-token expiry.")?),
        None => None,
    };
    Ok((access, refresh, expires))
}

fn authorize(client: &Client, stop: &AtomicBool, notify: impl FnOnce(String, String, u64) -> Result<(), String>) -> Result<Session, String> {
    let endpoints = client.endpoints()?;
    let start = Instant::now();
    check_cancel(stop)?;
    let response = request(&endpoints.device, Some(&[("client_id", &client.client_id), ("scope", endpoints.scope)]), None)?;
    let device = secret(&response, "device_code")?;
    let code = response["user_code"].as_str().filter(|s| !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .ok_or("Invalid browser verification code.")?.to_string();
    let url = response["verification_uri"].as_str().ok_or("Missing browser verification address.")?;
    if url != endpoints.verification { return Err("The provider returned an unexpected browser verification address.".into()); }
    let expiry = response["expires_in"].as_u64().filter(|n| *n > 0 && *n <= 1800).ok_or("Invalid sign-in expiry.")?;
    let mut interval = response["interval"].as_u64().unwrap_or(5).clamp(5, 60);
    notify(code, url.to_string(), expiry)?;
    loop {
        let poll_at = Instant::now() + Duration::from_secs(interval);
        while Instant::now() < poll_at {
            check_cancel(stop)?;
            if start.elapsed() >= Duration::from_secs(expiry) { return Err("Sign-in expired. Start again to get a new code.".into()); }
            std::thread::sleep(Duration::from_millis(100));
        }
        let response = request(&endpoints.token, Some(&[("client_id", &client.client_id), ("device_code", &device),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code")]), None)?;
        check_cancel(stop)?;
        if start.elapsed() >= Duration::from_secs(expiry) { return Err("Sign-in expired. Start again to get a new code.".into()); }
        match response["error"].as_str() {
            Some("authorization_pending") => continue,
            Some("slow_down") => { interval = interval.saturating_add(5); continue; }
            Some("access_denied") => return Err("Browser sign-in was declined.".into()),
            Some("expired_token") => return Err("Sign-in expired. Start again to get a new code.".into()),
            Some(_) => return Err("Provider rejected sign-in. Check that device login is enabled for this client ID.".into()),
            None => {}
        }
        let (access_token, refresh_token, expires_at) = token_fields(&response)?;
        let (username, identity) = identity(&endpoints.identity, &access_token)?;
        check_cancel(stop)?;
        return Ok(Session { client: client.clone(), username, identity, access_token, refresh_token, expires_at });
    }
}

/// Called under the credential store's cross-process lock to serialize rotation.
pub(crate) fn resolve(envelope: &str, host: &str, username: &str) -> Result<(String, Option<String>), String> {
    let Some(json) = envelope.strip_prefix(ENVELOPE) else { return Ok((envelope.to_string(), None)); };
    let mut session: Session = serde_json::from_str(json).map_err(|_| "Saved browser sign-in is invalid; reconnect the account.")?;
    if session.client.host != host || session.username != username { return Err("Saved browser sign-in does not match the selected account.".into()); }
    super::redact::remember(&session.access_token);
    if let Some(refresh) = &session.refresh_token { super::redact::remember(refresh); }
    let endpoints = session.client.endpoints()?;
    if session.expires_at.is_none_or(|expiry| expiry > now().saturating_add(60)) { return Ok((session.access_token, None)); }
    let refresh = session.refresh_token.as_ref().ok_or("Browser sign-in expired; reconnect the account.")?;
    let response = request(&endpoints.token, Some(&[("client_id", &session.client.client_id), ("refresh_token", refresh), ("grant_type", "refresh_token")]), None)?;
    if response.get("error").is_some() { return Err("Browser sign-in could not be renewed; reconnect the account.".into()); }
    let (access, next_refresh, expiry) = token_fields(&response)?;
    let (account, id) = identity(&endpoints.identity, &access)?;
    if account != username || id != session.identity { return Err("Renewed browser sign-in changed identity; reconnect the account.".into()); }
    session.access_token = access.clone();
    session.refresh_token = next_refresh.or(session.refresh_token);
    session.expires_at = expiry;
    let updated = encode(&session)?;
    Ok((access, Some(updated)))
}

pub(crate) fn encode(session: &Session) -> Result<String, String> {
    serde_json::to_string(session).map(|json| format!("{ENVELOPE}{json}")).map_err(|_| "Cannot save browser sign-in.".into())
}

pub(crate) fn git_username(envelope: &str) -> Result<Option<&'static str>, String> {
    let Some(json) = envelope.strip_prefix(ENVELOPE) else { return Ok(None); };
    let session: Session = serde_json::from_str(json).map_err(|_| "Saved browser sign-in is invalid; reconnect the account.")?;
    Ok(matches!(session.client.provider, ProviderKind::GitLabCom | ProviderKind::GitLabSelf).then_some("oauth2"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_client_endpoints_are_exact_and_tokens_are_bound_to_identity() {
        let client = Client { provider: ProviderKind::GitHubCom, host: "github.com".into(), client_id: "test-client".into() };
        assert_eq!(client.endpoints().unwrap().identity, "https://api.github.com/user");
        let mut invalid = client.clone(); invalid.host = "github.com.attacker.test".into();
        assert!(invalid.endpoints().is_err());
        let session = Session { client, username: "fixture".into(), identity: "42".into(), access_token: "test-only-access".into(), refresh_token: None, expires_at: None };
        let saved = encode(&session).unwrap();
        assert_eq!(resolve(&saved, "github.com", "fixture").unwrap().0, "test-only-access");
        assert!(resolve(&saved, "gitlab.com", "fixture").is_err());
        assert!(resolve(&saved, "github.com", "other").is_err());
        assert!(!format!("{:?}", super::super::redact::text("test-only-access")).contains("test-only-access"));
    }
    #[test]
    fn invalid_token_types_and_expiries_fail_closed() {
        assert!(token_fields(&serde_json::json!({"access_token":"fake", "token_type":"mac"})).is_err());
        assert!(token_fields(&serde_json::json!({"access_token":"fake", "token_type":"bearer", "expires_in":0})).is_err());
        assert!(token_fields(&serde_json::json!({"access_token":"fake\nheader", "token_type":"bearer"})).is_err());
        assert!(token_fields(&serde_json::json!({"access_token":"fake", "token_type":"bearer", "expires_in":3600})).is_ok());
    }
}
