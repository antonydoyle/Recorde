//! Device sign-in UI owns its worker and unsaved result. Dropping or cancelling
//! the view discards completion messages; authorization workers never save tokens.
use crate::auth::{
    oauth::{Client, Event, Login, Session},
    ProviderKind,
};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) struct BrowserLogin {
    target: Option<(ProviderKind, String)>,
    client_id: String,
    login: Option<Login>,
    verification: Option<(String, String, Instant)>,
    ready: Option<Session>,
    error: Option<String>,
}
impl BrowserLogin {
    pub fn cancel(&mut self) {
        self.login = None;
        self.verification = None;
        self.ready = None;
        self.error = None;
    }
    fn poll(&mut self) {
        let mut complete = None;
        if let Some(login) = &self.login {
            loop {
                match login.events.try_recv() {
                    Ok(Event::Verification {
                        code,
                        url,
                        expires_in,
                    }) => {
                        self.verification =
                            Some((code, url, Instant::now() + Duration::from_secs(expires_in)))
                    }
                    Ok(Event::Complete(result)) => {
                        complete = Some(result);
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        complete = Some(Err("Sign-in stopped; start again.".into()));
                        break;
                    }
                }
            }
        }
        if let Some(result) = complete {
            self.login = None;
            self.verification = None;
            match result.and_then(|session| {
                crate::auth::store::validate_oauth_storage(&session)?;
                Ok(session)
            }) {
                Ok(session) => self.ready = Some(session),
                Err(error) => self.error = Some(error),
            }
        }
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        provider: &ProviderKind,
        host: &str,
        saved: Option<&Client>,
        saving: bool,
    ) -> (Option<Client>, Option<Session>) {
        use crate::ui::{kit, motion};

        let built_in = crate::auth::oauth::built_in(provider, host);
        if self.target.as_ref() != Some(&(provider.clone(), host.to_string())) {
            self.cancel();
            self.target = Some((provider.clone(), host.to_string()));
            // Only a user-registered app is remembered; the built-in one comes from the build.
            self.client_id = saved
                .filter(|client| &client.provider == provider && client.host == host)
                .filter(|client| !client.is_built_in())
                .map(|client| client.client_id.clone())
                .unwrap_or_default();
        }
        if !Client::supported(provider) {
            kit::hint(
                ui,
                "Browser sign-in is available for GitHub and GitLab. Use a token for this \
                 provider.",
            );
            return (None, None);
        }
        self.poll();
        let editing = self.login.is_none() && self.ready.is_none() && !saving;
        let client_id_row = |ui: &mut egui::Ui, client_id: &mut String| {
            kit::setting_row(
                ui,
                "OAuth application",
                "Use an app registered for this exact host.",
                |ui| {
                    let label = ui.label("Client ID");
                    ui.add_enabled(
                        editing,
                        crate::ui::kit::text_field(client_id)
                            .desired_width(ui.available_width())
                            .hint_text("Registered public client ID"),
                    )
                    .labelled_by(label.id);
                },
            );
        };
        if built_in.is_none() {
            kit::hint(
                ui,
                "This server needs its own OAuth application. Register one with device flow \
                 enabled, then enter its public client ID. No client secret is needed or \
                 stored.",
            );
            client_id_row(ui, &mut self.client_id);
        }
        let chosen = chosen_client(provider, host, &self.client_id, built_in.clone());
        let mut configured = None;
        let mut save = None;
        if self.login.is_none() && self.ready.is_none() {
            let button = if built_in.is_some() {
                kit::primary_button("Sign in with browser")
            } else {
                kit::secondary_button("Sign in with browser")
            };
            let disabled_reason = if chosen.is_none() {
                "Enter your registered application's public client ID first."
            } else {
                "Wait for the account save to finish."
            };
            let response = ui.add_enabled(editing && chosen.is_some(), button);
            if kit::explain_disabled(response, disabled_reason)
                .on_hover_text("Opens a sign-in page where you approve Recorde with a short code.")
                .clicked()
            {
                if let Some(client) = chosen {
                    match client.validate() {
                        Ok(()) => {
                            self.error = None;
                            self.login = Some(Login::start(client.clone()));
                            // Remember only apps the user registered.
                            if !client.is_built_in() {
                                configured = Some(client);
                            }
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
            }
        }
        if built_in.is_some() {
            kit::disclosure(
                ui,
                "own_oauth_app",
                "Use your own OAuth app",
                !self.client_id.is_empty(),
                |ui| {
                    kit::hint(
                        ui,
                        "Only needed if your organisation requires its own registered app. \
                         Leave it blank to use Recorde's.",
                    );
                    client_id_row(ui, &mut self.client_id);
                },
            );
        }
        let progress_id = ui.make_persistent_id("browser_sign_in_progress");
        motion::reveal(ui, progress_id, self.login.is_some(), |ui| {
            if self.login.is_some() {
                ui.ctx().request_repaint_after(Duration::from_millis(100));
                kit::card(ui, |ui| {
                    if let Some((code, url, deadline)) = &self.verification {
                        kit::group_label(ui, "Enter this code in your browser");
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(code).monospace().size(26.0).strong(),
                            )
                            .wrap()
                            .selectable(true),
                        );
                        kit::hint(ui, &format!("Go to {url}"));
                        kit::hint(
                            ui,
                            &format!(
                                "Expires in {} seconds. Keep this code private.",
                                deadline.saturating_duration_since(Instant::now()).as_secs()
                            ),
                        );
                        ui.horizontal_wrapped(|ui| {
                            if ui.add(kit::secondary_button("Copy code")).clicked() {
                                ui.ctx().copy_text(code.clone());
                            }
                            if ui.add(kit::secondary_button("Open browser")).clicked() {
                                if let Err(error) = crate::external::open_url(url) {
                                    self.error = Some(error);
                                }
                            }
                        });
                    } else {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            kit::hint(ui, "Requesting a verification code…");
                        });
                    }
                    if ui
                        .add(kit::secondary_button("Cancel browser sign-in"))
                        .clicked()
                    {
                        self.cancel();
                    }
                });
            }
        });
        let ready_id = ui.make_persistent_id("browser_sign_in_ready");
        motion::reveal(ui, ready_id, self.ready.is_some(), |ui| {
            if let Some((username, host)) = self
                .ready
                .as_ref()
                .map(|session| (session.username.clone(), session.client.host.clone()))
            {
                kit::card(ui, |ui| {
                    kit::status_chip(ui, kit::Status::Ok, "Ready to save");
                    kit::hint(
                        ui,
                        &format!(
                            "Verified {username}@{host}. Save this account to the system \
                             credential store to use it in Recorde."
                        ),
                    );
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(!saving, kit::primary_button("Save signed-in account"))
                            .on_disabled_hover_text(
                                "Wait for the credential store operation to finish.",
                            )
                            .clicked()
                        {
                            // Retain the verified grant so a locked vault can be retried.
                            save = self.ready.clone();
                        }
                        if ui.add(kit::secondary_button("Discard sign-in")).clicked() {
                            self.cancel();
                        }
                    });
                });
            }
        });
        let error_id = ui.make_persistent_id("browser_sign_in_error");
        motion::reveal(ui, error_id, self.error.is_some(), |ui| {
            if let Some(error) = &self.error {
                kit::callout(ui, kit::Status::Error, error);
            }
        });
        (configured, save)
    }
}

