use crate::git::commit_settings::{self, Signing};
use crate::ui::button::Button;
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
};

#[derive(Default)]
pub struct CommitSigning {
    values: Option<Signing>,
    pending: Option<Receiver<Result<Signing, String>>>,
    message: Option<String>,
}

impl CommitSigning {
    pub fn reload(&mut self, path: PathBuf) {
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        self.message = None;
        self.values = None;
        std::thread::spawn(move || {
            let _ = tx.send(commit_settings::load(&path));
        });
    }

    pub fn show(&mut self, ui: &mut egui::Ui, path: PathBuf, state: crate::state::SharedState) {
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(result) => {
                    self.pending = None;
                    match result {
                        Ok(values) => {
                            self.values = Some(values);
                            self.message = None;
                        }
                        Err(error) => self.message = Some(error),
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.message =
                        Some("Could not load signing settings. Reload to try again.".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.values.is_none() && self.pending.is_none() && self.message.is_none() {
            self.reload(path.clone());
        }
        ui.label(egui::RichText::new("Commit signing").strong());
        ui.label("Use Git's existing keys and signing agent. Recorde does not store private keys or passphrases.");
        ui.label("Saves to the current worktree when worktree settings are enabled; otherwise to the shared repository settings.");
        if let Some(message) = &self.message {
            ui.colored_label(crate::theme::accent_red(), message);
        }
        let busy = self.pending.is_some();
        if busy {
            ui.spinner();
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        let mut save = false;
        if let Some(values) = &mut self.values {
            ui.add_enabled_ui(!busy, |ui| {
                ui.checkbox(&mut values.enabled, "Sign commits");
                egui::ComboBox::from_id_salt("commit_signing_format")
                    .selected_text(&values.format).show_ui(ui, |ui| {
                        for (value, label) in [("openpgp", "OpenPGP"), ("ssh", "SSH"), ("x509", "X.509")] {
                            ui.selectable_value(&mut values.format, value.into(), label);
                        }
                    });
                ui.label("Key ID, public key, or key path");
                ui.label("A blank key removes this scope's override and uses Git's inherited/default key.");
                ui.add(egui::TextEdit::singleline(&mut values.key).desired_width(430.0));
                ui.label("SSH: choose a public key available to your agent, or an existing key path. OpenPGP/X.509: use your key ID.");
                save = ui.button("Save commit signing").clicked();
            });
        }
        if ui
            .add_enabled(!busy, Button::new("Reload signing settings"))
            .clicked()
        {
            self.reload(path.clone());
        }
        if save {
            let values = self.values.clone().unwrap();
            let (tx, rx) = mpsc::channel();
            self.pending = Some(rx);
            self.message = None;
            let report_path = path.clone();
            crate::git::mutation::spawn_job(
                path.clone(),
                "Save commit signing",
                state,
                move || {
                    commit_settings::save(&path, &values)?;
                    commit_settings::load(&path)
                },
                move |result, state| {
                    if let Err(error) = &result {
                        let mut state = state.lock().unwrap();
                        let error = crate::auth::redact::text(error);
                        state.console_log.push(format!(
                            "Signing settings ({}): {error}",
                            report_path.display()
                        ));
                        if state.repo_path.as_ref() == Some(&report_path) {
                            state.error = Some(error);
                        }
                    }
                    let _ = tx.send(result);
                },
            );
        }
    }
}
