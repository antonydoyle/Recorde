use egui::Context;

use crate::git::operations::load_repo;
use crate::state::SharedState;

pub struct Toolbar {
    state: SharedState,
}

impl Toolbar {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub fn show(&mut self, ctx: &Context) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open Repository").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        load_repo(path, self.state.clone());
                    }
                }

                let state = self.state.lock().unwrap();
                if let Some(ref path) = state.repo_path {
                    ui.separator();
                    ui.label(path.display().to_string());
                }
                if let Some(ref err) = state.error {
                    ui.separator();
                    ui.colored_label(egui::Color32::RED, err);
                }
            });
        });
    }
}
