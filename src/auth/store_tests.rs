use super::*;

#[test]
#[ignore = "requires a native desktop credential vault"]
fn native_vault_round_trip_uses_an_isolated_test_entry() {
    let service = format!(
        "recorde-vault-fixture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let entry = keyring::Entry::new(&service, "fixture-user").unwrap();
    entry.set_password("recorde-disposable-test-value").unwrap();
    let actual = entry.get_password();
    let cleanup = entry.delete_credential();
    assert!(actual.is_ok_and(|value| value == "recorde-disposable-test-value"));
    cleanup.unwrap();
    assert!(matches!(entry.get_password(), Err(keyring::Error::NoEntry)));
}
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

#[derive(Default)]
struct TestVault {
    values: RefCell<HashMap<(String, String), String>>,
    fail_get: Cell<bool>,
    fail_set: Cell<bool>,
    fail_delete: Cell<bool>,
    lose_write: Cell<bool>,
}
impl Vault for TestVault {
    fn get(&self, host: &str, user: &str) -> Result<Option<String>, String> {
        if self.fail_get.get() {
            return Err("vault locked".into());
        }
        Ok(self
            .values
            .borrow()
            .get(&(host.into(), user.into()))
            .cloned())
    }
    fn set(&self, host: &str, user: &str, token: &str) -> Result<(), String> {
        if self.fail_set.get() {
            return Err("write failed".into());
        }
        if !self.lose_write.get() {
            self.values
                .borrow_mut()
                .insert((host.into(), user.into()), token.into());
        }
        Ok(())
    }
    fn delete(&self, host: &str, user: &str) -> Result<(), String> {
        if self.fail_delete.get() {
            return Err("delete failed".into());
        }
        self.values.borrow_mut().remove(&(host.into(), user.into()));
        Ok(())
    }
}
fn fixture() -> (tempfile::TempDir, Store<TestVault>) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store {
        path: dir.path().join("tokens.toml"),
        vault: TestVault::default(),
    };
    (dir, store)
}
fn legacy(store: &Store<TestVault>) -> Vec<u8> {
    let text =
        "[[tokens]]\nhost = 'GitHub.com'\nusername = 'alice'\ntoken = 'legacy-test-secret'\n";
    std::fs::write(&store.path, text).unwrap();
    text.as_bytes().into()
}

