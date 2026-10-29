//! Settings > Accounts & SSH: provider accounts, browser sign-in, tokens,
//! SSH keys and commit identities.
#![allow(unused_imports)]
use super::*;

impl RecordeApp {
    pub(super) fn render_accounts_tab(&mut self, ui: &mut egui::Ui) {
        use crate::ui::{kit, motion};
        use kit::Status;

        if self.account_task.is_some() {
            ui.disable();
        }
        kit::page_header(
            ui,
            "Accounts & SSH",
            "Connect to GitHub, GitLab and other hosts, and choose who your commits are from.",
        );
        kit::callout(
            ui,
            Status::Info,
            "Connect with a hosting account (HTTPS) or an SSH key. Accounts use a securely \
             stored token and enable provider features such as pull requests. SSH keys let Git \
             connect without a password: keep the private key here and add the public key to \
             your hosting account. Repository browsing is available for GitHub and GitLab.",
        );
        ui.add_space(kit::space::MD);
        let accounts = self.config.accounts.clone();
        let repo_path = self.state.lock().unwrap().repo_path.clone();
        kit::section(
            ui,
            "Hosting accounts",
            "Use accounts to push and pull over HTTPS and access your host's features. Tokens \
             are saved in the system credential store; Recorde does not fall back to plain text.",
            |ui| {
                for acc in &accounts {
                    ui.push_id((&acc.host, &acc.username), |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(crate::icons::ACCOUNT_CIRCLE.to_string())
                                    .font(crate::icons::font(20.0))
                                    .color(theme::text_secondary()),
                            );
                            ui.label(RichText::new(acc.provider.label()).strong());
                            kit::status_chip(ui, Status::Ok, "Connected").on_hover_text(
                                "Account saved on this computer. Access depends on the \
                                     token's permissions and expiry.",
                            );
                        });
                        ui.add(egui::Label::new(format!("{}@{}", acc.username, acc.host)).wrap());
                        if let (Some(path), Ok(host)) =
                            (&repo_path, crate::auth::endpoint::normalize_host(&acc.host))
                        {
                            if self
                                .config
                                .get_repo_config(path)
                                .provider_accounts
                                .get(&host)
                                == Some(&acc.username)
                            {
                                kit::status_chip(ui, Status::Info, "Used for this repository");
                            }
                        }
                        let confirm_id = ui.make_persistent_id("remove_account");
                        let mut confirm = motion::is_open(ui.ctx(), confirm_id, false);
                        if ui.add(kit::danger_outline_button("Remove…")).clicked() {
                            confirm = true;
                        }
                        motion::reveal(ui, confirm_id, confirm, |ui| {
                            kit::callout(
                                ui,
                                Status::Warning,
                                &format!(
                                    "Remove {}@{}? Recorde will forget this token. HTTPS Git \
                                     operations for {} may need another account or credentials \
                                     from your Git credential helper. Update any repository or \
                                     profile that selects this account. This does not revoke \
                                     the token on the host or remove SSH keys.",
                                    acc.username, acc.host, acc.host
                                ),
                            );
                            ui.horizontal_wrapped(|ui| {
                                if ui.add(kit::secondary_button("Cancel")).clicked() {
                                    confirm = false;
                                }
                                if ui.add(kit::danger_button("Remove account")).clicked() {
                                    let account = acc.clone();
                                    self.account_task = Some(account_task(move || {
                                        crate::auth::store::remove(
                                            &account.host,
                                            &account.username,
                                        )?;
                                        Ok(AccountChange::Removed(account))
                                    }));
                                    confirm = false;
                                }
                            });
                        });
                        motion::set_open(ui.ctx(), confirm_id, confirm);
                    });
                    kit::row_divider(ui);
                }
                if let Some(path) = &repo_path {
                    let groups = accounts_by_host(&accounts);
                    if !groups.is_empty() {
                        kit::group_label(ui, "Account for this repository");
                        kit::hint(
                            ui,
                            "Choose one account per host when you have several. Automatic uses \
                             the remote's account or your global profile; if several still \
                             match, Recorde asks you to choose here.",
                        );
                    }
                    for (host, usernames) in groups {
                        let mut selected = self
                            .config
                            .get_repo_config(path)
                            .provider_accounts
                            .get(&host)
                            .cloned()
                            .unwrap_or_default();
                        let previous_selection = selected.clone();
                        kit::setting_row(ui, &host, "", |ui| {
                            egui::ComboBox::from_id_salt(("repo_account", &host))
                                .selected_text(if selected.is_empty() {
                                    "Automatic"
                                } else {
                                    &selected
                                })
                                .width(ui.available_width().min(260.0))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut selected, String::new(), "Automatic");
                                    for username in usernames {
                                        ui.selectable_value(
                                            &mut selected,
                                            username.clone(),
                                            &username,
                                        );
                                    }
                                });
                        });
                        if selected != previous_selection {
                            let previous = self.config.get_repo_config(path);
                            let repo = self
                                .config
                                .per_repo
                                .entry(path.display().to_string())
                                .or_default();
                            if selected.is_empty() {
                                repo.provider_accounts.remove(&host);
                            } else {
                                repo.provider_accounts.insert(host, selected);
                            }
                            if let Err(error) = self.config.try_save() {
                                self.config
                                    .per_repo
                                    .insert(path.display().to_string(), previous);
                                self.add_account_error = Some(error);
                            } else {
                                self.state.lock().unwrap().invalidate_provider_data();
                            }
                        }
                    }
                    if !accounts.is_empty() {
                        kit::row_divider(ui);
                    }
                }
                let add_button_id = ui.make_persistent_id("add_account_button");
                motion::reveal(ui, add_button_id, !self.add_account_open, |ui| {
                    let add = if accounts.is_empty() {
                        kit::empty_state(
                            ui,
                            crate::icons::ACCOUNT_CIRCLE,
                            "No accounts connected",
                            "Add an account to push and pull over HTTPS and to see pull \
                             requests in Recorde.",
                            Some("Add account…"),
                        )
                    } else {
                        ui.add(kit::secondary_button("Add account…")).clicked()
                    };
                    if add {
                        self.add_account_open = true;
                        self.add_account_provider = ProviderKind::GitHubCom;
                        self.add_account_host = ProviderKind::GitHubCom.default_host().to_string();
                        self.add_account_error = None;
                    }
                });
                let form_id = ui.make_persistent_id("add_account_form");
                motion::reveal(ui, form_id, self.add_account_open, |ui| {
                    kit::group_label(ui, "1. Where is your code hosted?");
                    for (provider, description) in [
                        (ProviderKind::GitHubCom, "GitHub.com"),
                        (
                            ProviderKind::GitHubEnterprise,
                            "GitHub Enterprise — your company's own GitHub server",
                        ),
                        (ProviderKind::GitLabCom, "GitLab.com"),
                        (
                            ProviderKind::GitLabSelf,
                            "GitLab self-managed — your own GitLab server",
                        ),
                        (ProviderKind::BitbucketCloud, "Bitbucket Cloud"),
                        (ProviderKind::AzureDevOps, "Azure DevOps"),
                    ] {
                        if ui
                            .radio_value(
                                &mut self.add_account_provider,
                                provider.clone(),
                                description,
                            )
                            .changed()
                        {
                            self.add_account_host = provider.default_host().to_string();
                        }
                    }
                    let host_id = ui.make_persistent_id("custom_account_host");
                    motion::reveal(ui, host_id, self.add_account_provider.needs_host(), |ui| {
                        kit::setting_row(
                            ui,
                            "Host",
                            "Server name only, for example git.example.com. \
                             An HTTPS port is optional.",
                            |ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.add_account_host)
                                        .desired_width(ui.available_width()),
                                );
                            },
                        );
                    });
                    let eff_host = self
                        .add_account_provider
                        .effective_host(&self.add_account_host)
                        .trim()
                        .to_string();
                    kit::row_divider(ui);
                    kit::group_label(ui, "2. Sign in");
                    kit::hint(ui, token_guidance(&self.add_account_provider));
                    if let Ok(host) = crate::auth::endpoint::normalize_host(&eff_host) {
                        let url = match self.add_account_provider {
                            ProviderKind::GitHubCom | ProviderKind::GitHubEnterprise => {
                                Some(format!("https://{host}/settings/tokens"))
                            }
                            ProviderKind::GitLabCom | ProviderKind::GitLabSelf => Some(format!(
                                "https://{host}/-/user_settings/personal_access_tokens"
                            )),
                            _ => None,
                        };
                        if let Some(url) = url {
                            if ui
                                .add(kit::secondary_button("Open token settings"))
                                .clicked()
                            {
                                if let Err(error) = super::open_url(&url) {
                                    self.add_account_error = Some(error);
                                }
                            }
                        }
                    }
                    kit::setting_row(
                        ui,
                        "Username",
                        "Use your Atlassian email for Bitbucket Cloud.",
                        |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.add_account_username)
                                    .desired_width(ui.available_width()),
                            );
                        },
                    );
                    kit::setting_row(
                        ui,
                        "Personal access token",
                        "Used instead of your password. Keep it private.",
                        |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.add_account_token)
                                    .password(true)
                                    .desired_width(ui.available_width()),
                            );
                        },
                    );
                    let reason = account_save_hint(
                        &self.add_account_username,
                        &self.add_account_token,
                        &eff_host,
                    );
                    let validation_id = ui.make_persistent_id("account_validation");
                    motion::reveal(ui, validation_id, reason.is_some(), |ui| {
                        if let Some(reason) = reason {
                            kit::hint(ui, reason);
                        }
                    });
                    ui.add_space(kit::space::SM);
                    ui.horizontal_wrapped(|ui| {
                        if kit::explain_disabled(
                            ui.add_enabled(
                                reason.is_none() && self.account_task.is_none(),
                                kit::primary_button("Save account"),
                            ),
                            reason.unwrap_or("Wait for the credential store operation to finish."),
                        )
                        .clicked()
                        {
                            let username = self.add_account_username.trim().to_string();
                            let token = self.add_account_token.trim().to_string();
                            let provider = self.add_account_provider.clone();
                            self.account_task = Some(account_task(move || {
                                let host = crate::auth::endpoint::normalize_host(&eff_host)?;
                                crate::auth::store::save(&host, &username, &token)?;
                                Ok(AccountChange::Saved(StoredAccount {
                                    provider,
                                    host,
                                    username,
                                }))
                            }));
                        }
                        if ui.add(kit::secondary_button("Cancel")).clicked() {
                            self.add_account_open = false;
                            self.browser_login.cancel();
                            self.add_account_username.clear();
                            self.add_account_token.clear();
                            self.add_account_error = None;
                        }
                    });
                    kit::row_divider(ui);
                    kit::disclosure(
                        ui,
                        "advanced_browser_sign_in",
                        "Sign in with a browser instead (advanced)",
                        false,
                        |ui| {
                            let browser_host = crate::auth::endpoint::normalize_host(
                                self.add_account_provider
                                    .effective_host(&self.add_account_host),
                            )
                            .unwrap_or_default();
                            let (configured, session) = self.browser_login.show(
                                ui,
                                &self.add_account_provider,
                                &browser_host,
                                self.config.oauth_clients.get(&browser_host),
                                self.account_task.is_some(),
                            );
                            if let Some(client) = configured {
                                self.config
                                    .oauth_clients
                                    .insert(client.host.clone(), client);
                                self.save_config();
                            }
                            if let Some(session) = session {
                                self.account_task = Some(account_task(move || {
                                    crate::auth::store::save_oauth(&session)?;
                                    Ok(AccountChange::Saved(StoredAccount {
                                        provider: session.client.provider,
                                        host: session.client.host,
                                        username: session.username,
                                    }))
                                }));
                            }
                        },
                    );
                });
                let busy_id = ui.make_persistent_id("credential_store_busy");
                motion::reveal(ui, busy_id, self.account_task.is_some(), |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        kit::hint(ui, "Accessing credential store…");
                    });
                });
                let error_id = ui.make_persistent_id("account_error");
                motion::reveal(ui, error_id, self.add_account_error.is_some(), |ui| {
                    if let Some(error) = &self.add_account_error {
                        kit::callout(ui, Status::Error, error);
                        // The shared error is untyped: preserve the existing retry condition.
                        if ui
                            .add_enabled(
                                self.account_task.is_none(),
                                kit::secondary_button("Retry credential migration"),
                            )
                            .clicked()
                        {
                            self.account_task = Some(account_task(|| {
                                crate::auth::store::migrate()?;
                                Ok(AccountChange::Migrated)
                            }));
                        }
                    }
                });
            },
        );
        kit::section(
            ui,
            "SSH keys",
            "SSH keys let Git push and pull without a password. Add the public key to your \
             hosting account; the private key stays on this computer. Provider features may \
             still need a hosting account above.",
            |ui| {
                if let Some(diag) = &self.ssh_diagnostics {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("SSH agent");
                        kit::status_chip(
                            ui,
                            if diag.agent_running {
                                Status::Ok
                            } else {
                                Status::Neutral
                            },
                            if diag.agent_running {
                                "Running"
                            } else {
                                "Not running"
                            },
                        );
                    });
                    kit::hint(
                        ui,
                        if diag.agent_running {
                            if diag.agent_keys.is_empty() {
                                "The agent is available, but no keys are loaded yet."
                            } else {
                                "The agent holds loaded keys so Git can use them to connect."
                            }
                        } else {
                            "The agent is unavailable. Git may still use a key file directly."
                        },
                    );
                    let found: Vec<_> = diag
                        .key_files
                        .iter()
                        .filter(|(_, exists)| *exists)
                        .map(|(name, _)| name.as_str())
                        .collect();
                    if found.is_empty() {
                        kit::status_chip(ui, Status::Neutral, "No SSH keys found");
                        kit::hint(
                            ui,
                            "No standard key files were found; keys with custom names may \
                             still exist.",
                        );
                    } else {
                        ui.add(
                            egui::Label::new(format!("Keys found: {}", found.join(", "))).wrap(),
                        );
                    }
                    kit::disclosure(ui, "ssh_details", "Show technical details", false, |ui| {
                        kit::hint(
                            ui,
                            "Snapshot from the last SSH check. Only standard key file names \
                             are checked.",
                        );
                        let socket =
                            std::env::var("SSH_AUTH_SOCK").unwrap_or_else(|_| "not set".into());
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!("SSH_AUTH_SOCK: {socket}")).monospace(),
                            )
                            .wrap()
                            .selectable(true),
                        );
                        for key in &diag.agent_keys {
                            ui.add(
                                egui::Label::new(RichText::new(key).monospace())
                                    .wrap()
                                    .selectable(true),
                            );
                        }
                        for (name, exists) in &diag.key_files {
                            kit::hint(
                                ui,
                                &format!(
                                    "~/.ssh/{name}: {}",
                                    if *exists { "found" } else { "missing" }
                                ),
                            );
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        kit::hint(ui, "Checking SSH agent and standard key files…");
                    });
                }
                kit::row_divider(ui);
                self.render_ssh_keygen(ui);
            },
        );
        kit::section(
            ui,
            "Commit identity",
            "This name and email are written into every commit you make. Use an email linked \
             to your hosting account so commits are attributed to you.",
            |ui| {
                kit::setting_row(
                    ui,
                    "Commit author name",
                    "Default for all repositories.",
                    |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.git_name_input)
                                .desired_width(ui.available_width()),
                        );
                    },
                );
                kit::setting_row(
                    ui,
                    "Commit email",
                    "Use an email linked to your hosting account.",
                    |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.git_email_input)
                                .desired_width(ui.available_width()),
                        );
                    },
                );
                ui.add_space(kit::space::SM);
                if ui.add(kit::primary_button("Save identity")).clicked() {
                    match save_git_identity(None, &self.git_name_input, &self.git_email_input) {
                        Ok(()) => self.state.lock().unwrap().error = None,
                        Err(error) => self.state.lock().unwrap().error = Some(error),
                    }
                }
                kit::hint(
                    ui,
                    "Repository settings can override this global identity for one repository.",
                );
            },
        );
        self.render_profiles(ui);
        kit::disclosure(
            ui,
            "git_credential_helper",
            "Advanced: Git credential helper",
            false,
            |ui| {
                kit::hint(
                    ui,
                    &format!(
                        "System credential helper: {}",
                        git_config_value(None, "credential.helper")
                            .unwrap_or_else(|| "not configured".into())
                    ),
                );
                kit::hint(
                    ui,
                    "For HTTPS remotes, Recorde's saved tokens take precedence over the system \
                     credential helper, which supplies credentials when no Recorde account \
                     matches.",
                );
            },
        );
    }
}

