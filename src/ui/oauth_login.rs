//! Device sign-in UI owns its worker and unsaved result. Dropping or cancelling
//! the view discards completion messages; authorization workers never save tokens.
use crate::auth::{oauth::{Client, Event, Login, Session}, ProviderKind};
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
        self.login = None; self.verification = None; self.ready = None; self.error = None;
    }
    fn poll(&mut self) {
        let mut complete = None;
        if let Some(login) = &self.login {
            loop {
                match login.events.try_recv() {
                    Ok(Event::Verification { code, url, expires_in }) => self.verification = Some((code, url, Instant::now() + Duration::from_secs(expires_in))),
                    Ok(Event::Complete(result)) => { complete = Some(result); break; }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => { complete = Some(Err("Sign-in stopped; start again.".into())); break; }
                }
            }
        }
        if let Some(result) = complete {
            self.login = None; self.verification = None;
            match result { Ok(session) => self.ready = Some(session), Err(error) => self.error = Some(error) }
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, provider: &ProviderKind, host: &str, saved: Option<&Client>, saving: bool) -> (Option<Client>, Option<Session>) {
        if self.target.as_ref() != Some(&(provider.clone(), host.to_string())) {
            self.cancel();
            self.target = Some((provider.clone(), host.to_string()));
            self.client_id = saved.filter(|client| &client.provider == provider && client.host == host).map(|client| client.client_id.clone()).unwrap_or_default();
        }
        if !Client::supported(provider) {
            ui.label("Browser sign-in is available for GitHub and GitLab. Use a token for this provider.");
            return (None, None);
        }
        self.poll();
        ui.heading("Browser sign-in");
        ui.label("Use a public client ID registered for this exact provider host, with device authorization enabled. No client secret is needed or stored.");
        let editing = self.login.is_none() && self.ready.is_none() && !saving;
        ui.add_enabled(editing, egui::TextEdit::singleline(&mut self.client_id).hint_text("Registered public client ID"));
        let mut configured = None;
        let mut save = None;
        if self.login.is_none() && self.ready.is_none() {
            if ui.add_enabled(editing && !self.client_id.trim().is_empty(), egui::Button::new("Start browser sign-in")).clicked() {
                let client = Client { provider: provider.clone(), host: host.into(), client_id: self.client_id.trim().into() };
                match client.validate() {
                    Ok(()) => { self.error = None; self.login = Some(Login::start(client.clone())); configured = Some(client); }
                    Err(error) => self.error = Some(error),
                }
            }
        }
        if self.login.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
            if let Some((code, url, deadline)) = &self.verification {
                ui.label(format!("Enter code {code} at {url}"));
                ui.label(format!("Expires in {} seconds", deadline.saturating_duration_since(Instant::now()).as_secs()));
                ui.horizontal(|ui| {
                    if ui.button("Copy verification code").clicked() { ui.output_mut(|out| out.copied_text = code.clone()); }
                    if ui.button("Open browser").clicked() {
                        if let Err(error) = crate::external::open_url(url) { self.error = Some(error); }
                    }
                });
            } else { ui.label("Requesting a verification code..."); }
            if ui.button("Cancel browser sign-in").clicked() { self.cancel(); }
        }
        if let Some(session) = &self.ready {
            ui.label(format!("Verified {} on {}. Save this account to the OS credential store?", session.username, session.client.host));
            if ui.add_enabled(!saving, egui::Button::new("Save signed-in account")).clicked() { save = self.ready.take(); }
            if ui.button("Discard sign-in").clicked() { self.cancel(); }
        }
        if let Some(error) = &self.error { ui.colored_label(crate::theme::accent_red(), error); }
        (configured, save)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> Session { Session { client: Client { provider: ProviderKind::GitHubCom,host:"github.com".into(),client_id:"test-client".into() }, username:"fixture".into(),identity:"42".into(),access_token:"test-access".into(),refresh_token:None,expires_at:None,refresh_expires_at:None } }
    #[test]
    fn cancelled_or_replaced_sign_in_cannot_deliver_a_saved_account() {
        let mut ui = BrowserLogin::default();
        let (login, tx) = Login::test_channel();
        ui.login = Some(login);
        assert!(tx.send(Event::Complete(Ok(session()))).is_ok());
        ui.cancel(); ui.poll(); assert!(ui.ready.is_none());
        let (old, tx) = Login::test_channel(); ui.login=Some(old);
        let (new, new_tx) = Login::test_channel(); ui.login=Some(new);
        assert!(tx.send(Event::Complete(Ok(session()))).is_err());
        assert!(new_tx.send(Event::Complete(Ok(session()))).is_ok());
        ui.poll(); assert!(ui.ready.is_some());
        ui.cancel(); assert!(ui.ready.is_none());
    }
}
