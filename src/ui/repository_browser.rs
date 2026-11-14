//! Native account repository picker. One bounded worker runs at a time; network
//! requests happen only on explicit load/search/fork actions, never on UI frames.
use crate::auth::StoredAccount;
use crate::providers::repositories::{self, Page, Repository};
use crate::theme;
use std::sync::mpsc;

enum Completion {
    List(Result<Page, String>, bool, String),
    Fork(Result<Repository, String>),
}

struct Pending {
    account: String,
    receiver: mpsc::Receiver<Completion>,
}

#[derive(Default)]
pub struct RepositoryBrowser {
    pub open: bool,
    account: String,
    query: String,
    loaded_query: String,
    items: Vec<Repository>,
    selected: Option<String>,
    next_page: Option<u32>,
    limited: bool,
    loaded: bool,
    ssh: bool,
    pending: Option<Pending>,
    fork_confirmation: Option<Repository>,
    error: Option<String>,
    notice: Option<String>,
}

fn account_key(account: &StoredAccount) -> String {
    format!(
        "{}\n{}\n{}",
        account.provider.label(),
        account.host,
        account.username
    )
}

impl RepositoryBrowser {
    /// Pending reads and writes must finish before restarting for an update.
    /// Closing the window does not cancel a provider request or its result.
    pub fn is_busy(&self) -> bool {
        self.pending.is_some()
    }

    fn reset_results(&mut self) {
        self.loaded_query.clear();
        self.items.clear();
        self.selected = None;
        self.next_page = None;
        self.limited = false;
        self.loaded = false;
        self.fork_confirmation = None;
        self.error = None;
        self.notice = None;
    }

    fn poll(&mut self, accounts: &[StoredAccount]) {
        let Some(pending) = &self.pending else {
            return;
        };
        let completion = match pending.receiver.try_recv() {
            Ok(value) => value,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Completion::List(
                Err("Repository request stopped. Refresh to try again.".into()),
                false,
                String::new(),
            ),
        };
        let current = pending.account == self.account
            && accounts
                .iter()
                .any(|account| account_key(account) == pending.account);
        self.pending = None;
        if !current {
            self.reset_results();
            self.error = Some(
                "The selected account changed. Select an account and load repositories again."
                    .into(),
            );
            return;
        }
        match completion {
            Completion::List(Ok(page), append, query) => {
                if append && query != self.loaded_query {
                    self.error = Some(
                        "Repository results changed. Search again before loading more.".into(),
                    );
                    return;
                }
                if !append {
                    self.items.clear();
                    self.selected = None;
                    // Commit result identity only when its first page succeeds.
                    // A failed search must leave the old page cursor tied to the
                    // old query, so Load more cannot mix two result sets.
                    self.loaded_query = query;
                }
                for repository in page.repositories {
                    if !self.items.iter().any(|item| item.path == repository.path) {
                        self.items.push(repository);
                    }
                }
                self.next_page = page.next_page;
                self.limited = page.limited;
                self.loaded = true;
            }
            Completion::Fork(Ok(repository)) => {
                self.selected = Some(repository.path.clone());
                self.items.retain(|item| item.path != repository.path);
                self.items.insert(0, repository);
                self.notice = Some("Fork accepted. The provider may need a little time to prepare its Git objects before cloning. Refresh your repositories if it is not ready yet.".into());
            }
            Completion::List(Err(error), _, _) | Completion::Fork(Err(error)) => {
                self.error = Some(error)
            }
        }
    }

    fn query_for_page(&self, page: u32) -> Result<String, String> {
        if page == 1 {
            Ok(self.query.trim().to_string())
        } else if self.loaded && self.next_page == Some(page) {
            Ok(self.loaded_query.clone())
        } else {
            Err("Search again before requesting this repository page.".into())
        }
    }

