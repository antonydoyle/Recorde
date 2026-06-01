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
    MergeBranch(String),
    RebaseOnto(String),
    RenameBranch(String),
    PublishBranch(String),
    SetUpstream(String),
    ForceDeleteBranch(String),
    PushTag(String),
    DeleteRemoteTag(String),
    PreviewStash(usize, String),   // (index, oid)
    StashBranch(usize),            // index
    RestoreBranch(String, String), // (name, oid)
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
        open.insert("TAGS", true);
        open.insert("STASHES", false);
        open.insert("WORKTREES", true);
        Self { state, open }
    }

    pub fn show(&mut self, ui: &mut Ui) -> Option<SidebarAction> {
        ui.painter().rect_filled(ui.max_rect(), 0.0, theme::bg_panel());

        // Right border
        let r = ui.max_rect();
        ui.painter().line_segment(
            [r.right_top(), r.right_bottom()],
            Stroke::new(1.0, theme::border()),
        );

        let (graph_opt, head, stashes, ab_map, merged_set, deleted, worktrees, selected_commit) = {
            let s = self.state.lock().unwrap();
            (s.commit_graph.clone(), s.head_branch.clone().unwrap_or_default(),
             s.stashes.clone(), s.branch_ahead_behind.clone(),
             s.merged_into_head.clone(), s.deleted_branches.clone(),
             s.worktrees.clone(), s.selected_commit.clone())
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
                        ui.label(RichText::new("Open a repository").color(theme::text_muted()).size(12.0));
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
                    if r.contains('/') {
                        remote.push(r);
                    } else if graph.tag_names.contains(*r) {
                        tags.push(r);
                    } else {
                        local.push(r);
                    }
                }

                // ── LOCAL BRANCHES ────────────────────────────────────
                {
                    let open = self.open.entry("LOCAL").or_insert(true);
                    widgets::section_header(ui, "LOCAL BRANCHES", local.len(), open);
                    if *open {
                        ui.add_space(2.0);
                        for name in &local {
                            let is_head = *name == head.as_str();
                            let is_merged = merged_set.contains(*name);
                            let ab = ab_map.get(*name).copied();
                            let resp = branch_row(ui, name, is_head, false, ab, is_merged);
                            resp.context_menu(|ui| {
                                ui.set_min_width(195.0);
                                if !is_head {
                                    if ui.button("Checkout").clicked() {
                                        action = Some(SidebarAction::CheckoutBranch(name.to_string()));
                                        ui.close_menu();
                                    }
                                    ui.separator();
                                }
                                if ui.button("Rename…").clicked() {
                                    action = Some(SidebarAction::RenameBranch(name.to_string()));
                                    ui.close_menu();
                                }
                                if !is_head {
                                    if ui.button("Publish Branch").clicked() {
                                        action = Some(SidebarAction::PublishBranch(name.to_string()));
                                        ui.close_menu();
                                    }
                                    if ui.button("Set Upstream…").clicked() {
                                        action = Some(SidebarAction::SetUpstream(name.to_string()));
                                        ui.close_menu();
                                    }
                                    ui.separator();
                                    if ui.button("Merge into current").clicked() {
                                        action = Some(SidebarAction::MergeBranch(name.to_string()));
                                        ui.close_menu();
                                    }
                                    if ui.button("Rebase onto").clicked() {
                                        action = Some(SidebarAction::RebaseOnto(name.to_string()));
                                        ui.close_menu();
                                    }
                                }
                                if !is_head {
                                    ui.separator();
                                    if ui.button("Create Branch From Here").clicked() {
                                        action = Some(SidebarAction::OpenCreateBranch { from_oid: None });
                                        ui.close_menu();
                                    }
                                    ui.separator();
                                    if ui.add(egui::Button::new(
                                        RichText::new("Force Delete").color(theme::ACCENT_RED),
                                    )).clicked() {
                                        action = Some(SidebarAction::ForceDeleteBranch(name.to_string()));
                                        ui.close_menu();
                                    }
                                    if ui.add(egui::Button::new(
                                        RichText::new("Delete Branch").color(theme::ACCENT_RED),
                                    )).clicked() {
                                        action = Some(SidebarAction::DeleteBranch(name.to_string()));
                                        ui.close_menu();
                                    }
                                } else {
                                    ui.separator();
                                    if ui.button("Create Branch From Here").clicked() {
                                        action = Some(SidebarAction::OpenCreateBranch { from_oid: None });
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
                            branch_row(ui, name, false, true, None, false);
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
                                ui.set_min_width(160.0);
                                if ui.button("Push to Remote").clicked() {
                                    action = Some(SidebarAction::PushTag(name.to_string()));
                                    ui.close_menu();
                                }
                                if ui.button("Delete from Remote").clicked() {
                                    action = Some(SidebarAction::DeleteRemoteTag(name.to_string()));
                                    ui.close_menu();
                                }
                                ui.separator();
                                if ui.add(egui::Button::new(
                                    RichText::new("Delete Tag").color(theme::ACCENT_RED),
                                )).clicked() {
                                    action = Some(SidebarAction::DeleteTag(name.to_string()));
                                    ui.close_menu();
                                }
                            });
                        }
                        // Create tag button — tag the selected commit if one is active
                        ui.horizontal(|ui| {
                            ui.add_space(14.0);
                            let label = if selected_commit.is_some() {
                                RichText::new("+ Tag Selected Commit").color(theme::ACCENT_TEAL).size(11.5)
                            } else {
                                RichText::new("+ Create Tag at HEAD").color(theme::text_muted()).size(11.5)
                            };
                            if ui.add(egui::Button::new(label)
                                .fill(Color32::TRANSPARENT).stroke(Stroke::NONE)).clicked()
                            {
                                action = Some(SidebarAction::CreateTag(
                                    selected_commit.clone().unwrap_or_default()
                                ));
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
                            let oid = stash.oid.clone();
                            let display = stash.message
                                .trim_start_matches("On ")
                                .trim_start_matches("WIP on ")
                                .to_string();
                            let resp = ui.horizontal(|ui| {
                                ui.add_space(14.0);
                                let (dr, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                                ui.painter().circle_filled(dr.center(), 3.5, theme::ACCENT_YELLOW);
                                ui.add_space(6.0);
                                ui.label(RichText::new(&display).color(theme::text_secondary()).size(12.0));
                            }).response;
                            resp.context_menu(|ui| {
                                ui.set_min_width(155.0);
                                if ui.button("Preview Diff").clicked() {
                                    action = Some(SidebarAction::PreviewStash(idx, oid.clone())); ui.close_menu();
                                }
                                ui.separator();
                                if ui.button("Apply").clicked()  { action = Some(SidebarAction::StashApply(idx)); ui.close_menu(); }
                                if ui.button("Pop").clicked()    { action = Some(SidebarAction::StashPop(idx)); ui.close_menu(); }
                                if ui.button("Create Branch Here…").clicked() {
                                    action = Some(SidebarAction::StashBranch(idx)); ui.close_menu();
                                }
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

                // ── WORKTREES ─────────────────────────────────────────
                // Only show when there are linked worktrees beyond the main one
                if worktrees.len() > 1 {
                    let linked: Vec<_> = worktrees.iter().filter(|w| !w.is_main).collect();
                    let open = self.open.entry("WORKTREES").or_insert(true);
                    widgets::section_header(ui, "WORKTREES", linked.len(), open);
                    if *open {
                        ui.add_space(2.0);
                        for wt in &linked {
                            let label = wt.branch.as_deref()
                                .unwrap_or_else(|| &wt.head_oid[..wt.head_oid.len().min(8)]);
                            ui.horizontal(|ui| {
                                ui.add_space(14.0);
                                let (dr, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                                ui.painter().circle_filled(dr.center(), 3.5, theme::ACCENT_TEAL);
                                ui.add_space(6.0);
                                ui.label(RichText::new(label).color(theme::text_secondary()).size(12.0));
                            });
                        }
                        ui.add_space(4.0);
                    }
                }

                // ── RECENTLY DELETED ──────────────────────────────────
                if !deleted.is_empty() {
                    let open = self.open.entry("DELETED").or_insert(true);
                    widgets::section_header(ui, "RECENTLY DELETED", deleted.len(), open);
                    if *open {
                        ui.add_space(2.0);
                        for entry in deleted.iter().rev() {
                            let resp = ui.horizontal(|ui| {
                                ui.add_space(14.0);
                                let (dr, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                                ui.painter().circle_filled(dr.center(), 3.5, theme::ACCENT_RED);
                                ui.add_space(6.0);
                                ui.label(RichText::new(&entry.name).color(theme::text_secondary()).size(12.0).strikethrough());
                            }).response;
                            resp.context_menu(|ui| {
                                ui.set_min_width(155.0);
                                if ui.button("Restore Branch").clicked() {
                                    action = Some(SidebarAction::RestoreBranch(entry.name.clone(), entry.oid.clone()));
                                    ui.close_menu();
                                }
                            });
                        }
                        ui.add_space(4.0);
                    }
                }
            });

        action
    }
}

// ── Branch row ────────────────────────────────────────────────────────────────

fn branch_row(ui: &mut Ui, name: &str, is_head: bool, is_remote: bool, ahead_behind: Option<(usize, usize)>, is_merged: bool) -> egui::Response {
    let row_h = 26.0;
    let avail = ui.available_width();
    let (row_rect, resp) = ui.allocate_exact_size(Vec2::new(avail, row_h), egui::Sense::click());

    let hovered = resp.hovered();
    let bg = if hovered { theme::bg_hover() } else { Color32::TRANSPARENT };
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
        else { theme::text_muted() };
    ui.painter().circle_filled(Pos2::new(dot_x, row_rect.center().y), 3.5, dot_color);

    // Branch name — greyed out if fully merged into HEAD
    let text_color = if is_head { theme::text_primary() }
        else if is_merged { theme::text_muted() }
        else { theme::text_secondary() };
    let text_style = if is_head {
        RichText::new(name).color(text_color).size(12.0).strong()
    } else {
        RichText::new(name).color(text_color).size(12.0)
    };

    // Right-side indicator: "merged" badge or ahead/behind counts
    let compact_merged = is_merged && !is_head && avail < 245.0;
    let (ab_text, ab_color) = if is_merged && !is_head {
        (if compact_merged { "M" } else { "merged" }.to_string(), theme::text_muted())
    } else {
        let txt = match ahead_behind {
            Some((a, b)) if a > 0 && b > 0 => format!("↑{} ↓{}", a, b),
            Some((a, 0)) if a > 0           => format!("↑{}", a),
            Some((0, b)) if b > 0           => format!("↓{}", b),
            _                               => String::new(),
        };
        (txt, theme::text_muted())
    };

    let right_margin = if ab_text.is_empty() { 8.0 } else { 4.0 };
    let ab_width = if ab_text.is_empty() { 0.0 } else if compact_merged { 18.0 } else if is_merged { 46.0 } else { 52.0 };

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
        let ab_id = ui.make_persistent_id(("branch-status", name));
        let ab_resp = ui.interact(ab_rect, ab_id, egui::Sense::hover());
        if compact_merged {
            ab_resp.on_hover_text("Merged into current branch");
        }
        ui.allocate_ui_at_rect(ab_rect, |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(RichText::new(&ab_text).color(ab_color).size(10.5));
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
        ui.painter().rect_filled(row_rect, 0.0, theme::bg_hover());
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
            ui.label(RichText::new(name).color(theme::text_secondary()).size(12.0));
        });
    });

    resp
}

fn empty_hint(ui: &mut Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(20.0);
        ui.label(RichText::new(text).color(theme::text_muted()).size(11.0));
    });
}
