use std::path::PathBuf;

use egui::{Context, RichText};

use crate::state::SharedState;
use crate::theme;

/// Actions the toolbar can request from the app.
pub enum ToolbarAction {
    OpenRepo(PathBuf),
    Stash,
    CreateBranch,
}

pub struct Toolbar {
    state: SharedState,
}

impl Toolbar {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub fn show(&mut self, ctx: &Context) -> Option<ToolbarAction> {
        let mut action = None;

        egui::TopBottomPanel::top("toolbar")
            .exact_height(36.0)
            .frame(
                egui::Frame::none()
                    .fill(theme::BG_ELEVATED)
                    .inner_margin(egui::Margin::symmetric(8.0, 0.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    // Open repo
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("⊞  Open Repository")
                                    .color(theme::TEXT_PRIMARY)
                                    .size(12.5),
                            )
                            .fill(theme::BG_SURFACE)
                            .stroke(egui::Stroke::new(1.0, theme::BORDER)),
                        )
                        .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            action = Some(ToolbarAction::OpenRepo(path));
                        }
                    }

                    ui.add_space(6.0);
                    ui.add(egui::Separator::default().vertical().spacing(4.0));
                    ui.add_space(6.0);

                    let has_repo = self.state.lock().unwrap().repo_path.is_some();

                    // New Branch button
                    if ui
                        .add_enabled(
                            has_repo,
                            egui::Button::new(
                                RichText::new("⎇  New Branch")
                                    .color(theme::TEXT_SECONDARY)
                                    .size(12.0),
                            )
                            .fill(theme::BG_SURFACE)
                            .stroke(egui::Stroke::new(1.0, theme::BORDER)),
                        )
                        .clicked()
                    {
                        action = Some(ToolbarAction::CreateBranch);
                    }

                    ui.add_space(4.0);

                    // Stash button
                    let (dirty, has_stashes) = {
                        let s = self.state.lock().unwrap();
                        (s.dirty_count > 0, !s.stashes.is_empty())
                    };

                    let stash_label = if has_stashes {
                        RichText::new("⚑  Stash")
                    } else {
                        RichText::new("⚑  Stash")
                    };

                    if ui
                        .add_enabled(
                            has_repo && dirty,
                            egui::Button::new(
                                stash_label.color(theme::TEXT_SECONDARY).size(12.0),
                            )
                            .fill(theme::BG_SURFACE)
                            .stroke(egui::Stroke::new(1.0, theme::BORDER)),
                        )
                        .on_hover_text("Stash all uncommitted changes")
                        .clicked()
                    {
                        action = Some(ToolbarAction::Stash);
                    }

                    ui.add_space(8.0);
                    ui.add(egui::Separator::default().vertical().spacing(4.0));
                    ui.add_space(8.0);

                    // Repo name
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

        action
    }
}
