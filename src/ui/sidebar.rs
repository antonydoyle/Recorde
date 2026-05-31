use std::collections::HashMap;

use egui::{Color32, Pos2, Rect, RichText, Stroke, Ui, Vec2};

use crate::state::SharedState;
use crate::theme;
use crate::ui::widgets;

pub enum SidebarAction {
    CheckoutBranch(String),
    OpenCreateBranch { from_oid: Option<String> },
    DeleteBranch(String),
    CreateTag(String),
    DeleteTag(String),
    StashApply(usize),
    StashPop(usize),
    StashDrop(usize),
}

pub struct Sidebar {
    state: SharedState,
    open: HashMap<&'static str, bool>,
}

impl Sidebar {
    pub fn new(state: SharedState) -> Self {
        let mut open = HashMap::new();
        open.insert("LOCAL", true);
        open.insert("REMOTE", false);
        open.insert("TAGS", false);
        open.insert("STASHES", false);
        Self { state, open }
    }

    pub fn show(&mut self, ui: &mut Ui) -> Option<SidebarAction> {
        ui.painter().rect_filled(ui.max_rect(), 0.0, theme::BG_PANEL);

        // Right border
        let r = ui.max_rect();
        ui.painter().line_segment(
            [r.right_top(), r.right_bottom()],
            Stroke::new(1.0, theme::BORDER),
        );

        let (graph_opt, head, stashes, ab_map) = {
            let s = self.state.lock().unwrap();
            (s.commit_graph.clone(), s.head_branch.clone().unwrap_or_default(),
             s.stashes.clone(), s.branch_ahead_behind.clone())
        };

        let mut action: Option<SidebarAction> = None;

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.add_space(8.0);

                let Some(ref graph) = graph_opt else {
                    ui.add_space(20.0);
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        ui.label(RichText::new("Open a repository").color(theme::TEXT_MUTED).size(12.0));
                    });
                    return;
                };

                // Split refs into categories
                let mut local:  Vec<&str> = Vec::new();
                let mut remote: Vec<&str> = Vec::new();
                let mut tags:   Vec<&str> = Vec::new();

                let mut all_refs: Vec<&str> = graph.nodes.iter()
                    .flat_map(|n| n.refs.iter().map(|r| r.as_str()))
                    .collect();
                all_refs.sort_unstable();
                all_refs.dedup();

                for r in &all_refs {
                    if r.contains('/') { remote.push(r); }
                    else if r.starts_with('v') && r.chars().nth(1).map_or(false, |c| c.is_ascii_digit()) { tags.push(r); }
                    else { local.push(r); }
                }

                // ── LOCAL BRANCHES ────────────────────────────────────
                {
                    let open = self.open.entry("LOCAL").or_insert(true);
                    widgets::section_header(ui, "LOCAL BRANCHES", local.len(), open);
                    if *open {
                        ui.add_space(2.0);
                        for name in &local {
                            let is_head = *name == head.as_str();
                            let ab = ab_map.get(*name).copied();
                            let resp = branch_row(ui, name, is_head, false, ab);
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
                                    if ui.add(egui::Button::new(
                                        RichText::new("Delete Branch").color(theme::ACCENT_RED),
                                    )).clicked() {
                                        action = Some(SidebarAction::DeleteBranch(name.to_string()));
                                        ui.close_menu();
                                    }
                                }
                            });
                        }
                        if local.is_empty() { empty_hint(ui, "No local branches"); }
                        ui.add_space(4.0);
                    }
                }

                // ── REMOTE BRANCHES ───────────────────────────────────
                {
                    let open = self.open.entry("REMOTE").or_insert(false);
                    widgets::section_header(ui, "REMOTE BRANCHES", remote.len(), open);
                    if *open {
                        ui.add_space(2.0);
                        for name in &remote {
                            branch_row(ui, name, false, true, None);
                        }
                        if remote.is_empty() { empty_hint(ui, "No remote branches"); }
                        ui.add_space(4.0);
                    }
                }

                // ── TAGS ──────────────────────────────────────────────
                {
                    let open = self.open.entry("TAGS").or_insert(false);
                    widgets::section_header(ui, "TAGS", tags.len(), open);
                    if *open {
                        ui.add_space(2.0);
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
                        // Create tag button
                        ui.horizontal(|ui| {
                            ui.add_space(14.0);
                            if ui.add(egui::Button::new(
                                RichText::new("+ Create Tag").color(theme::TEXT_MUTED).size(11.5),
                            ).fill(Color32::TRANSPARENT).stroke(Stroke::NONE)).clicked() {
                                action = Some(SidebarAction::CreateTag(String::new()));
                            }
                        });
                        if tags.is_empty() { empty_hint(ui, "No tags"); }
                        ui.add_space(4.0);
                    }
                }

                // ── STASHES ───────────────────────────────────────────
                {
                    let open = self.open.entry("STASHES").or_insert(false);
                    widgets::section_header(ui, "STASHES", stashes.len(), open);
                    if *open {
                        ui.add_space(2.0);
                        for stash in &stashes {
                            let idx = stash.index;
                            let display = stash.message.trim_start_matches("On ").to_string();
                            let resp = ui.horizontal(|ui| {
                                ui.add_space(14.0);
                                let (dr, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                                ui.painter().circle_filled(dr.center(), 3.5, theme::ACCENT_YELLOW);
                                ui.add_space(6.0);
                                ui.label(RichText::new(&display).color(theme::TEXT_SECONDARY).size(12.0));
                            }).response;
                            resp.context_menu(|ui| {
                                ui.set_min_width(110.0);
                                if ui.button("Apply").clicked()  { action = Some(SidebarAction::StashApply(idx)); ui.close_menu(); }
                                if ui.button("Pop").clicked()    { action = Some(SidebarAction::StashPop(idx)); ui.close_menu(); }
                                ui.separator();
                                if ui.add(egui::Button::new(RichText::new("Drop").color(theme::ACCENT_RED))).clicked() {
                                    action = Some(SidebarAction::StashDrop(idx)); ui.close_menu();
                                }
                            });
                        }
                        if stashes.is_empty() { empty_hint(ui, "No stashes"); }
                        ui.add_space(4.0);
                    }
                }
            });

        action
    }
}

