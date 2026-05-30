use egui::{Color32, RichText, Ui};

use crate::state::{ModalKind, SharedState};
use crate::theme;

/// Actions the sidebar can request from the app.
pub enum SidebarAction {
    CheckoutBranch(String),
    OpenCreateBranch { from_oid: Option<String> },
    DeleteBranch(String),
    CreateTag(String),   // tag name to create at HEAD
    DeleteTag(String),
    StashApply(usize),
    StashPop(usize),
    StashDrop(usize),
}

pub struct Sidebar {
    state: SharedState,
}

impl Sidebar {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    /// Returns an action if the user triggered one via a context menu.
    pub fn show(&mut self, ui: &mut Ui) -> Option<SidebarAction> {
        ui.painter().rect_filled(ui.max_rect(), 0.0, theme::BG_SURFACE);

        // Snapshot everything needed, then drop the lock.
        let (graph_opt, head, stashes) = {
            let s = self.state.lock().unwrap();
            (
                s.commit_graph.clone(),
                s.head_branch.clone().unwrap_or_default(),
                s.stashes.clone(),
            )
        };

        let mut action: Option<SidebarAction> = None;

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.add_space(6.0);

                let Some(ref graph) = graph_opt else {
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new("No repository open.")
                            .color(theme::TEXT_MUTED)
                            .size(12.0),
                    );
                    return;
                };

                // Categorise refs
                let mut local: Vec<&str> = Vec::new();
                let mut remote: Vec<&str> = Vec::new();
                let mut tags: Vec<&str> = Vec::new();

                let mut all_refs: Vec<&str> = graph
                    .nodes
                    .iter()
                    .flat_map(|n| n.refs.iter().map(|r| r.as_str()))
                    .collect();
                all_refs.sort_unstable();
                all_refs.dedup();

                for r in &all_refs {
                    if r.contains('/') {
                        remote.push(r);
                    } else if r.starts_with('v')
                        && r.chars().nth(1).map_or(false, |c| c.is_ascii_digit())
                    {
                        tags.push(r);
                    } else {
                        local.push(r);
                    }
                }

                // ── Local Branches ────────────────────────────────────────
                section(ui, "LOCAL BRANCHES", local.len(), |ui| {
                    for name in &local {
                        let is_head = *name == head.as_str();
                        let resp = branch_row(ui, name, is_head, false);
                        resp.context_menu(|ui| {
                            ui.set_min_width(160.0);
                            if !is_head {
                                if ui.button("Checkout").clicked() {
                                    action = Some(SidebarAction::CheckoutBranch(name.to_string()));
                                    ui.close_menu();
                                }
                                ui.separator();
                            }
                            if ui.button("Create Branch From Here").clicked() {
                                action = Some(SidebarAction::OpenCreateBranch { from_oid: None });
                                ui.close_menu();
                            }
                            if !is_head {
                                ui.separator();
                                if ui
                                    .add(egui::Button::new(
                                        RichText::new("Delete Branch")
                                            .color(theme::ACCENT_RED),
                                    ))
                                    .clicked()
                                {
                                    action = Some(SidebarAction::DeleteBranch(name.to_string()));
                                    ui.close_menu();
                                }
                            }
                        });
                    }
                    if local.is_empty() {
                        empty_label(ui);
                    }
                });

                ui.add_space(4.0);

                // ── Remote Branches ───────────────────────────────────────
                section(ui, "REMOTES", remote.len(), |ui| {
                    for name in &remote {
                        branch_row(ui, name, false, true);
                    }
                    if remote.is_empty() {
                        empty_label(ui);
                    }
                });

                ui.add_space(4.0);