    fn load(&mut self, ctx: &egui::Context, account: StoredAccount, page: u32) {
        if self.pending.is_some() {
            return;
        }
        let query = match self.query_for_page(page) {
            Ok(query) => query,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        self.error = None;
        self.notice = None;
        self.fork_confirmation = None;
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(Pending {
            account: account_key(&account),
            receiver,
        });
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = repositories::list(&account, &query, page);
            let _ = sender.send(Completion::List(result, page > 1, query));
            ctx.request_repaint();
        });
    }

    fn fork(&mut self, ctx: &egui::Context, account: StoredAccount, repository: Repository) {
        if self.pending.is_some() {
            return;
        }
        self.error = None;
        self.notice = None;
        self.fork_confirmation = None;
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(Pending {
            account: account_key(&account),
            receiver,
        });
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = repositories::fork(&account, &repository);
            let _ = sender.send(Completion::Fork(result));
            ctx.request_repaint();
        });
    }

    /// Call every frame to collect finished requests, even while the window is closed.
    /// Returned URLs are ready for the application's existing clone destination form.
    pub fn show(&mut self, ctx: &egui::Context, accounts: &[StoredAccount]) -> Option<String> {
        self.poll(accounts);
        if !self.open {
            return None;
        }
        if !accounts
            .iter()
            .any(|account| account_key(account) == self.account)
            && self.pending.is_none()
        {
            self.account = accounts.first().map(account_key).unwrap_or_default();
            self.reset_results();
        }
        let mut open = self.open;
        let mut clone = None;
        egui::Window::new("Browse provider repositories")
            .id(egui::Id::new("repository_browser"))
            .open(&mut open)
            .default_width(760.0)
            .default_height(560.0)
            .min_width(440.0)
            .resizable(true)
            .show(ctx, |ui| {
                if accounts.is_empty() {
                    ui.label("Connect an account in Settings > Accounts & SSH to browse repositories.");
                    ui.label("You can also clone any repository directly using its HTTPS or SSH URL.");
                    return;
                }
                let busy = self.pending.is_some();
                let previous = self.account.clone();
                ui.add_enabled_ui(!busy, |ui| {
                    egui::ComboBox::from_label("Account")
                        .selected_text(accounts.iter().find(|a| account_key(a) == self.account)
                            .map(|a| format!("{} @ {}", a.username, a.host)).unwrap_or_default())
                        .show_ui(ui, |ui| {
                            for account in accounts {
                                ui.selectable_value(&mut self.account, account_key(account),
                                    format!("{} @ {} · {}", account.username, account.host, account.provider.label()));
                            }
                        });
                });
                if self.account != previous { self.reset_results(); }
                let Some(account) = accounts.iter().find(|a| account_key(a) == self.account).cloned() else { return; };
                if !repositories::supported(&account.provider) {
                    ui.label("Repository browsing and personal forks are available for GitHub and GitLab. Clone this provider using its repository URL.");
                    return;
                }
                ui.add_space(6.0);
                ui.label("Leave search empty for your repositories, or search all repositories visible to this account.");
                let mut search = false;
                ui.add_enabled_ui(!busy, |ui| {
                    ui.horizontal(|ui| {
                        let response = ui.add(crate::ui::kit::text_field(&mut self.query)
                            .hint_text(if matches!(account.provider, crate::auth::ProviderKind::GitHubCom | crate::auth::ProviderKind::GitHubEnterprise) {
                                "Repository name or GitHub search query"
                            } else { "Project name or path" })
                            .desired_width((ui.available_width() - 100.0).max(160.0)));
                        search = ui.button("Search").clicked()
                            || (response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
                    });
                });
                if search { self.load(ctx, account.clone(), 1); }
                if let Some(error) = &self.error { ui.colored_label(theme::accent_red(), error); }
                if let Some(notice) = &self.notice { ui.colored_label(theme::text_secondary(), notice); }
                if busy || search { ui.horizontal(|ui| { ui.spinner(); ui.label("Contacting provider…"); }); }
                if !self.loaded && !busy && !search && self.items.is_empty() {
                    ui.add_space(16.0);
                    ui.label("Choose Search to load repositories. Credentials stay in your OS credential store.");
                }
                if self.loaded && self.items.is_empty() { ui.label("No matching repositories. Try another search or check this account's access."); }
                ui.separator();
                egui::ScrollArea::vertical().id_salt("provider_repositories").max_height(300.0).show(ui, |ui| {
                    for repository in &self.items {
                        let selected = self.selected.as_deref() == Some(repository.path.as_str());
                        let response = ui.selectable_label(selected, egui::RichText::new(&repository.path).strong());
                        if response.clicked() { self.selected = Some(repository.path.clone()); self.fork_confirmation = None; }
                        ui.horizontal_wrapped(|ui| {
                            ui.label(egui::RichText::new(if repository.private { "Private / restricted" } else { "Public" }).small().color(theme::text_muted()));
                            if repository.fork { ui.label(egui::RichText::new("Fork").small().color(theme::text_muted())); }
                            if repository.archived { ui.label(egui::RichText::new("Archived").small().color(theme::accent_yellow())); }
                        });
                        if !repository.description.is_empty() { ui.label(egui::RichText::new(&repository.description).small().color(theme::text_secondary())); }
                        ui.separator();
                    }
                });
                if self.loaded {
                    let scope = if self.loaded_query.is_empty() { "Your repositories".to_string() }
                        else { format!("Search: {}", self.loaded_query) };
                    ui.label(egui::RichText::new(format!("{scope} · {} loaded", self.items.len())).small().color(theme::text_muted()));
                }
                if let Some(page) = self.next_page {
                    if ui.add_enabled(!busy && !search, egui::Button::new("Load more")).clicked() { self.load(ctx, account.clone(), page); }
                }
                if self.limited { ui.label("Showing up to 1,000 repositories. Narrow your search for more specific results."); }
                if let Some(repository) = self.items.iter().find(|item| Some(item.path.as_str()) == self.selected.as_deref()).cloned() {
                    ui.separator();
                    ui.label(egui::RichText::new(&repository.path).strong());
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.ssh, false, "HTTPS");
                        ui.selectable_value(&mut self.ssh, true, "SSH");
                        if !repository.web_url.is_empty() { ui.hyperlink_to("Open on provider", &repository.web_url); }
                    });
                    let url = repositories::clone_url(&repository, &account, self.ssh);
                    if self.ssh { ui.label("SSH uses your configured SSH identity."); }
                    else { ui.label("HTTPS uses the selected saved account."); }
                    match &url {
                        Ok(url) => { ui.add(egui::Label::new(egui::RichText::new(url).monospace().small()).wrap()); }
                        Err(error) => { ui.colored_label(theme::accent_red(), error); }
                    }
                    ui.horizontal(|ui| {
                        if ui.add_enabled(url.is_ok(), egui::Button::new("Copy URL")).clicked() {
                            if let Ok(url) = &url { ui.ctx().copy_text(url.clone()); }
                        }
                        if ui.add_enabled(!busy && !search && url.is_ok(), crate::ui::button::Button::new(
                            egui::RichText::new("Choose clone destination…").color(theme::on_primary())).fill(theme::primary_fill())).clicked() {
                            clone = url.ok();
                        }
                        if ui.add_enabled(!busy && !search, egui::Button::new("Fork to my account…")).clicked() {
                            self.fork_confirmation = Some(repository.clone());
                        }
                    });
                }
                if let Some(repository) = self.fork_confirmation.clone() {
                    ui.separator();
                    ui.label(format!("Create a personal fork of {} as {} on {}?", repository.path, account.username, account.host));
                    ui.label("This creates a repository on the provider. Organization destinations can be selected on the provider website.");
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!busy && !search, egui::Button::new("Create personal fork")).clicked() {
                            self.fork(ctx, account.clone(), repository);
                        }
                        if ui.button("Cancel fork").clicked() { self.fork_confirmation = None; }
                    });
                }
            });
        self.open = open && clone.is_none();
        clone
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> StoredAccount {
        StoredAccount {
            provider: crate::auth::ProviderKind::GitHubCom,
            host: "github.com".into(),
            username: "alice".into(),
        }
    }

    fn repository() -> Repository {
        Repository {
            path: "team/project".into(),
            description: String::new(),
            web_url: "https://github.com/team/project".into(),
            https_url: "https://github.com/team/project.git".into(),
            ssh_url: "git@github.com:team/project.git".into(),
            private: true,
            archived: false,
            fork: false,
        }
    }

    #[test]
    fn completed_requests_do_not_restore_results_for_removed_accounts() {
        let (sender, receiver) = mpsc::channel();
        let mut browser = RepositoryBrowser {
            account: account_key(&account()),
            pending: Some(Pending {
                account: account_key(&account()),
                receiver,
            }),
            ..Default::default()
        };
        sender.send(Completion::Fork(Ok(repository()))).unwrap();
        browser.poll(&[]);
        assert!(browser.pending.is_none());
        assert!(browser.items.is_empty());
        assert!(browser
            .error
            .as_deref()
            .unwrap()
            .contains("account changed"));
    }

    #[test]
    fn pages_deduplicate_repositories_and_forks_select_the_returned_project() {
        let mut browser = RepositoryBrowser {
            account: account_key(&account()),
            ..Default::default()
        };
        for append in [false, true] {
            let (sender, receiver) = mpsc::channel();
            browser.pending = Some(Pending {
                account: account_key(&account()),
                receiver,
            });
            sender
                .send(Completion::List(
                    Ok(Page {
                        repositories: vec![repository()],
                        next_page: Some(2),
                        limited: false,
                    }),
                    append,
                    String::new(),
                ))
                .unwrap();
            browser.poll(&[account()]);
        }
        assert_eq!(browser.items.len(), 1);
        assert_eq!(browser.next_page, Some(2));
        let (sender, receiver) = mpsc::channel();
        browser.pending = Some(Pending {
            account: account_key(&account()),
            receiver,
        });
        let mut fork = repository();
        fork.path = "alice/project".into();
        sender.send(Completion::Fork(Ok(fork))).unwrap();
        browser.poll(&[account()]);
        assert_eq!(browser.selected.as_deref(), Some("alice/project"));
        assert_eq!(browser.items.len(), 2);
        assert!(browser.notice.as_deref().unwrap().contains("prepare"));
    }

    fn deliver(browser: &mut RepositoryBrowser, completion: Completion) {
        let (sender, receiver) = mpsc::channel();
        browser.pending = Some(Pending {
            account: account_key(&account()),
            receiver,
        });
        sender.send(completion).unwrap();
        browser.poll(&[account()]);
    }

    #[test]
    fn failed_search_preserves_the_previous_query_and_pagination_cursor() {
        let mut browser = RepositoryBrowser {
            account: account_key(&account()),
            query: "new query".into(),
            loaded_query: "original query".into(),
            loaded: true,
            items: vec![repository()],
            selected: Some(repository().path),
            next_page: Some(2),
            ..Default::default()
        };
        assert_eq!(browser.query_for_page(1).unwrap(), "new query");
        assert_eq!(browser.loaded_query, "original query");
        deliver(
            &mut browser,
            Completion::List(
                Err("Search was rate limited.".into()),
                false,
                "new query".into(),
            ),
        );
        assert_eq!(browser.items, vec![repository()]);
        assert_eq!(browser.loaded_query, "original query");
        assert_eq!(browser.selected.as_deref(), Some("team/project"));
        assert_eq!(browser.next_page, Some(2));
        assert_eq!(browser.query_for_page(2).unwrap(), "original query");
        assert!(browser.query_for_page(3).is_err());

        let mut next = repository();
        next.path = "team/second".into();
        deliver(
            &mut browser,
            Completion::List(
                Ok(Page {
                    repositories: vec![next.clone()],
                    next_page: Some(3),
                    limited: false,
                }),
                true,
                "original query".into(),
            ),
        );
        assert_eq!(browser.items, vec![repository(), next]);
        assert_eq!(browser.loaded_query, "original query");
        assert_eq!(browser.next_page, Some(3));
    }

    #[test]
    fn successful_search_replaces_results_and_rejects_a_mismatched_append() {
        let mut browser = RepositoryBrowser {
            account: account_key(&account()),
            loaded: true,
            loaded_query: "old query".into(),
            items: vec![repository()],
            selected: Some(repository().path),
            next_page: Some(2),
            ..Default::default()
        };
        let mut replacement = repository();
        replacement.path = "other/repository".into();
        deliver(
            &mut browser,
            Completion::List(
                Ok(Page {
                    repositories: vec![replacement.clone()],
                    next_page: Some(2),
                    limited: false,
                }),
                false,
                "new query".into(),
            ),
        );
        assert_eq!(browser.items, vec![replacement.clone()]);
        assert_eq!(browser.selected, None);
        assert_eq!(browser.loaded_query, "new query");
        assert_eq!(browser.query_for_page(2).unwrap(), "new query");
        deliver(
            &mut browser,
            Completion::List(
                Ok(Page {
                    repositories: vec![repository()],
                    next_page: None,
                    limited: false,
                }),
                true,
                "old query".into(),
            ),
        );
        assert_eq!(browser.items, vec![replacement]);
        assert_eq!(browser.next_page, Some(2));
        assert_eq!(browser.loaded_query, "new query");
        assert!(browser
            .error
            .as_deref()
            .unwrap()
            .contains("results changed"));
    }

    #[test]
    fn closed_browser_remains_busy_until_the_pending_request_is_collected() {
        let (sender, receiver) = mpsc::channel();
        let mut browser = RepositoryBrowser {
            account: account_key(&account()),
            open: false,
            pending: Some(Pending {
                account: account_key(&account()),
                receiver,
            }),
            ..Default::default()
        };
        assert!(browser.is_busy());
        browser.poll(&[account()]);
        assert!(browser.is_busy());
        sender
            .send(Completion::Fork(Err("Fork was refused.".into())))
            .unwrap();
        assert!(
            browser.is_busy(),
            "Uncollected results must still block restart"
        );
        browser.poll(&[account()]);
        assert!(!browser.is_busy());
        assert_eq!(browser.error.as_deref(), Some("Fork was refused."));
    }
}