// ── Branch row ────────────────────────────────────────────────────────────────

fn branch_row(ui: &mut Ui, name: &str, is_head: bool, is_remote: bool, ahead_behind: Option<(usize, usize)>) -> egui::Response {
    let row_h = 26.0;
    let avail = ui.available_width();
    let (row_rect, resp) = ui.allocate_exact_size(Vec2::new(avail, row_h), egui::Sense::click());

    let hovered = resp.hovered();
    let bg = if hovered { theme::BG_HOVER } else { Color32::TRANSPARENT };
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(row_rect, 0.0, bg);
    }

    // Teal left accent bar for current branch
    if is_head {
        ui.painter().rect_filled(
            Rect::from_min_size(row_rect.left_top(), Vec2::new(3.0, row_h)),
            2.0,
            theme::ACCENT_TEAL,
        );
    }

    // Branch icon dot
    let dot_x = row_rect.left() + 16.0;
    let dot_color = if is_head { theme::ACCENT_TEAL }
        else if is_remote { theme::ACCENT_BLUE.gamma_multiply(0.7) }
        else { theme::TEXT_MUTED };
    ui.painter().circle_filled(Pos2::new(dot_x, row_rect.center().y), 3.5, dot_color);

    // Branch name
    let text_color = if is_head { theme::TEXT_PRIMARY } else { theme::TEXT_SECONDARY };
    let text_style = if is_head {
        RichText::new(name).color(text_color).size(12.0).strong()
    } else {
        RichText::new(name).color(text_color).size(12.0)
    };

    // Ahead/behind counts on the right
    let ab_text = match ahead_behind {
        Some((a, b)) if a > 0 && b > 0 => format!("↑{} ↓{}", a, b),
        Some((a, 0)) if a > 0           => format!("↑{}", a),
        Some((0, b)) if b > 0           => format!("↓{}", b),
        _                               => String::new(),
    };

    let right_margin = if ab_text.is_empty() { 8.0 } else { 4.0 };
    let ab_width = if ab_text.is_empty() { 0.0 } else { 52.0 };

    let text_rect = Rect::from_min_max(
        Pos2::new(dot_x + 12.0, row_rect.min.y),
        Pos2::new(row_rect.max.x - ab_width - right_margin, row_rect.max.y),
    );
    ui.allocate_ui_at_rect(text_rect, |ui| {
        ui.horizontal_centered(|ui| {
            ui.label(text_style);
        });
    });

    if !ab_text.is_empty() {
        let ab_rect = Rect::from_min_max(
            Pos2::new(row_rect.max.x - ab_width - 4.0, row_rect.min.y),
            Pos2::new(row_rect.max.x - 4.0, row_rect.max.y),
        );
        ui.allocate_ui_at_rect(ab_rect, |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(RichText::new(&ab_text).color(theme::TEXT_MUTED).size(10.5));
            });
        });
    }

    resp
}

fn tag_row(ui: &mut Ui, name: &str) -> egui::Response {
    let row_h = 24.0;
    let avail = ui.available_width();
    let (row_rect, resp) = ui.allocate_exact_size(Vec2::new(avail, row_h), egui::Sense::click());

    if resp.hovered() {
        ui.painter().rect_filled(row_rect, 0.0, theme::BG_HOVER);
    }

    let cx = row_rect.left() + 18.0;
    let cy = row_rect.center().y;
    let s = 4.0_f32;
    // Diamond icon
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            Pos2::new(cx, cy - s), Pos2::new(cx + s, cy),
            Pos2::new(cx, cy + s), Pos2::new(cx - s, cy),
        ],
        theme::ACCENT_PURPLE,
        Stroke::NONE,
    ));

    let text_rect = Rect::from_min_max(
        Pos2::new(cx + 12.0, row_rect.min.y),
        Pos2::new(row_rect.max.x - 8.0, row_rect.max.y),
    );
    ui.allocate_ui_at_rect(text_rect, |ui| {
        ui.horizontal_centered(|ui| {
            ui.label(RichText::new(name).color(theme::TEXT_SECONDARY).size(12.0));
        });
    });

    resp
}

fn empty_hint(ui: &mut Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(20.0);
        ui.label(RichText::new(text).color(theme::TEXT_MUTED).size(11.0));
    });
}
