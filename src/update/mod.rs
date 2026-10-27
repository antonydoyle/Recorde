//! Opt-in authenticated application updates.
mod apply;
mod instance;
mod network;
mod persistence;
mod security;
pub(crate) use security::create_private_file;
mod witness;
pub use apply::{acknowledge, startup_recovery_check};
pub use instance::{register as register_instance, InstanceGuard};
use recorde_update_signing::{self as protocol, Target};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
pub fn keys() -> Result<protocol::KeySet, String> {
    serde_json::from_slice(include_bytes!("../../packaging/update-keys.json"))
        .map_err(|_| "Installed update signing keys are invalid.".into())
}
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
pub fn run_helper(path: &std::path::Path) -> Result<(), String> {
    apply::run(path)
}
fn token(account: Option<&str>) -> Result<Option<String>, String> {
    let Some(account) = account else {
        return Ok(None);
    };
    let mut url = url::Url::parse("https://github.com/antonydoyle/Recorde.git").unwrap();
    url.set_username(account)
        .map_err(|_| "Invalid selected update account.")?;
    let (matched, token) = crate::auth::store::lookup_for_url(url.as_str(), None)?
        .ok_or("The explicitly selected GitHub account has no saved credential.")?;
    if matched != account {
        return Err("Selected update account identity changed.".into());
    }
    Ok(Some(token))
}
fn target(channel: &str) -> Target {
    let kind = if cfg!(target_os = "macos") {
        "dmg".into()
    } else {
        apply::installation()
            .map(|(_, kind)| kind)
            .unwrap_or_else(|_| "portable".into())
    };
    Target {
        platform: std::env::consts::OS.into(),
        arch: if cfg!(target_os = "macos") {
            "universal".into()
        } else {
            std::env::consts::ARCH.into()
        },
        package_kind: kind,
        channel: channel.into(),
    }
}
struct Staged {
    directory: PathBuf,
    envelope: Vec<u8>,
    target: Target,
    artifact_name: String,
    can_install: bool,
}
enum Event {
    Checked(network::Offer, Target),
    Downloaded(Staged),
    ApplyReady,
}
pub struct Controller {
    preferences: persistence::Preferences,
    error: Option<String>,
    status: String,
    task: Option<Receiver<Result<Event, String>>>,
    offer: Option<(network::Offer, Target)>,
    staged: Option<Staged>,
    pub applying: bool,
    pub ready_to_close: bool,
    confirm: bool,
    persistence_failed: bool,
    cancel: Arc<AtomicBool>,
}
impl Default for Controller {
    fn default() -> Self {
        std::thread::spawn(apply::cleanup_confirmed_attempts);
        let result = persistence::preferences();
        let persistence_failed = result.is_err();
        let error = result.as_ref().err().cloned();
        Self {
            preferences: result.unwrap_or_default(),
            error,
            status: persistence::latest_outcome().unwrap_or_default(),
            task: None,
            offer: None,
            staged: None,
            applying: false,
            ready_to_close: false,
            confirm: false,
            persistence_failed,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl Controller {
    pub fn poll(&mut self, ctx: &egui::Context) {
        if let Some(task) = &self.task {
            match task.try_recv() {
                Ok(result) => {
                    self.task = None;
                    match result {
                        Ok(Event::Checked(offer, target)) => {
                            self.status = format!(
                                "Authenticated update {} is available.",
                                offer.manifest.version
                            );
                            self.offer = Some((offer, target));
                        }
                        Ok(Event::Downloaded(staged)) => {
                            self.status = "Download verified and ready.".into();
                            self.staged = Some(staged);
                        }
                        Ok(Event::ApplyReady) => self.ready_to_close = true,
                        Err(error) => {
                            self.error = Some(error);
                            self.applying = false;
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.task = None;
                    self.error = Some("Update worker ended unexpectedly.".into());
                    self.applying = false;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100))
                }
            }
        }
        if self.preferences.automatic_checks
            && !self.persistence_failed
            && self.task.is_none()
            && !self.applying
            && now().saturating_sub(self.preferences.last_check) > 24 * 60 * 60
        {
            self.check();
        }
    }
    fn spawn(&mut self, work: impl FnOnce() -> Result<Event, String> + Send + 'static) {
        self.cancel.store(false, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.task = Some(rx);
        self.error = None;
        std::thread::spawn(move || {
            let _ = tx.send(work());
        });
    }
    fn save(&mut self) {
        if let Err(error) = persistence::save_preferences(&self.preferences) {
            self.error = Some(error);
            self.persistence_failed = true;
        }
    }
    fn check(&mut self) {
        if self.persistence_failed {
            return;
        }
        self.preferences.last_check = now();
        self.save();
        if self.persistence_failed {
            return;
        }
        self.offer = None;
        self.staged = None;
        self.confirm = false;
        let target = target(&self.preferences.channel);
        let account = self.preferences.account.clone();
        let cancel = self.cancel.clone();
        self.status = "Checking signed release metadata…".into();
        self.spawn(move || {
            let token = token(account.as_deref())?;
            let offer = network::check(&target, token.as_deref(), &keys()?)?;
            if cancel.load(Ordering::Relaxed) {
                return Err("Update check cancelled.".into());
            }
            persistence::accept(
                &persistence::root()?,
                &offer.manifest,
                &target,
                env!("CARGO_PKG_VERSION"),
                now(),
            )?;
            Ok(Event::Checked(offer, target))
        });
    }
    fn download(&mut self) {
        let Some((offer, target)) = &self.offer else {
            return;
        };
        let envelope = offer.envelope.clone();
        let target = target.clone();
        let account = self.preferences.account.clone();
        let cancel = self.cancel.clone();
        self.status = "Downloading and verifying the signed artifact…".into();
        self.spawn(move || {
            let manifest = protocol::verify(&envelope, &keys()?)?;
            let root = persistence::root()?;
            persistence::accept(&root, &manifest, &target, env!("CARGO_PKG_VERSION"), now())?;
            let directory = security::private_tempdir(&root, "release-")?;
            security::owned_directory(directory.path())?;
            let path = directory.path().join(&manifest.artifact_name);
            let mut file = security::create_private(&path)?;
            let token = token(account.as_deref())?;
            network::download(&manifest, token.as_deref(), &mut file, &cancel)?;
            file.sync_all()
                .map_err(|_| "Cannot flush verified download.")?;
            drop(file);
            use std::io::Write;
            let mut receipt = security::create_private(&directory.path().join("envelope.json"))?;
            receipt
                .write_all(&envelope)
                .and_then(|_| receipt.sync_all())
                .map_err(|_| "Cannot save update receipt.")?;
            drop(receipt);
            let can_install = !cfg!(target_os = "macos")
                && apply::installation().is_ok()
                && (target.package_kind != "deb" || apply::deb_helper_available());
            Ok(Event::Downloaded(Staged {
                directory: directory.keep(),
                envelope,
                target,
                artifact_name: manifest.artifact_name,
                can_install,
            }))
        });
    }
    pub fn start_apply(&mut self) {
        let Some(staged) = &self.staged else { return };
        let directory = staged.directory.clone();
        let envelope = staged.envelope.clone();
        let target = staged.target.clone();
        self.applying = true;
        self.status = "Preparing verified installation handoff…".into();
        self.spawn(move || {
            apply::start(&directory, &envelope, &target)?;
            Ok(Event::ApplyReady)
        });
    }
    pub fn report_error(&mut self, error: String) {
        self.error = Some(error);
        self.confirm = false;
    }
    /// Returns true only after the user confirms apply; the caller must guard operations and flush drafts.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        accounts: &[crate::auth::StoredAccount],
        can_apply: bool,
    ) -> bool {
        ui.heading("Application updates");
        ui.label(format!("Installed version {}", env!("CARGO_PKG_VERSION")));
        ui.label("Recorde verifies every update before installation.");
        let busy = self.task.is_some() || self.applying;
        ui.add_enabled_ui(!busy && !self.persistence_failed, |ui| {
            if ui
                .checkbox(
                    &mut self.preferences.automatic_checks,
                    "Check automatically once a day",
                )
                .changed()
            {
                self.save();
            }
            let mut changed = false;
            egui::ComboBox::from_id_salt("update-channel")
                .selected_text(&self.preferences.channel)
                .show_ui(ui, |ui| {
                    for channel in ["stable", "preview"] {
                        changed |= ui
                            .selectable_value(
                                &mut self.preferences.channel,
                                channel.into(),
                                channel,
                            )
                            .changed();
                    }
                });
            egui::ComboBox::from_id_salt("update-account")
                .selected_text(
                    self.preferences
                        .account
                        .as_deref()
                        .unwrap_or("Public access (no account)"),
                )
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(
                            &mut self.preferences.account,
                            None,
                            "Public access (no account)",
                        )
                        .changed();
                    for account in accounts
                        .iter()
                        .filter(|account| account.host == "github.com")
                    {
                        changed |= ui
                            .selectable_value(
                                &mut self.preferences.account,
                                Some(account.username.clone()),
                                &account.username,
                            )
                            .changed();
                    }
                });
            if changed {
                self.offer = None;
                self.staged = None;
                self.confirm = false;
                self.status.clear();
                self.save();
            }
            ui.label("Private releases need a GitHub account with access to antonydoyle/Recorde.");
            if ui.button("Check for updates").clicked() {
                self.check();
            }
            if self.offer.is_some()
                && self.staged.is_none()
                && ui.button("Download verified update").clicked()
            {
                self.download();
            }
        });
        if busy {
            ui.spinner();
            if !self.applying && ui.button("Cancel download/check").clicked() {
                self.cancel.store(true, Ordering::Relaxed);
            }
        }
        if !self.status.is_empty() {
            ui.label(&self.status);
        }
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if self.persistence_failed && !busy && ui.button("Retry loading update settings").clicked()
        {
            match persistence::preferences() {
                Ok(preferences) => {
                    self.preferences = preferences;
                    self.offer = None;
                    self.staged = None;
                    self.confirm = false;
                    self.status.clear();
                    self.persistence_failed = false;
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
        let mut apply = false;
        if let Some(staged) = &self.staged {
            ui.label(format!("Verified download: {}", staged.directory.display()));
            ui.label(&staged.artifact_name);
            if ui.button("Open download folder").clicked() {
                crate::git::lifecycle::reveal_in_file_manager(&staged.directory);
            }
            let supported = staged.can_install;
            if ui
                .add_enabled(
                    !busy && supported && can_apply,
                    egui::Button::new("Install and restart…"),
                )
                .clicked()
            {
                self.confirm = true;
            }
            if !supported {
                ui.label("Automatic installation is unavailable here. Open the download folder to install the verified update manually.");
            }
            if !can_apply {
                ui.label("Finish repository operations, clones and merge/rebase sequences before restarting.");
            }
            if self.confirm {
                if staged.target.package_kind == "msi" {
                    ui.label("Windows Installer may ask for administrator permission.");
                }
                ui.label("Recorde will save your drafts, close, and install the update. It will reopen when installation finishes.");
                if ui
                    .add_enabled(
                        !busy && can_apply,
                        egui::Button::new("Confirm install and restart"),
                    )
                    .clicked()
                {
                    apply = true;
                    self.confirm = false;
                }
                if ui.button("Cancel").clicked() {
                    self.confirm = false;
                }
            }
        }
        apply
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
