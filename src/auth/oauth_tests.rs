use super::*;
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

#[derive(Default)]
struct FakeClock {
    elapsed: Cell<Duration>,
    waits: RefCell<Vec<Duration>>,
}
impl Clock for FakeClock {
    fn elapsed(&self) -> Duration {
        self.elapsed.get()
    }
    fn wait(
        &self,
        duration: Duration,
        deadline: Duration,
        stop: &AtomicBool,
    ) -> Result<(), String> {
        check_cancel(stop)?;
        self.waits.borrow_mut().push(duration);
        self.elapsed
            .set(self.elapsed().saturating_add(duration).min(deadline));
        check_deadline(self, deadline, stop)
    }
}
#[derive(Default)]
struct Mock {
    replies: RefCell<VecDeque<Value>>,
    calls: RefCell<Vec<(String, Vec<(String, String)>, Option<String>)>>,
    after: Option<Box<dyn Fn(usize)>>,
}
impl Mock {
    fn new(replies: Vec<Value>) -> Self {
        Self {
            replies: RefCell::new(replies.into()),
            ..Default::default()
        }
    }
}
impl Transport for Mock {
    fn request(
        &self,
        url: &str,
        form: Option<&[(&str, &str)]>,
        token: Option<&str>,
    ) -> Result<Value, String> {
        self.calls.borrow_mut().push((
            url.into(),
            form.unwrap_or_default()
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            token.map(str::to_string),
        ));
        if let Some(after) = &self.after {
            after(self.calls.borrow().len());
        }
        self.replies
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| "Unexpected mock request".into())
    }
}
fn client() -> Client {
    Client {
        provider: ProviderKind::GitHubCom,
        host: "github.com".into(),
        client_id: "test-public-client".into(),
    }
}
fn device(interval: u64, expiry: u64) -> Value {
    json!({ "device_code":"test-device-secret", "user_code":"ABCD-EFGH", "verification_uri":"https://github.com/login/device", "interval":interval, "expires_in":expiry })
}
fn tokens() -> Value {
    json!({ "access_token":"test-new-access", "refresh_token":"test-new-refresh", "token_type":"bearer", "expires_in":3600, "refresh_token_expires_in":10000 })
}
fn user() -> Value {
    json!({"login":"fixture", "id":42})
}
fn session() -> Session {
    Session {
        client: client(),
        username: "fixture".into(),
        identity: "42".into(),
        access_token: "test-old-access".into(),
        refresh_token: Some("test-old-refresh".into()),
        expires_at: Some(100),
        refresh_expires_at: Some(10000),
    }
}

