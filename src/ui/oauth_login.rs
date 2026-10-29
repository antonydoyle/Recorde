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

        if self.target.as_ref() != Some(&(provider.clone(), host.to_string())) {
            self.cancel();
            self.target = Some((provider.clone(), host.to_string()));
            self.client_id = saved
                .filter(|client| &client.provider == provider && client.host == host)
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
        kit::hint(
            ui,
            "Recorde has no built-in OAuth app. Register an OAuth application for this host \
             with device flow enabled, then enter its public client ID. No client secret is \
             needed or stored.",
        );
        let editing = self.login.is_none() && self.ready.is_none() && !saving;
        kit::setting_row(
            ui,
            "OAuth application",
            "Use an app registered for this exact host.",
            |ui| {
                let label = ui.label("Client ID");
                ui.add_enabled(
                    editing,
                    egui::TextEdit::singleline(&mut self.client_id)
                        .desired_width(ui.available_width())
                        .hint_text("Registered public client ID"),
                )
                .labelled_by(label.id);
            },
        );
        let mut configured = None;
        let mut save = None;
        if self.login.is_none() && self.ready.is_none() {
            if ui
                .add_enabled(
                    editing && !self.client_id.trim().is_empty(),
                    kit::secondary_button("Start browser sign-in"),
                )
                .on_disabled_hover_text(
                    "Enter your registered application's public client ID first, and wait for \
                     any account save to finish.",
                )
                .clicked()
            {
                let client = Client {
                    provider: provider.clone(),
                    host: host.into(),
                    client_id: self.client_id.trim().into(),
                };
                match client.validate() {
                    Ok(()) => {
                        self.error = None;
                        self.login = Some(Login::start(client.clone()));
                        configured = Some(client);
                    }
                    Err(error) => self.error = Some(error),
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;
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