#[test]
fn verified_migration_removes_plaintext_and_is_repeatable() {
    let (_dir, store) = fixture();
    legacy(&store);
    store.migrate().unwrap();
    let bytes = std::fs::read_to_string(&store.path).unwrap();
    assert!(!bytes.contains("legacy-test-secret"));
    assert!(!bytes.contains("token ="));
    assert_eq!(
        store
            .lookup("github.com", Some("alice"))
            .unwrap()
            .unwrap()
            .1,
        "legacy-test-secret"
    );
    store.migrate().unwrap();
    assert_eq!(std::fs::read_to_string(&store.path).unwrap(), bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&store.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn migration_failure_keeps_legacy_bytes_and_retry_recovers() {
    for failure in 0..3 {
        let (_dir, store) = fixture();
        let bytes = legacy(&store);
        store.vault.fail_get.set(failure == 0);
        store.vault.fail_set.set(failure == 1);
        store.vault.lose_write.set(failure == 2);
        assert!(store.migrate().is_err());
        assert_eq!(std::fs::read(&store.path).unwrap(), bytes);
        store.vault.fail_get.set(false);
        store.vault.fail_set.set(false);
        store.vault.lose_write.set(false);
        store.migrate().unwrap();
        assert!(!std::fs::read_to_string(&store.path)
            .unwrap()
            .contains("legacy-test-secret"));
    }
}

#[test]
fn migration_does_not_overwrite_newer_vault_value() {
    let (_dir, store) = fixture();
    legacy(&store);
    store
        .vault
        .set("github.com", "alice", "newer-secret")
        .unwrap();
    store.migrate().unwrap();
    assert_eq!(
        store.lookup("github.com", None).unwrap().unwrap().1,
        "newer-secret"
    );
}

#[test]
fn host_and_account_selection_never_fall_back_to_first_account() {
    let (_dir, store) = fixture();
    store.save("GITHUB.com", "alice", "alice-secret").unwrap();
    assert!(store.lookup("sub.github.com", None).unwrap().is_none());
    assert!(store
        .lookup("github.com.evil.test", None)
        .unwrap()
        .is_none());
    assert!(store.lookup("github.com:8443", None).unwrap().is_none());
    store.save("github.com", "bob", "bob-secret").unwrap();
    assert!(store.lookup("github.com", None).is_err());
    assert_eq!(
        store.lookup("github.com", Some("bob")).unwrap().unwrap().1,
        "bob-secret"
    );
    assert!(store
        .lookup("github.com", Some("missing"))
        .unwrap()
        .is_none());
    assert!(!std::fs::read_to_string(&store.path)
        .unwrap()
        .contains("secret"));
}

#[test]
fn failed_save_and_delete_are_retryable_without_plaintext_writes() {
    let (_dir, store) = fixture();
    store.vault.fail_set.set(true);
    assert!(store.save("github.com", "alice", "test-secret").is_err());
    assert!(!store.path.exists());
    store.vault.fail_set.set(false);
    store.save("github.com", "alice", "test-secret").unwrap();
    store.vault.fail_delete.set(true);
    assert!(store.remove("github.com", "alice").is_err());
    assert!(store.lookup("github.com", None).unwrap().is_none());
    store.vault.fail_delete.set(false);
    store.remove("github.com", "alice").unwrap();
    assert!(store.vault.get("github.com", "alice").unwrap().is_none());
}

#[test]
fn malformed_legacy_file_never_leaks_parser_input_or_gets_overwritten() {
    let (_dir, store) = fixture();
    let bytes = b"token = secret-do-not-log!!!";
    std::fs::write(&store.path, bytes).unwrap();
    let error = match store.migrate() {
        Err(e) => e,
        Ok(_) => panic!("expected parse failure"),
    };
    assert!(!error.contains("secret-do-not-log"));
    assert_eq!(std::fs::read(&store.path).unwrap(), bytes);
    assert!(
        !vault_error(keyring::Error::BadEncoding(b"secret-do-not-log".to_vec()))
            .contains("secret-do-not-log")
    );
}

#[test]
fn repository_binding_and_url_username_must_agree() {
    assert_eq!(selected_username(None, Some("bob")).unwrap(), Some("bob"));
    assert_eq!(
        selected_username(Some("alice"), None).unwrap(),
        Some("alice")
    );
    assert_eq!(
        selected_username(Some("bob"), Some("bob")).unwrap(),
        Some("bob")
    );
    assert!(selected_username(Some("alice"), Some("bob")).is_err());
}

#[test]
fn oauth_rotation_is_verified_in_vault_before_access_is_returned() {
    for fail in [false, true] {
        let (_dir, store) = fixture();
        store
            .save("github.com", "fixture", "old-vault-envelope")
            .unwrap();
        store
            .save("github.com", "other", "unrelated-token")
            .unwrap();
        store.vault.lose_write.set(fail);
        let result = store.lookup_with("github.com", Some("fixture"), |saved, host, user| {
            assert_eq!(
                (saved, host, user),
                ("old-vault-envelope", "github.com", "fixture")
            );
            Ok(("test-new-access".into(), Some("new-vault-envelope".into())))
        });
        if fail {
            assert!(result.is_err());
        } else {
            assert_eq!(result.unwrap().unwrap().1, "test-new-access");
        }
        assert_eq!(
            store.vault.get("github.com", "fixture").unwrap().as_deref(),
            Some(if fail {
                "old-vault-envelope"
            } else {
                "new-vault-envelope"
            })
        );
        assert_eq!(
            store.vault.get("github.com", "other").unwrap().as_deref(),
            Some("unrelated-token")
        );
        let index = std::fs::read_to_string(&store.path).unwrap();
        assert!(!index.contains("envelope") && !index.contains("test-new-access"));
    }
}
#[test]
fn transport_usernames_keep_bitbucket_email_for_api_and_pat_identity_unchanged() {
    assert_eq!(
        transport_username("bitbucket.org", "user@example.com", "test-pat", false).unwrap(),
        "x-bitbucket-api-token-auth"
    );
    assert_eq!(
        transport_username("bitbucket.org", "user@example.com", "test-pat", true).unwrap(),
        "user@example.com"
    );
    assert_eq!(
        transport_username("github.com", "fixture", "test-pat", false).unwrap(),
        "fixture"
    );
}

#[test]
fn bitbucket_clone_username_is_a_hint_with_bound_single_and_ambiguous_accounts() {
    let (_dir, store) = fixture();
    let endpoint =
        super::super::endpoint::parse("https://alice@bitbucket.org/workspace/repo.git", false)
            .unwrap();
    store
        .save("bitbucket.org", "alice@example.com", "alice-api-token")
        .unwrap();
    let selected = remote_username(&endpoint, Some("alice@example.com"), &[]).unwrap();
    assert_eq!(
        store.lookup(&endpoint.host, selected).unwrap().unwrap().0,
        "alice@example.com"
    );
    let selected = remote_username(&endpoint, None, &[]).unwrap();
    assert_eq!(
        store.lookup(&endpoint.host, selected).unwrap().unwrap().0,
        "alice@example.com"
    );
    store
        .save("bitbucket.org", "bob@example.com", "bob-api-token")
        .unwrap();
    assert!(store.lookup(&endpoint.host, selected).is_err());
    let selected = remote_username(&endpoint, Some("bob@example.com"), &[]).unwrap();
    assert_eq!(
        store.lookup(&endpoint.host, selected).unwrap().unwrap().0,
        "bob@example.com"
    );
    let api_endpoint =
        super::super::endpoint::parse("https://alice@bitbucket.org/workspace/repo.git", true)
            .unwrap();
    assert_eq!(
        remote_username(&api_endpoint, Some("alice@example.com"), &[]).unwrap(),
        Some("alice@example.com")
    );
}

#[test]
fn known_oauth_transport_users_require_exact_host_kind_and_unambiguous_account() {
    use super::super::{ProviderKind, StoredAccount};
    for (provider, host, placeholder) in [
        (ProviderKind::GitHubCom, "github.com", "x-access-token"),
        (ProviderKind::GitLabCom, "gitlab.com", "oauth2"),
    ] {
        let (_dir, store) = fixture();
        store.save(host, "alice", "test-alice-token").unwrap();
        let endpoint = super::super::endpoint::parse(
            &format!("https://{placeholder}@{host}/team/repo.git"),
            false,
        )
        .unwrap();
        let accounts = [StoredAccount {
            provider,
            host: host.into(),
            username: "alice".into(),
        }];
        let selected = remote_username(&endpoint, Some("alice"), &accounts).unwrap();
        assert_eq!(selected, Some("alice"));
        let selected = remote_username(&endpoint, None, &accounts).unwrap();
        assert_eq!(store.lookup(host, selected).unwrap().unwrap().0, "alice");
        store.save(host, "bob", "test-bob-token").unwrap();
        assert!(store.lookup(host, selected).is_err());
        assert!(remote_username(&endpoint, Some("alice"), &[]).is_err());
        let arbitrary = super::super::endpoint::parse(
            &format!("https://someone-else@{host}/team/repo.git"),
            false,
        )
        .unwrap();
        assert!(remote_username(&arbitrary, Some("alice"), &accounts).is_err());
        let mut wrong_host = accounts[0].clone();
        wrong_host.host = format!("other.{host}");
        assert!(remote_username(&endpoint, Some("alice"), &[wrong_host]).is_err());
        let mut wrong_kind = accounts[0].clone();
        wrong_kind.provider = ProviderKind::AzureDevOps;
        assert!(remote_username(&endpoint, Some("alice"), &[wrong_kind]).is_err());
    }
}