#[test]
fn device_interval_is_never_clamped_down_and_slow_down_respects_provider_interval() {
    let http = Mock::new(vec![
        device(120, 1200),
        json!({"error":"authorization_pending"}),
        json!({"error":"slow_down", "interval":240}),
        tokens(),
        user(),
    ]);
    let clock = FakeClock::default();
    let result = authorize_with(
        &client(),
        &AtomicBool::new(false),
        &http,
        &clock,
        |code, url, expiry| {
            assert_eq!(code, "ABCD-EFGH");
            assert_eq!(url, "https://github.com/login/device");
            assert_eq!(expiry, 1200);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(
        clock.waits.borrow().as_slice(),
        &[
            Duration::from_secs(120),
            Duration::from_secs(120),
            Duration::from_secs(240)
        ]
    );
    assert_eq!(result.identity, "42");
    assert!(result.refresh_token.is_some());
    let calls = http.calls.borrow();
    assert!(calls[0].1.iter().any(|(key, value)| key == "scope"
        && value.split(' ').any(|scope| scope == "offline_access")
        && value.split(' ').any(|scope| scope == "workflow")));
    assert_eq!(calls.last().unwrap().2.as_deref(), Some("test-new-access"));
    assert!(
        calls
            .iter()
            .all(|(url, _, _)| !url.contains("test-device-secret")
                && !url.contains("test-new-access"))
    );
}
#[test]
fn slow_down_adds_five_seconds_even_without_server_interval() {
    let http = Mock::new(vec![
        device(5, 100),
        json!({"error":"slow_down"}),
        tokens(),
        user(),
    ]);
    let clock = FakeClock::default();
    authorize_with(
        &client(),
        &AtomicBool::new(false),
        &http,
        &clock,
        |_, _, _| Ok(()),
    )
    .unwrap();
    assert_eq!(
        clock.waits.borrow().as_slice(),
        &[Duration::from_secs(5), Duration::from_secs(10)]
    );
}
#[test]
fn expiry_and_cancellation_prevent_further_polling_or_identity_completion() {
    let http = Mock::new(vec![device(120, 60)]);
    let clock = FakeClock::default();
    assert!(authorize_with(
        &client(),
        &AtomicBool::new(false),
        &http,
        &clock,
        |_, _, _| Ok(())
    )
    .err()
    .unwrap()
    .contains("expired"));
    assert_eq!(http.calls.borrow().len(), 1);
    let stop = Arc::new(AtomicBool::new(false));
    let after_stop = stop.clone();
    let mut http = Mock::new(vec![device(5, 100), tokens()]);
    http.after = Some(Box::new(move |count| {
        if count == 2 {
            after_stop.store(true, Ordering::Relaxed);
        }
    }));
    assert!(authorize_with(
        &client(),
        &stop,
        &http,
        &FakeClock::default(),
        |_, _, _| Ok(())
    )
    .err()
    .unwrap()
    .contains("cancelled"));
    assert_eq!(http.calls.borrow().len(), 2);
    let http = Mock::new(vec![]);
    assert!(authorize_with(
        &client(),
        &AtomicBool::new(true),
        &http,
        &FakeClock::default(),
        |_, _, _| Ok(())
    )
    .is_err());
    assert!(http.calls.borrow().is_empty());
}
#[test]
fn device_errors_are_sanitized_and_verification_uri_must_match_exactly() {
    for error in [
        "access_denied",
        "expired_token",
        "device_flow_disabled",
        "unknown",
    ] {
        let http = Mock::new(vec![
            device(5, 100),
            json!({"error":error, "error_description":"test-private-server-response"}),
        ]);
        let error = authorize_with(
            &client(),
            &AtomicBool::new(false),
            &http,
            &FakeClock::default(),
            |_, _, _| Ok(()),
        )
        .err()
        .unwrap();
        assert!(!error.contains("test-private-server-response"));
    }
    for url in [
        "https://github.com.attacker.test/login/device",
        "http://github.com/login/device",
        "https://github.com/login/device?code=secret",
    ] {
        let mut response = device(5, 100);
        response["verification_uri"] = json!(url);
        let http = Mock::new(vec![response]);
        assert!(authorize_with(
            &client(),
            &AtomicBool::new(false),
            &http,
            &FakeClock::default(),
            |_, _, _| panic!("invalid URI must not be shown")
        )
        .is_err());
        assert_eq!(http.calls.borrow().len(), 1);
    }
}
#[test]
fn refresh_rotates_and_verifies_stable_account_identity() {
    let saved = encode(&session()).unwrap();
    let http = Mock::new(vec![tokens(), user()]);
    let (access, updated) = resolve_with(&saved, "github.com", "fixture", &http, 100).unwrap();
    assert_eq!(access, "test-new-access");
    let next: Session =
        serde_json::from_str(updated.as_ref().unwrap().strip_prefix(ENVELOPE).unwrap()).unwrap();
    assert_eq!(next.refresh_token.as_deref(), Some("test-new-refresh"));
    assert_eq!(next.expires_at, Some(3700));
    assert_eq!(next.refresh_expires_at, Some(10100));
    assert!(http.calls.borrow()[0]
        .1
        .iter()
        .any(|(k, v)| k == "grant_type" && v == "refresh_token"));
    let http = Mock::new(vec![tokens(), json!({"login":"fixture", "id":99})]);
    assert!(resolve_with(&saved, "github.com", "fixture", &http, 100)
        .err()
        .unwrap()
        .contains("identity"));
    assert!(resolve_with(&saved, "evil.test", "fixture", &Mock::new(vec![]), 100).is_err());
    assert!(resolve_with(&saved, "github.com", "another", &Mock::new(vec![]), 100).is_err());
}
#[test]
fn expired_refresh_missing_identity_and_invalid_payload_fail_closed() {
    let mut saved = session();
    saved.refresh_expires_at = Some(100);
    assert!(resolve_with(
        &encode(&saved).unwrap(),
        "github.com",
        "fixture",
        &Mock::new(vec![]),
        100
    )
    .is_err());
    for payload in [
        json!({"access_token":"test", "token_type":"mac"}),
        json!({"access_token":"test", "token_type":"bearer", "expires_in":0}),
        json!({"access_token":"test\nheader", "token_type":"bearer"}),
    ] {
        assert!(token_fields(&payload, 100).is_err());
    }
    let http = Mock::new(vec![device(5, 100), tokens(), json!({"login":"fixture"})]);
    assert!(authorize_with(
        &client(),
        &AtomicBool::new(false),
        &http,
        &FakeClock::default(),
        |_, _, _| Ok(())
    )
    .is_err());
    assert!(parse_response(302, b"{}".as_slice()).is_err());
    assert!(parse_response(200, vec![b'x'; LIMIT as usize + 1].as_slice()).is_err());
    assert!(parse_response(200, b"invalid json".as_slice()).is_err());
}
#[test]
fn supported_hosts_and_gitlab_username_are_exact() {
    let mut c = client();
    c.host = "github.com.attacker.test".into();
    assert!(c.validate().is_err());
    for provider in [ProviderKind::BitbucketCloud, ProviderKind::AzureDevOps] {
        let c = Client {
            host: provider.default_host().into(),
            provider,
            client_id: "fixture".into(),
        };
        assert!(c.validate().is_err());
    }
    let mut s = session();
    s.client.provider = ProviderKind::GitLabCom;
    s.client.host = "gitlab.com".into();
    assert_eq!(
        s.client.endpoints().unwrap().device,
        "https://gitlab.com/oauth/authorize_device"
    );
    assert_eq!(git_username(&encode(&s).unwrap()).unwrap(), Some("oauth2"));
    assert_eq!(git_username("test-pat").unwrap(), None);
}

#[test]
fn successful_grant_near_deadline_can_finish_identity_but_still_honors_cancel() {
    let clock = std::rc::Rc::new(FakeClock::default());
    let network_clock = clock.clone();
    let mut http = Mock::new(vec![device(5, 10), tokens(), user()]);
    http.after = Some(Box::new(move |count| {
        if count >= 2 {
            network_clock
                .elapsed
                .set(Duration::from_secs(10 * count as u64));
        }
    }));
    assert!(authorize_with(
        &client(),
        &AtomicBool::new(false),
        &http,
        clock.as_ref(),
        |_, _, _| Ok(())
    )
    .is_ok());
    assert_eq!(http.calls.borrow().len(), 3);

    let stop = Arc::new(AtomicBool::new(false));
    let network_stop = stop.clone();
    let mut http = Mock::new(vec![device(5, 10), tokens(), user()]);
    http.after = Some(Box::new(move |count| {
        if count == 3 {
            network_stop.store(true, Ordering::Relaxed);
        }
    }));
    assert!(authorize_with(
        &client(),
        &stop,
        &http,
        &FakeClock::default(),
        |_, _, _| Ok(())
    )
    .err()
    .unwrap()
    .contains("cancelled"));
}

#[test]
fn renamed_refresh_account_has_a_specific_reconnect_diagnostic() {
    let http = Mock::new(vec![tokens(), json!({"login":"renamed", "id":42})]);
    let error = resolve_with(
        &encode(&session()).unwrap(),
        "github.com",
        "fixture",
        &http,
        100,
    )
    .unwrap_err();
    assert!(error.contains("renamed") && error.contains("reconnect"));
}

#[test]
fn built_in_apps_cover_only_github_com_and_gitlab_com() {
    let github = Some("Ov23liFixture");
    let gitlab = Some("0123456789abcdef");
    let client = built_in_from(&ProviderKind::GitHubCom, "github.com", github, gitlab).unwrap();
    assert_eq!(client.client_id, "Ov23liFixture");
    let client = built_in_from(&ProviderKind::GitLabCom, "gitlab.com", github, gitlab).unwrap();
    assert_eq!(client.client_id, "0123456789abcdef");
    for (provider, host) in [
        (ProviderKind::GitHubEnterprise, "github.example.com"),
        (ProviderKind::GitLabSelf, "gitlab.example.com"),
        (ProviderKind::GitHubCom, "gitlab.com"),
        (ProviderKind::BitbucketCloud, "bitbucket.org"),
    ] {
        assert!(
            built_in_from(&provider, host, github, gitlab).is_none(),
            "{host}"
        );
    }
    // Builds without injected IDs, or with malformed ones, have no built-in app.
    assert!(built_in_from(&ProviderKind::GitHubCom, "github.com", None, gitlab).is_none());
    assert!(built_in_from(
        &ProviderKind::GitHubCom,
        "github.com",
        Some("bad id"),
        gitlab
    )
    .is_none());
}