                // ── Tags ──────────────────────────────────────────────────
                section(ui, "TAGS", tags.len(), |ui| {
                    for name in &tags {
                        let resp = tag_row(ui, name);
                        resp.context_menu(|ui| {
                            ui.set_min_width(130.0);
                            if ui.add(egui::Button::new(
                                RichText::new("Delete Tag").color(theme::ACCENT_RED),
                            )).clicked() {
                                action = Some(SidebarAction::DeleteTag(name.to_string()));
                                ui.close_menu();
                            }
                        });
                    }
                    // Create tag at HEAD
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        if ui.add(egui::Button::new(
                            RichText::new("+ Create Tag").color(theme::TEXT_MUTED).size(11.0),
                        ).fill(Color32::TRANSPARENT).stroke(egui::Stroke::NONE)).clicked() {
                            action = Some(SidebarAction::CreateTag(String::new()));
                        }
                    });
                    if tags.is_empty() { empty_label(ui); }
                });

                ui.add_space(4.0);

                // ── Stashes ───────────────────────────────────────────────
                section(ui, "STASHES", stashes.len(), |ui| {
                    if stashes.is_empty() {
                        empty_label(ui);
                    } else {
                        for stash in &stashes {
                            let display = stash
                                .message
                                .trim_start_matches("On ")
                                .to_string();
                            let resp = ui
                                .horizontal(|ui| {
                                    ui.add_space(12.0);
                                    let (r, _) = ui.allocate_exact_size(
                                        egui::Vec2::splat(8.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter()
                                        .circle_filled(r.center(), 3.0, theme::ACCENT_YELLOW);
                                    ui.add_space(4.0);
                                    ui.label(
                                        RichText::new(&display)
                                            .color(theme::TEXT_SECONDARY)
                                            .size(12.0),
                                    );
                                })
                                .response;

                            let idx = stash.index;
                            resp.context_menu(|ui| {
                                ui.set_min_width(120.0);
                                if ui.button("Apply").clicked() {
                                    action = Some(SidebarAction::StashApply(idx));
                                    ui.close_menu();
                                }
                                if ui.button("Pop").clicked() {
                                    action = Some(SidebarAction::StashPop(idx));
                                    ui.close_menu();
                                }
                                ui.separator();
                                if ui
                                    .add(egui::Button::new(
                                        RichText::new("Drop").color(theme::ACCENT_RED),
                                    ))
                                    .clicked()
                                {
                                    action = Some(SidebarAction::StashDrop(idx));
                                    ui.close_menu();
                                }
                            });
                        }
                    }
                });
            });

        action
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn section(ui: &mut Ui, title: &str, count: usize, add_contents: impl FnOnce(&mut Ui)) {
    egui::CollapsingHeader::new(
        RichText::new(title)
            .color(theme::TEXT_MUTED)
            .size(10.5)
            .strong(),
    )
    .default_open(count > 0 && count <= 30)
    .show(ui, |ui| {
        ui.add_space(2.0);
        add_contents(ui);
        ui.add_space(2.0);
    });
}

fn branch_row(ui: &mut Ui, name: &str, is_head: bool, is_remote: bool) -> egui::Response {
    let resp = ui
        .horizontal(|ui| {
            ui.add_space(12.0);
            let (dot_rect, _) = ui.allocate_exact_size(egui::Vec2::splat(8.0), egui::Sense::hover());
            let dot_color = if is_head {
                theme::ACCENT_GREEN
            } else if is_remote {
                Color32::from_rgb(0x3A, 0x6A, 0xCC)
            } else {
                theme::TEXT_MUTED
            };
            ui.painter().circle_filled(dot_rect.center(), 3.0, dot_color);
            ui.add_space(4.0);
            let text = if is_head {
                RichText::new(name).color(theme::TEXT_PRIMARY).size(12.0).strong()
            } else {
                RichText::new(name).color(theme::TEXT_SECONDARY).size(12.0)
            };
            ui.label(text);
        })
        .response;

    if resp.hovered() {
        ui.painter().rect_filled(resp.rect, 2.0, Color32::from_white_alpha(8));
    }
    resp
}

fn tag_row(ui: &mut Ui, name: &str) -> egui::Response {
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        let (r, _) = ui.allocate_exact_size(egui::Vec2::splat(8.0), egui::Sense::hover());
        let c = r.center();
        let s = 3.0_f32;
        ui.painter().add(egui::Shape::convex_polygon(
            vec![
                egui::Pos2::new(c.x, c.y - s),
                egui::Pos2::new(c.x + s, c.y),
                egui::Pos2::new(c.x, c.y + s),
                egui::Pos2::new(c.x - s, c.y),
            ],
            theme::ACCENT_PURPLE,
            egui::Stroke::NONE,
        ));
        ui.add_space(4.0);
        ui.label(RichText::new(name).color(theme::TEXT_SECONDARY).size(12.0));
    }).response
}

fn empty_label(ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.add_space(20.0);
        ui.label(RichText::new("None").color(theme::TEXT_MUTED).size(11.0));
    });
}
