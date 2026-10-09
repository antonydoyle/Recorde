use crate::git::conflict_edit::{self, Snapshot};
use crate::state::SharedState;

struct Loaded {
    snapshot: Snapshot,
    draft: Result<Option<conflict_edit::Draft>, String>,
}

pub struct ConflictEditor {
    path: std::path::PathBuf,
    file: String,
    snapshot: Option<Snapshot>,
    loading: Option<std::sync::mpsc::Receiver<Result<Loaded, String>>>,
    result: String,
    persisted: String,
    stale_draft: Option<String>,
    draft_error: Option<String>,
    error: Option<String>,
    saving: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
}
impl ConflictEditor {
    pub fn new(path: std::path::PathBuf, file: String) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let repo = path.clone();
        let name = file.clone();
        std::thread::spawn(move || {
            let loaded = conflict_edit::load(&repo, &name).map(|snapshot| {
                let draft = conflict_edit::read_draft(&snapshot);
                Loaded { snapshot, draft }
            });
            let _ = tx.send(loaded);
        });
        Self {
            path,
            file,
            snapshot: None,
            loading: Some(rx),
            result: String::new(),
            persisted: String::new(),
            stale_draft: None,
            draft_error: None,
            error: None,
            saving: None,
        }
    }
    /// Returns true when closed or submitted. Continue/Abort remain explicit.
    pub fn show(&mut self, ctx: &egui::Context, state: SharedState) -> bool {
        if state.lock().unwrap().repo_path.as_ref() != Some(&self.path) {
            return true;
        }
        if let Some(rx) = &self.saving {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    self.snapshot = None;
                    return true;
                }
                Ok(Err(error)) => {
                    self.error = Some(error);
                    self.saving = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.error = Some("Save stopped; your result is retained.".into());
                    self.saving = None;
                }
                _ => {}
            }
        }
        if let Some(rx) = &self.loading {
            match rx.try_recv() {
                Ok(result) => {
                    match result {
                        Ok(Loaded { snapshot, draft }) => {
                            self.result = snapshot.result.clone();
                            match draft {
                                Ok(Some(draft)) if draft.matches(&snapshot) => {
                                    self.result = draft.result
                                }
                                Ok(Some(draft)) => self.stale_draft = Some(draft.result),
                                Err(error) => {
                                    self.draft_error = Some(format!(
                                        "Could not read saved conflict draft: {error}"
                                    ))
                                }
                                _ => {}
                            }
                            self.persisted = self.result.clone();
                            self.snapshot = Some(snapshot);
                        }
                        Err(error) => self.error = Some(error),
                    }
                    self.loading = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.error =
                        Some("Conflict loading stopped. Reopen the editor to retry.".into());
                    self.loading = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        let mut open = true;
        let mut save = false;
        egui::Window::new(format!("Resolve {}", self.file)).open(&mut open).default_width(1000.0)
            .show(ctx, |ui| {
                if self.saving.is_some() { ui.disable(); }
                if let Some(error) = &self.error { ui.colored_label(egui::Color32::RED, error); }
                if let Some(error) = &self.draft_error { ui.colored_label(egui::Color32::RED, error); }
                if self.stale_draft.is_some() {
                    ui.label("A saved draft belongs to an earlier version of this conflict. Compare it with the current versions before saving.");
                    if ui.button("Recover earlier draft into result").clicked() {
                        self.result = self.stale_draft.take().unwrap();
                    }
                }
                let Some(snapshot) = &self.snapshot else { if self.loading.is_some() { ui.spinner(); } return; };
                ui.label("Base is the common ancestor. During rebase, ours is the target branch and theirs is the replayed commit.");
                ui.columns(3, |columns| {
                    for (column, (label, text)) in columns.iter_mut().zip([("Base", &snapshot.base), ("Ours", &snapshot.ours), ("Theirs", &snapshot.theirs)]) {
                        column.label(label);
                        egui::ScrollArea::both().id_source(label).max_height(220.0).show(column, |ui| {
                            ui.add(egui::TextEdit::multiline(&mut text.as_str()).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY));
                        });
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Use ours").clicked() { self.result = snapshot.ours.clone(); }
                    if ui.button("Use theirs").clicked() { self.result = snapshot.theirs.clone(); }
                    if ui.button("Reset result").clicked() { self.result = snapshot.result.clone(); }
                });
                ui.label("Result — edit to combine the versions, then remove every conflict marker.");
                egui::ScrollArea::both().id_source("conflict_result").max_height(300.0).show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut self.result).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).desired_rows(12));
                });
                save = ui.button("Save and mark resolved").clicked();
            });
        self.persist_draft();
        if save {
            if let Some(snapshot) = self.snapshot.clone() {
                let result = self.result.clone();
                let (tx, rx) = std::sync::mpsc::channel();
                self.saving = Some(rx);
                let work_snapshot = snapshot.clone();
                crate::git::mutation::spawn_job(
                    self.path.clone(),
                    "Resolve conflict",
                    state,
                    move || conflict_edit::save(&work_snapshot, &result),
                    move |saved, state| {
                        let _ = tx.send(saved);
                        if state.lock().unwrap().repo_path.as_ref() == Some(&snapshot.repo) {
                            crate::git::operations::refresh_repo(state.clone());
                        }
                    },
                );
            }
        }
        !open && self.saving.is_none()
    }

    fn persist_draft(&mut self) {
        if self.persisted == self.result {
            return;
        }
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        match conflict_edit::write_draft(snapshot, &self.result) {
            Ok(()) => {
                self.persisted = self.result.clone();
                self.draft_error = None;
            }
            Err(error) => {
                self.draft_error = Some(format!("Conflict draft could not be saved: {error}"))
            }
        }
    }
}

impl Drop for ConflictEditor {
    fn drop(&mut self) {
        self.persist_draft();
        if let Some(error) = &self.draft_error {
            log::warn!("{error}");
        }
    }
}
