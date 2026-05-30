use egui::{Context, RichText};

use crate::git::operations::load_repo;
use crate::state::SharedState;
use crate::theme;

pub struct Toolbar {
    state: SharedState,
}

impl Toolbar {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub fn show(&mut self, ctx: &Context) {
        egui::TopBottomPanel::top("toolbar")
            .exact_height(36.0)
            .frame(
                egui::Frame::none()
                    .fill(theme::BG_ELEVATED)
                    .inner_margin(egui::Margin::symmetric(8.0, 0.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    // Open repo button
                    let btn = ui.add(
                        egui::Button::new(
                            RichText::new("⊞  Open Repository")
                                .color(theme::TEXT_PRIMARY)
                                .size(12.5),
                        )
                        .fill(theme::BG_SURFACE)
                        .stroke(egui::Stroke::new(1.0, theme::BORDER)),
                    );
                    if btn.clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            load_repo(path, self.state.clone());
                        }
                    }

                    ui.add_space(8.0);
                    ui.add(egui::Separator::default().vertical().spacing(4.0));
                    ui.add_space(8.0);

                    // Repo path display
                    let state = self.state.lock().unwrap();
                    if let Some(ref path) = state.repo_path {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.display().to_string());
                        ui.label(
                            RichText::new(name)
                                .color(theme::TEXT_PRIMARY)
                                .size(13.0)
                                .strong(),
                        );
                    }
                });
            });
    }
}
