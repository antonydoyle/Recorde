use egui::{Color32, RichText, Ui};

use crate::state::SharedState;
use crate::theme;

pub struct Sidebar {
    state: SharedState,
}

impl Sidebar {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, theme::BG_SURFACE);

        let state = self.state.lock().unwrap();

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.add_space(6.0);

                let Some(ref graph) = state.commit_graph else {
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new("No repository open.")
                            .color(theme::TEXT_MUTED)
                            .size(12.0),
                    );
                    return;
                };

                // Split refs into categories
                let mut local_branches: Vec<&str> = Vec::new();
                let mut remote_branches: Vec<&str> = Vec::new();
                let mut tags: Vec<&str> = Vec::new();

                // Collect unique refs across all nodes
                let mut all_refs: Vec<&str> = graph
                    .nodes
                    .iter()
                    .flat_map(|n| n.refs.iter().map(|r| r.as_str()))
                    .collect();
                all_refs.sort_unstable();
                all_refs.dedup();

                for r in &all_refs {
                    if r.starts_with("origin/") || r.contains('/') {
                        remote_branches.push(r);
                    } else if r.starts_with('v')
                        && r.chars().nth(1).map_or(false, |c| c.is_ascii_digit())
                    {
                        tags.push(r);
                    } else {
                        local_branches.push(r);
                    }
                }

                let head = state.head_branch.as_deref().unwrap_or("");

                // --- Local Branches ---
                section(ui, "LOCAL BRANCHES", local_branches.len(), |ui| {
                    for name in &local_branches {
                        let is_head = *name == head;
                        branch_row(ui, name, is_head, false);
                    }
                    if local_branches.is_empty() {
                        empty_label(ui);
                    }
                });

                ui.add_space(4.0);

                // --- Remote Branches ---
                section(ui, "REMOTES", remote_branches.len(), |ui| {
                    for name in &remote_branches {
                        branch_row(ui, name, false, true);
                    }
                    if remote_branches.is_empty() {
                        empty_label(ui);
                    }
                });

                ui.add_space(4.0);

                // --- Tags ---
                section(ui, "TAGS", tags.len(), |ui| {
                    for name in &tags {
                        tag_row(ui, name);
                    }
                    if tags.is_empty() {
                        empty_label(ui);
                    }
                });

                ui.add_space(4.0);

                // --- Stashes (placeholder) ---
                section(ui, "STASHES", 0, |ui| {
                    empty_label(ui);
                });
            });
    }
}

fn section(ui: &mut Ui, title: &str, count: usize, add_contents: impl FnOnce(&mut Ui)) {
    let default_open = count > 0 && count <= 30;
    egui::CollapsingHeader::new(
        RichText::new(title)
            .color(theme::TEXT_MUTED)
            .size(10.5)
            .strong(),
    )
    .default_open(default_open)
    .show(ui, |ui| {
        ui.add_space(2.0);
        add_contents(ui);
        ui.add_space(2.0);
    });
}

fn branch_row(ui: &mut Ui, name: &str, is_head: bool, is_remote: bool) {
    let response = ui
        .horizontal(|ui| {
            ui.add_space(12.0);

            // Dot indicator
            let (dot_rect, _) = ui.allocate_exact_size(
                egui::Vec2::splat(8.0),
                egui::Sense::hover(),
            );
            let dot_color = if is_head {
                theme::ACCENT_GREEN
            } else if is_remote {
                theme::ACCENT_BLUE.gamma_multiply(0.6)
            } else {
                theme::TEXT_MUTED
            };
            ui.painter()
                .circle_filled(dot_rect.center(), 3.0, dot_color);

            ui.add_space(4.0);

            let text = if is_head {
                RichText::new(name)
                    .color(theme::TEXT_PRIMARY)
                    .size(12.0)
                    .strong()
            } else {
                RichText::new(name)
                    .color(theme::TEXT_SECONDARY)
                    .size(12.0)
            };
            ui.label(text);
        })
        .response;

    // Subtle hover background
    if response.hovered() {
        ui.painter().rect_filled(
            response.rect,
            2.0,
            Color32::from_white_alpha(8),
        );
    }
}

fn tag_row(ui: &mut Ui, name: &str) {
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(8.0), egui::Sense::hover());
        // Small diamond for tags
        let c = rect.center();
        let r = 3.0_f32;
        ui.painter().add(egui::Shape::convex_polygon(
            vec![
                egui::Pos2::new(c.x, c.y - r),
                egui::Pos2::new(c.x + r, c.y),
                egui::Pos2::new(c.x, c.y + r),
                egui::Pos2::new(c.x - r, c.y),
            ],
            theme::ACCENT_PURPLE,
            egui::Stroke::NONE,
        ));
        ui.add_space(4.0);
        ui.label(RichText::new(name).color(theme::TEXT_SECONDARY).size(12.0));
    });
}

fn empty_label(ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.add_space(20.0);
        ui.label(RichText::new("None").color(theme::TEXT_MUTED).size(11.0));
    });
}
