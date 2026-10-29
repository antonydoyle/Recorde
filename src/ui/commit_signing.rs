use crate::git::commit_settings::{self, Signing};
use crate::ui::kit;
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
        let busy = self.pending.is_some();
        if busy {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        let mut save = false;
        let mut reload = false;
        kit::setting_row(
            ui,
            "Commit signing",
            "Sign new commits with a key Git already uses, so hosts can mark them as verified. Recorde never stores private keys or passphrases.",
            |ui| {
                if let Some(message) = &self.message {
                    kit::callout(ui, kit::Status::Error, message);
                    ui.add_space(kit::space::SM);
                }
                if busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        kit::hint(ui, "Loading signing settings…");
                    });
                }
                if let Some(values) = &mut self.values {
                    ui.add_enabled_ui(!busy, |ui| {
                        ui.horizontal(|ui| {
                            kit::toggle(ui, &mut values.enabled, "Sign commits");
                            ui.label("Sign commits");
                        });
                        ui.add_space(kit::space::SM);
                        ui.horizontal(|ui| {
                            ui.label("Key type");
                            egui::ComboBox::from_id_salt("commit_signing_format")
                                .selected_text(format_label(&values.format))
                                .show_ui(ui, |ui| {
                                    for value in ["ssh", "openpgp", "x509"] {
                                        ui.selectable_value(
                                            &mut values.format,
                                            value.into(),
                                            format_label(value),
                                        );
                                    }
                                });
                        });
                        ui.add_space(kit::space::SM);
                        ui.label("Signing key");
                        ui.add(
                            egui::TextEdit::singleline(&mut values.key)
                                .hint_text(key_placeholder(&values.format))
                                .desired_width(ui.available_width().min(430.0)),
                        );
                        kit::hint(ui, key_help(&values.format));
                        kit::hint(
                            ui,
                            "Leave it blank to use the key from your global Git settings.",
                        );
                        ui.add_space(kit::space::SM);
                        ui.horizontal(|ui| {
                            save = ui
                                .add(kit::primary_button("Save signing settings"))
                                .on_hover_text("Write these settings to Git's configuration.")
                                .clicked();
                            reload = ui
                                .add(kit::secondary_button("Reload"))
                                .on_hover_text("Discard unsaved edits and read Git's settings again.")
                                .clicked();
                        });
                    });
                } else if !busy {
                    reload = ui.add(kit::secondary_button("Reload signing settings")).clicked();
                }
                kit::hint(
                    ui,
                    "Saved for this worktree when worktree settings are enabled, otherwise for the whole repository.",
                );
            },
        );
        if reload {
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

fn format_label(format: &str) -> &'static str {
    match format {
        "ssh" => "SSH key",
        "openpgp" => "OpenPGP (GPG)",
        "x509" => "X.509 certificate",
        _ => "Other",
    }
}

fn key_placeholder(format: &str) -> &'static str {
    match format {
        "ssh" => "e.g. ~/.ssh/id_ed25519.pub",
        "x509" => "Certificate ID",
        _ => "Key ID, e.g. 3AA5C34371567BD2",
    }
}

fn key_help(format: &str) -> &'static str {
    match format {
        "ssh" => "A public key file, or a key loaded in your SSH agent.",
        "x509" => "The certificate ID your signing tool (gpgsm) knows.",
        _ => "The ID of a key in your GPG keyring (gpg --list-secret-keys).",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_signing_format_has_plain_labels() {
        for format in ["ssh", "openpgp", "x509"] {
            assert_ne!(format_label(format), "Other");
            assert!(!key_placeholder(format).is_empty());
            assert!(!key_help(format).is_empty());
        }
    }
}