/// A client ID the user typed wins; otherwise the build's built-in app, if any.
fn chosen_client(
    provider: &ProviderKind,
    host: &str,
    custom: &str,
    built_in: Option<Client>,
) -> Option<Client> {
    let custom = custom.trim();
    if custom.is_empty() {
        return built_in;
    }
    Some(Client {
        provider: provider.clone(),
        host: host.into(),
        client_id: custom.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_client_id_overrides_the_built_in_app() {
        let built_in = Client {
            provider: ProviderKind::GitHubCom,
            host: "github.com".into(),
            client_id: "built-in".into(),
        };
        let github = ProviderKind::GitHubCom;
        assert_eq!(
            chosen_client(&github, "github.com", "  ", Some(built_in.clone())),
            Some(built_in.clone())
        );
        let custom = chosen_client(&github, "github.com", " mine ", Some(built_in)).unwrap();
        assert_eq!(custom.client_id, "mine");
        assert!(chosen_client(&github, "github.com", "", None).is_none());
    }
    fn session() -> Session {
        Session {
            client: Client {
                provider: ProviderKind::GitHubCom,
                host: "github.com".into(),
                client_id: "test-client".into(),
            },
            username: "fixture".into(),
            identity: "42".into(),
            access_token: "test-access".into(),
            refresh_token: None,
            expires_at: None,
            refresh_expires_at: None,
        }
    }
    #[test]
    fn cancelled_or_replaced_sign_in_cannot_deliver_a_saved_account() {
        let mut ui = BrowserLogin::default();
        let (login, tx) = Login::test_channel();
        ui.login = Some(login);
        assert!(tx.send(Event::Complete(Ok(session()))).is_ok());
        ui.cancel();
        ui.poll();
        assert!(ui.ready.is_none());
        let (old, tx) = Login::test_channel();
        ui.login = Some(old);
        let (new, new_tx) = Login::test_channel();
        ui.login = Some(new);
        assert!(tx.send(Event::Complete(Ok(session()))).is_err());
        assert!(new_tx.send(Event::Complete(Ok(session()))).is_ok());
        ui.poll();
        assert!(ui.ready.is_some());
        ui.cancel();
        assert!(ui.ready.is_none());
    }

    #[test]
    fn rendered_completion_requires_explicit_save_and_discard_clears_it() {
        let ctx = egui::Context::default();
        let mut login = BrowserLogin {
            target: Some((ProviderKind::GitHubCom, "github.com".into())),
            ..Default::default()
        };
        let frame = |login: &mut BrowserLogin, events| {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 700.0),
                )),
                events,
                ..Default::default()
            });
            let mut saved = None;
            egui::CentralPanel::default().show(
                &mut egui::Ui::new(
                    ctx.clone(),
                    egui::Id::new("test_root"),
                    egui::UiBuilder::new().max_rect(ctx.content_rect()),
                ),
                |ui| {
                    saved = login
                        .show(ui, &ProviderKind::GitHubCom, "github.com", None, false)
                        .1;
                },
            );
            let mut output = ctx.end_pass();
            output.textures_delta.clear();
            (output, saved)
        };
        let pointer = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        for (label, expect_saved) in [("Discard sign-in", false), ("Save signed-in account", true)]
        {
            let (worker, tx) = Login::test_channel();
            login.login = Some(worker);
            assert!(tx.send(Event::Complete(Ok(session()))).is_ok());
            let (output, saved) = frame(&mut login, vec![]);
            assert!(
                saved.is_none(),
                "worker completion must never save by itself"
            );
            assert!(login.ready.is_some());
            let position = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(text.galley.rect.translate(text.pos.to_vec2()).center())
                    }
                    _ => None,
                })
                .expect("the explicit completion action is rendered");
            let (_, saved) = frame(
                &mut login,
                vec![egui::Event::PointerMoved(position), pointer(position, true)],
            );
            assert!(saved.is_none());
            let (_, saved) = frame(&mut login, vec![pointer(position, false)]);
            assert_eq!(saved.is_some(), expect_saved);
            assert_eq!(login.ready.is_some(), expect_saved);
            if expect_saved {
                // A failed save leaves the verified result available for retry.
                assert!(frame(&mut login, vec![]).1.is_none());
                assert!(login.ready.is_some());
                login.cancel();
                assert!(login.ready.is_none());
            }
        }
    }
}