fn accounts_by_host(accounts: &[StoredAccount]) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut groups = std::collections::BTreeMap::<String, Vec<String>>::new();
    for account in accounts {
        if let Ok(host) = crate::auth::endpoint::normalize_host(&account.host) {
            groups
                .entry(host)
                .or_default()
                .push(account.username.clone());
        }
    }
    for usernames in groups.values_mut() {
        usernames.sort();
        usernames.dedup();
    }
    groups
}

fn account_save_hint(username: &str, token: &str, host: &str) -> Option<&'static str> {
    if host.trim().is_empty() {
        Some("Enter the hostname for your server before saving.")
    } else if username.trim().is_empty() {
        Some("Enter your username before saving.")
    } else if token.trim().is_empty() {
        Some("Enter a personal access token before saving.")
    } else {
        None
    }
}

fn token_guidance(provider: &ProviderKind) -> &'static str {
    match provider {
        ProviderKind::GitHubCom | ProviderKind::GitHubEnterprise => {
            "Create a classic personal access token in GitHub's Developer settings with repo, \
             read:user and workflow scopes. SSH-key upload also needs write:public_key."
        }
        ProviderKind::GitLabCom | ProviderKind::GitLabSelf => {
            "Create a personal access token in GitLab's user settings with api, read_user and \
             write_repository scopes for provider features and HTTPS Git access."
        }
        ProviderKind::BitbucketCloud => {
            "Use your Atlassian email and a scoped API token (repository, pull request and \
             pipeline permissions as needed). Create it in your Atlassian account's \
             security settings."
        }
        ProviderKind::AzureDevOps => {
            "Use your account name and a personal access token with Code, Build, Work Items and \
             Release permissions as needed. Create it in Azure DevOps user settings. Boards and \
             releases show the whole project."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_choices_normalize_sort_deduplicate_and_keep_ports_separate() {
        let accounts: Vec<_> = [
            ("GitHub.COM", "zoe"),
            ("github.com", "amy"),
            ("github.com", "amy"),
            ("github.com:8443", "work"),
            ("https://invalid", "ignored"),
        ]
        .into_iter()
        .map(|(host, username)| StoredAccount {
            provider: ProviderKind::GitHubEnterprise,
            host: host.into(),
            username: username.into(),
        })
        .collect();
        let groups = accounts_by_host(&accounts);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups["github.com"], vec!["amy", "zoe"]);
        assert_eq!(groups["github.com:8443"], vec!["work"]);
        assert!(accounts_by_host(&[]).is_empty());
    }

    #[test]
    fn save_hint_explains_each_existing_required_field() {
        assert!(account_save_hint("", "", "").unwrap().contains("hostname"));
        assert!(account_save_hint(" ", "token", "github.com")
            .unwrap()
            .contains("username"));
        assert!(account_save_hint("user", " ", "github.com")
            .unwrap()
            .contains("token"));
        assert_eq!(account_save_hint("user", "token", "github.com"), None);
    }

    #[test]
    fn token_guidance_covers_each_provider_and_ssh_upload_permissions() {
        for provider in [ProviderKind::GitHubCom, ProviderKind::GitHubEnterprise] {
            let guidance = token_guidance(&provider);
            for permission in ["repo", "read:user", "workflow", "write:public_key"] {
                assert!(guidance.contains(permission));
            }
        }
        for provider in [ProviderKind::GitLabCom, ProviderKind::GitLabSelf] {
            assert!(token_guidance(&provider).contains("api"));
            assert!(token_guidance(&provider).contains("write_repository"));
        }
        assert!(token_guidance(&ProviderKind::BitbucketCloud).contains("Atlassian email"));
        assert!(token_guidance(&ProviderKind::AzureDevOps).contains("Code, Build"));
    }
}
