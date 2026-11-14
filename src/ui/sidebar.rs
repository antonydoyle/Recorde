use crate::ui::button::Button;
use std::collections::HashMap;

use egui::{Color32, Pos2, Rect, RichText, Stroke, Ui, Vec2};

use crate::state::SharedState;
use crate::theme;
use crate::ui::{kit, motion, widgets};

pub enum SidebarAction {
    OpenRepository(std::path::PathBuf),
    CheckoutBranch(String),
    OpenCreateBranch {
        from_oid: Option<String>,
    },
    DeleteBranch(String),
    CreateTag(String),
    DeleteTag(String),
    StashApply(usize, String),
    StashPop(usize, String),
    StashDrop(usize, String),
    MergeBranch(String),
    RebaseOnto(String),
    RenameBranch(String),
    PublishBranch(String),
    SetUpstream(String, String),
    ForceDeleteBranch(String),
    PushTag(String),
    DeleteRemoteTag(String),
    PreviewStash(usize, String),   // (index, oid)
    StashBranch(usize, String),    // (index, oid)
    RestoreBranch(String, String), // (name, oid)
    /// Remote-tracking shorthand such as `origin/feature`.
    CheckoutRemoteBranch(String),
    DeleteRemoteBranch(String),
    /// Select and scroll to a ref's commit in the graph.
    RevealCommit(String),
    /// Open the merge dialog with this branch chosen.
    MergeWithOptions(String),
    /// Move a branch that is not checked out to its upstream.
    FastForward(String),
}

pub struct Sidebar {
    state: SharedState,
    open: HashMap<&'static str, bool>,
    /// Case-insensitive branch and tag filter. Matching sections expand.
    filter: String,
    /// Mirrors the graph's colour-blind lane palette for branch dots.
    colorblind: bool,
}

impl Sidebar {
    pub fn new(state: SharedState) -> Self {
        let mut open = HashMap::new();
        open.insert("LOCAL", true);
        open.insert("REMOTE", false);
        open.insert("TAGS", true);
        open.insert("STASHES", false);
        open.insert("WORKTREES", true);
        Self {
            state,
            open,
            filter: String::new(),
            colorblind: false,
        }
    }

    pub fn set_colorblind(&mut self, enabled: bool) {
        self.colorblind = enabled;
    }

    pub fn show(&mut self, ui: &mut Ui) -> Option<SidebarAction> {
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, theme::bg_panel());

        // Right border
        let r = ui.max_rect();
        ui.painter().line_segment(
            [r.right_top(), r.right_bottom()],
            Stroke::new(1.0, theme::border()),
        );

        let repo_path = self.state.lock().unwrap().repo_path.clone();
        let (graph_opt, head, stashes, ab_map, merged_set, deleted, worktrees, selected_commit) = {
            let s = self.state.lock().unwrap();
            (
                s.commit_graph.clone(),
                s.head_branch.clone().unwrap_or_default(),
                s.stashes.clone(),
                s.branch_ahead_behind.clone(),
                s.merged_into_head.clone(),
                s.deleted_branches.clone(),
                s.worktrees.clone(),
                s.selected_commit.clone(),
            )
        };

        let mut action: Option<SidebarAction> = None;
        let sidebar_width = ui.available_width();

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.set_width(ui.available_width().min(sidebar_width));
                // No "Repository" heading: the tab, toolbar and status bar already
                // name the repository, so the branch filter leads the sidebar.
                ui.add_space(kit::space::MD);
                let saved_count={let s=self.state.lock().unwrap();s.saved_work.len()+s.saved_work_warnings.len()};
                if saved_count>0 && ui.button(format!("Saved work ({saved_count})")).clicked() {
                    self.state.lock().unwrap().show_saved_work=true;
                }
                let Some(ref graph) = graph_opt else {
                    ui.add_space(20.0);
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        ui.label(
                            RichText::new("Open a repository")
                                .color(theme::text_muted())
                                .size(13.0),
                        );
                    });
                    return;
                };

                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(
                        crate::icons::rich(crate::icons::SEARCH, 16.0).color(theme::text_muted()),
                    );
                    let clear_width = if self.filter.is_empty() {
                        0.0
                    } else {
                        32.0 + ui.spacing().item_spacing.x
                    };
                    let field_width =
                        (ui.available_width() - clear_width - kit::space::SM).max(1.0);
                    // The search icon says "filter"; a short hint fits the default
                    // sidebar width without truncating, and the tooltip names both.
                    ui.add_sized(
                        [field_width, 28.0],
                        crate::ui::kit::compact_text_field(&mut self.filter)
                            .hint_text("Branches and tags")
                            .desired_width(field_width),
                    )
                    .on_hover_text("Filter branches and tags by name");
                    if !self.filter.is_empty()
                        && widgets::icon_btn_mat(ui, crate::icons::CLOSE, 16.0, "Clear filter")
                            .clicked()
                    {
                        self.filter.clear();
                    }
                });
                ui.add_space(8.0);
                let needle = self.filter.trim().to_lowercase();
                let filtering = !needle.is_empty();
                let shown = |name: &&str| !filtering || name.to_lowercase().contains(&needle);
                let none = |empty: &'static str| if filtering { "No matches" } else { empty };

                // Split refs into categories. Upstream choices always list every remote branch.
                let mut local: Vec<&str> = graph.local_names.iter().map(String::as_str).collect();
                let mut remote: Vec<&str> = graph.remote_names.iter().map(String::as_str).collect();
                let mut tags: Vec<&str> = graph.tag_names.iter().map(String::as_str).collect();
                local.sort_unstable();
                remote.sort_unstable();
                tags.sort_unstable();
                local.retain(shown);
                tags.retain(shown);
                let remote_rows: Vec<&str> = remote.iter().copied().filter(shown).collect();
                let stash_rows: Vec<_> = stashes
                    .iter()
                    .filter(|stash| !filtering || stash.message.to_lowercase().contains(&needle))
                    .collect();

                // The current branch's dot uses its graph lane colour, so a branch
                // has the same colour here as on its graph chip.
                let head_color = graph
                    .ref_targets
                    .get(&format!("refs/heads/{head}"))
                    .and_then(|oid| graph.oid_to_row.get(oid))
                    .map(|&row| {
                        let order = theme::graph_color_order(repo_path.as_deref());
                        theme::lane_color_for_palette(
                            order[graph.nodes[row].lane % order.len()],
                            self.colorblind,
                        )
                    });

                // ── LOCAL BRANCHES ────────────────────────────────────
                {
                    let open = self.open.entry("LOCAL").or_insert(true);
                    widgets::section_header(ui, "Local branches", local.len(), open);
                    motion::reveal(ui, egui::Id::new("sidebar_local"), *open || filtering, |ui| {
                        ui.add_space(2.0);
                        for name in &local {
                            let is_head = *name == head.as_str();
                            let is_merged = merged_set.contains(*name);
                            let ab = ab_map.get(*name).copied();
                            let identity = if is_head { head_color } else { None };
                            let resp = branch_row(ui, name, is_head, false, ab, is_merged, identity)
                                .on_hover_text(
                                    "Click to show in graph, double-click to check out, right-click for options",
                                );
                            let tip = graph.ref_targets.get(&format!("refs/heads/{name}")).cloned();
                            if resp.double_clicked() && !is_head {
                                action = Some(SidebarAction::CheckoutBranch(name.to_string()));
                            } else if resp.clicked() {
                                if let Some(tip) = tip.clone() {
                                    action = Some(SidebarAction::RevealCommit(tip));
                                }
                            }
                            let selected = is_head
                                || (tip.is_some() && tip.as_ref() == selected_commit.as_ref());
                            widgets::row_actions_menu(ui, &resp, name, selected, |ui| {
                                ui.set_min_width(195.0);
                                if !is_head {
                                    if ui.button("Checkout").clicked() {
                                        action =
                                            Some(SidebarAction::CheckoutBranch(name.to_string()));
                                        ui.close();
                                    }
                                    ui.separator();
                                }
                                if ui.button("Rename…").clicked() {
                                    action = Some(SidebarAction::RenameBranch(name.to_string()));
                                    ui.close();
                                }
                                if ui.button("Publish Branch").clicked() {
                                    action = Some(SidebarAction::PublishBranch(name.to_string()));
                                    ui.close();
                                }
                                ui.menu_button("Set Upstream", |ui| {
                                    if remote.is_empty() {
                                        ui.label("Fetch remote branches first.");
                                    }
                                    for target in &remote {
                                        if ui.button(*target).clicked() {
                                            action = Some(SidebarAction::SetUpstream(
                                                name.to_string(),
                                                target.to_string(),
                                            ));
                                            ui.close();
                                        }
                                    }
                                });
                                if !is_head && ab.is_some_and(|(ahead, behind)| ahead == 0 && behind > 0) {
                                    if ui
                                        .button("Fast-forward to upstream")
                                        .on_hover_text("Move this branch to its upstream without checking it out")
                                        .clicked()
                                    {
                                        action = Some(SidebarAction::FastForward(name.to_string()));
                                        ui.close();
                                    }
                                }
                                if !is_head {
                                    ui.separator();
                                    if ui
                                        .button("Merge into current")
                                        .on_hover_text(format!("Merge {name} into {head}."))
                                        .clicked()
                                    {
                                        action = Some(SidebarAction::MergeBranch(name.to_string()));
                                        ui.close();
                                    }
                                    if ui.button("Merge with options…").clicked() {
                                        action = Some(SidebarAction::MergeWithOptions(name.to_string()));
                                        ui.close();
                                    }
                                    if ui
                                        .button("Rebase onto")
                                        .on_hover_text(format!(
                                            "Rebase {head} onto {name} — \
                                             replays your commits on top of it."
                                        ))
                                        .clicked()
                                    {
                                        action = Some(SidebarAction::RebaseOnto(name.to_string()));
                                        ui.close();
                                    }
                                }
                                if !is_head {
                                    ui.separator();
                                    if ui.button("Create Branch From Here").clicked() {
                                        action = Some(SidebarAction::OpenCreateBranch {
                                            from_oid: tip.clone(),
                                        });
                                        ui.close();
                                    }
                                    ui.separator();
                                    if ui
                                        .add(Button::new(
                                            RichText::new("Force Delete")
                                                .color(theme::accent_red()),
                                        ))
                                        .clicked()
                                    {
                                        action = Some(SidebarAction::ForceDeleteBranch(
                                            name.to_string(),
                                        ));
                                        ui.close();
                                    }
                                    if ui
                                        .add(Button::new(
                                            RichText::new("Delete Branch")
                                                .color(theme::accent_red()),
                                        ))
                                        .clicked()
                                    {
                                        action =
                                            Some(SidebarAction::DeleteBranch(name.to_string()));
                                        ui.close();
                                    }
                                } else {
                                    ui.separator();
                                    if ui.button("Create Branch From Here").clicked() {
                                        action = Some(SidebarAction::OpenCreateBranch {
                                            from_oid: None,
                                        });
                                        ui.close();
                                    }
                                }
                            });
                        }
                        if local.is_empty() {
                            empty_hint(
                                ui,
                                none("No local branches yet. Create a branch to get started."),
                            );
                        }
                        ui.add_space(4.0);
                    });
                }

                // ── REMOTE BRANCHES ───────────────────────────────────
                {
                    let open = self.open.entry("REMOTE").or_insert(false);
                    widgets::section_header(ui, "Remotes", remote_rows.len(), open);
                    motion::reveal(ui, egui::Id::new("sidebar_remotes"), *open || filtering, |ui| {
                        ui.add_space(2.0);
                        for (remote_name, branches) in group_remote_branches(&remote_rows, &graph.configured_remotes) {
                            egui::CollapsingHeader::new(format!("{remote_name} ({})", branches.len()))
                                .id_salt(("remote_group", &remote_name, filtering))
                                .default_open(true)
                                .open(if filtering { Some(true) } else { None })
                                .show(ui, |ui| {
                        for name in &branches {
                            let resp = branch_row(ui, name, false, true, None, false, None)
                                .on_hover_text(
                                    "Click to show in graph, double-click to check out, right-click for options",
                                );
                            let tip = graph.ref_targets.get(&format!("refs/remotes/{name}")).cloned();
                            if resp.double_clicked() {
                                action = Some(SidebarAction::CheckoutRemoteBranch(name.to_string()));
                            } else if resp.clicked() {
                                if let Some(tip) = tip.clone() {
                                    action = Some(SidebarAction::RevealCommit(tip));
                                }
                            }
                            let selected =
                                tip.is_some() && tip.as_ref() == selected_commit.as_ref();
                            widgets::row_actions_menu(ui, &resp, name, selected, |ui| {
                                ui.set_min_width(195.0);
                                if ui.button("Checkout").on_hover_text("Create or switch to a local branch that tracks this branch").clicked() {
                                    action = Some(SidebarAction::CheckoutRemoteBranch(name.to_string()));
                                    ui.close();
                                }
                                ui.separator();
                                if ui
                                    .button("Merge into current")
                                    .on_hover_text(format!("Merge {name} into {head}."))
                                    .clicked()
                                {
                                    action = Some(SidebarAction::MergeBranch(name.to_string()));
                                    ui.close();
                                }
                                if ui.button("Merge with options…").clicked() {
                                    action = Some(SidebarAction::MergeWithOptions(name.to_string()));
                                    ui.close();
                                }
                                if ui
                                    .button("Rebase onto")
                                    .on_hover_text(format!(
                                        "Rebase {head} onto {name} — \
                                         replays your commits on top of it."
                                    ))
                                    .clicked()
                                {
                                    action = Some(SidebarAction::RebaseOnto(name.to_string()));
                                    ui.close();
                                }
                                ui.separator();
                                if ui.button("Create Branch From Here").clicked() {
                                    action = Some(SidebarAction::OpenCreateBranch { from_oid: tip.clone() });
                                    ui.close();
                                }
                                if ui.button("Copy Name").clicked() {
                                    ui.ctx().copy_text(name.to_string());
                                    ui.close();
                                }
                                ui.separator();
                                if ui
                                    .add(Button::new(
                                        RichText::new("Delete from Remote").color(theme::accent_red()),
                                    ))
                                    .clicked()
                                {
                                    action = Some(SidebarAction::DeleteRemoteBranch(name.to_string()));
                                    ui.close();
                                }
                            });
                        }
                                });
                        }
                        if remote_rows.is_empty() {
                            empty_hint(
                                ui,
                                none("No remote branches yet. Fetch to check for branches."),
                            );
                        }
                        ui.add_space(4.0);
                    });
                }

                // ── TAGS ──────────────────────────────────────────────
                {
                    let open = self.open.entry("TAGS").or_insert(false);
                    let (label, tooltip) = if selected_commit.is_some() {
                        ("+ Tag Selected Commit", "Tag the selected commit")
                    } else {
                        ("+ Create Tag at HEAD", "Create a tag at HEAD")
                    };
                    let create_tag = ui
                        .horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            let header_width =
                                (ui.available_width() - 32.0 - kit::space::SM).max(1.0);
                            ui.allocate_ui_with_layout(
                                Vec2::new(header_width, 34.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.set_width(header_width);
                                    widgets::section_header(ui, "Tags", tags.len(), open);
                                },
                            );
                            let button =
                                widgets::icon_btn_mat(ui, crate::icons::ADD, 16.0, tooltip);
                            button.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    ui.is_enabled(),
                                    label,
                                )
                            });
                            button.clicked()
                        })
                        .inner;
                    if create_tag {
                        action = Some(SidebarAction::CreateTag(
                            selected_commit.clone().unwrap_or_default(),
                        ));
                    }
                    motion::reveal(ui, egui::Id::new("sidebar_tags"), *open || filtering, |ui| {
                        ui.add_space(2.0);
                        for name in &tags {
                            let resp = tag_row(ui, name)
                                .on_hover_text("Click to show in graph, right-click for options");
                            if resp.clicked() {
                                if let Some(tip) = graph.ref_targets.get(&format!("refs/tags/{name}")) {
                                    action = Some(SidebarAction::RevealCommit(tip.clone()));
                                }
                            }
                            let selected = selected_commit.as_ref().is_some_and(|oid| {
                                graph.ref_targets.get(&format!("refs/tags/{name}")) == Some(oid)
                            });
                            widgets::row_actions_menu(ui, &resp, name, selected, |ui| {
                                ui.set_min_width(160.0);
                                if ui.button("Push to Remote").clicked() {
                                    action = Some(SidebarAction::PushTag(name.to_string()));
                                    ui.close();
                                }
                                if ui.button("Delete from Remote").clicked() {
                                    action = Some(SidebarAction::DeleteRemoteTag(name.to_string()));
                                    ui.close();
                                }
                                ui.separator();
                                if ui
                                    .add(Button::new(
                                        RichText::new("Delete Tag").color(theme::accent_red()),
                                    ))
                                    .clicked()
                                {
                                    action = Some(SidebarAction::DeleteTag(name.to_string()));
                                    ui.close();
                                }
                            });
                        }
                        if tags.is_empty() {
                            empty_hint(ui, none("No tags yet. Create a tag to mark a commit."));
                        }
                        ui.add_space(4.0);
                    });
                }

                // ── STASHES ───────────────────────────────────────────
                {
                    let open = self.open.entry("STASHES").or_insert(false);
                    widgets::section_header(ui, "Stashes", stash_rows.len(), open);
                    motion::reveal(ui, egui::Id::new("sidebar_stashes"), *open || filtering, |ui| {
                        ui.add_space(2.0);
                        for stash in &stash_rows {
                            let idx = stash.index;
                            let oid = stash.oid.clone();
                            let display = stash
                                .message
                                .trim_start_matches("On ")
                                .trim_start_matches("WIP on ")
                                .to_string();
                            let row_width = (ui.available_width() - kit::space::SM).max(1.0);
                            let resp = ui
                                .horizontal(|ui| {
                                    ui.set_width(row_width);
                                    ui.set_min_height(26.0);
                                    ui.add_space(14.0);
                                    let (dr, _) = ui.allocate_exact_size(
                                        Vec2::splat(8.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().circle_filled(
                                        dr.center(),
                                        3.5,
                                        theme::accent_yellow(),
                                    );
                                    ui.add_space(6.0);
                                    let width = (ui.available_width() - widgets::ROW_ACTIONS_WIDTH)
                                        .max(1.0);
                                    ui.add_sized(
                                        [width, 22.0],
                                        egui::Label::new(
                                            RichText::new(&display)
                                                .color(theme::text_secondary())
                                                .size(13.0),
                                        )
                                        .truncate(),
                                    );
                                })
                                .response
                                .on_hover_text("Right-click for options");
                            let selected = selected_commit.as_deref() == Some(oid.as_str());
                            widgets::row_actions_menu(ui, &resp, &display, selected, |ui| {
                                ui.set_min_width(155.0);
                                if ui.button("Preview Diff").clicked() {
                                    action = Some(SidebarAction::PreviewStash(idx, oid.clone()));
                                    ui.close();
                                }
                                ui.separator();
                                if ui
                                    .button("Apply")
                                    .on_hover_text("Apply the saved changes and keep the stash.")
                                    .clicked()
                                {
                                    action = Some(SidebarAction::StashApply(idx, oid.clone()));
                                    ui.close();
                                }
                                if ui
                                    .button("Pop")
                                    .on_hover_text("Apply the saved changes and delete the stash.")
                                    .clicked()
                                {
                                    action = Some(SidebarAction::StashPop(idx, oid.clone()));
                                    ui.close();
                                }
                                if ui.button("Create Branch Here…").clicked() {
                                    action = Some(SidebarAction::StashBranch(idx, oid.clone()));
                                    ui.close();
                                }
                                ui.separator();
                                if ui
                                    .add(Button::new(
                                        RichText::new("Drop").color(theme::accent_red()),
                                    ))
                                    .clicked()
                                {
                                    action = Some(SidebarAction::StashDrop(idx, oid.clone()));
                                    ui.close();
                                }
                            });
                        }
                        if stash_rows.is_empty() {
                            empty_hint(
                                ui,
                                none("No stashes yet. Stash changes to save them for later."),
                            );
                        }
                        ui.add_space(4.0);
                    });
                }

                // ── WORKTREES ─────────────────────────────────────────
                // Only show when there are linked worktrees beyond the main one
                if worktrees.len() > 1 {
                    let linked: Vec<_> = worktrees.iter().filter(|w| !w.is_main).collect();
                    let open = self.open.entry("WORKTREES").or_insert(true);
                    widgets::section_header(ui, "Worktrees", linked.len(), open);
                    motion::reveal(ui, egui::Id::new("sidebar_worktrees"), *open, |ui| {
                        ui.add_space(2.0);
                        for wt in &linked {
                            let label = wt
                                .branch
                                .as_deref()
                                .unwrap_or_else(|| &wt.head_oid[..wt.head_oid.len().min(8)]);
                            ui.horizontal(|ui| {
                                ui.add_space(14.0);
                                let (dr, _) =
                                    ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                                ui.painter()
                                    .circle_filled(dr.center(), 3.5, theme::accent_teal());
                                ui.add_space(6.0);
                                if ui
                                    .add(
                                        egui::Button::selectable(
                                            false,
                                            RichText::new(label)
                                                .color(theme::text_secondary())
                                                .size(13.0),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(&wt.path)
                                    .clicked()
                                {
                                    action =
                                        Some(SidebarAction::OpenRepository(wt.path.clone().into()));
                                }
                            });
                        }
                        ui.add_space(4.0);
                    });
                }

                // ── RECENTLY DELETED ──────────────────────────────────
                if !deleted.is_empty() {
                    let open = self.open.entry("DELETED").or_insert(true);
                    widgets::section_header(ui, "RECENTLY DELETED", deleted.len(), open);
                    motion::reveal(ui, egui::Id::new("sidebar_deleted"), *open, |ui| {
                        ui.add_space(2.0);
                        for entry in deleted.iter().rev() {
                            let resp = ui
                                .horizontal(|ui| {
                                    ui.add_space(14.0);
                                    let (dr, _) = ui.allocate_exact_size(
                                        Vec2::splat(8.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().circle_filled(
                                        dr.center(),
                                        3.5,
                                        theme::accent_red(),
                                    );
                                    ui.add_space(6.0);
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&entry.name)
                                                .color(theme::text_secondary())
                                                .size(13.0)
                                                .strikethrough(),
                                        )
                                        .truncate(),
                                    );
                                })
                                .response
                                .on_hover_text("Right-click to restore");
                            resp.context_menu(|ui| {
                                ui.set_min_width(155.0);
                                if ui.button("Restore Branch").clicked() {
                                    action = Some(SidebarAction::RestoreBranch(
                                        entry.name.clone(),
                                        entry.oid.clone(),
                                    ));
                                    ui.close();
                                }
                            });
                        }
                        ui.add_space(4.0);
                    });
                }
            });

        action
    }
}

// ── Branch row ────────────────────────────────────────────────────────────────

/// Group with the longest configured prefix: `team/origin/topic` can belong
/// to remote `team/origin`, not necessarily to remote `team`.
fn group_remote_branches<'a>(
    branches: &[&'a str],
    remotes: &[String],
) -> std::collections::BTreeMap<String, Vec<&'a str>> {
    let mut groups = std::collections::BTreeMap::new();
    for branch in branches {
        let remote = remotes
            .iter()
            .filter(|remote| {
                branch
                    .strip_prefix(remote.as_str())
                    .is_some_and(|suffix| suffix.starts_with('/'))
            })
            .max_by_key(|remote| remote.len())
            .map(String::as_str)
            .unwrap_or_else(|| branch.split('/').next().unwrap_or(branch));
        groups
            .entry(remote.to_string())
            .or_insert_with(Vec::new)
            .push(*branch);
    }
    groups
}

/// Graph chips carry fully qualified references, so a branch and a tag sharing
/// a name cannot accidentally invoke each other's operations.
pub fn graph_reference_menu(
    ui: &mut Ui,
    reference: &str,
    oid: &str,
    state: &SharedState,
) -> Option<SidebarAction> {
    let mut action = None;
    ui.set_min_width(210.0);
    ui.label(egui::RichText::new(reference).color(theme::text_muted()));
    ui.separator();
    if let Some(name) = reference.strip_prefix("refs/heads/") {
        let is_head = state.lock().unwrap().head_branch.as_deref() == Some(name);
        if !is_head && ui.button("Checkout").clicked() {
            action = Some(SidebarAction::CheckoutBranch(name.into()));
        }
        if ui.button("Rename…").clicked() {
            action = Some(SidebarAction::RenameBranch(name.into()));
        }
        if ui.button("Publish Branch").clicked() {
            action = Some(SidebarAction::PublishBranch(name.into()));
        }
        if !is_head {
            if ui.button("Merge with options…").clicked() {
                action = Some(SidebarAction::MergeWithOptions(name.into()));
            }
            if ui.button("Rebase onto").clicked() {
                action = Some(SidebarAction::RebaseOnto(name.into()));
            }
            if ui.button("Delete Branch…").clicked() {
                action = Some(SidebarAction::DeleteBranch(name.into()));
            }
        }
    } else if let Some(name) = reference.strip_prefix("refs/remotes/") {
        if ui.button("Checkout tracking branch").clicked() {
            action = Some(SidebarAction::CheckoutRemoteBranch(name.into()));
        }
        if ui.button("Merge with options…").clicked() {
            action = Some(SidebarAction::MergeWithOptions(name.into()));
        }
        if ui.button("Rebase onto").clicked() {
            action = Some(SidebarAction::RebaseOnto(name.into()));
        }
        if ui.button("Delete from Remote…").clicked() {
            action = Some(SidebarAction::DeleteRemoteBranch(name.into()));
        }
    } else if let Some(name) = reference.strip_prefix("refs/tags/") {
        if ui.button("Push to Remote").clicked() {
            action = Some(SidebarAction::PushTag(name.into()));
        }
        if ui.button("Delete Tag…").clicked() {
            action = Some(SidebarAction::DeleteTag(name.into()));
        }
        if ui.button("Delete from Remote…").clicked() {
            action = Some(SidebarAction::DeleteRemoteTag(name.into()));
        }
    }
    if ui.button("Create Branch From Here").clicked() {
        action = Some(SidebarAction::OpenCreateBranch {
            from_oid: Some(oid.into()),
        });
    }
    ui.separator();
    if ui.button("Copy Reference").clicked() {
        ui.ctx().copy_text(reference.to_string());
        ui.close();
    }
    if ui.button("Copy SHA").clicked() {
        ui.ctx().copy_text(oid.to_string());
        ui.close();
    }
    if action.is_some() {
        ui.close();
    }
    action
}

fn branch_row(
    ui: &mut Ui,
    name: &str,
    is_head: bool,
    is_remote: bool,
    ahead_behind: Option<(usize, usize)>,
    is_merged: bool,
    lane_color: Option<Color32>,
) -> egui::Response {
    let row_h = 26.0;
    let avail = (ui.available_width() - kit::space::SM).max(1.0);
    let (row_rect, resp) = ui.allocate_exact_size(Vec2::new(avail, row_h), egui::Sense::click());

    resp.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            is_head,
            format!(
                "{} branch {name}{}",
                if is_remote { "Remote" } else { "Local" },
                if is_head { ", current branch" } else { "" }
            ),
        )
    });
    let bg = motion::mix(
        Color32::TRANSPARENT,
        theme::bg_hover(),
        motion::hover(ui, &resp),
    );
    ui.painter().rect_filled(row_rect, 0.0, bg);

    // Accent bar marks the current branch, like selected navigation elsewhere.
    if is_head {
        ui.painter().rect_filled(
            Rect::from_min_size(row_rect.left_top(), Vec2::new(3.0, row_h)),
            2.0,
            theme::accent_teal(),
        );
    }

    // Branch icon dot
    let dot_x = row_rect.left() + 16.0;
    let dot_color = if let Some(color) = lane_color {
        color
    } else if is_head {
        theme::accent_teal()
    } else if is_remote {
        theme::accent_blue().gamma_multiply(0.7)
    } else {
        theme::text_muted()
    };
    ui.painter()
        .circle_filled(Pos2::new(dot_x, row_rect.center().y), 3.5, dot_color);

    // Branch name — greyed out if fully merged into HEAD
    let text_color = if is_head {
        theme::text_primary()
    } else if is_merged {
        theme::text_muted()
    } else {
        theme::text_secondary()
    };

    // Right-side indicator: "merged" badge or ahead/behind counts
    let compact_merged = is_merged && !is_head && avail < 245.0;
    let (ab_text, ab_color) = if is_merged && !is_head {
        (
            if compact_merged { "M" } else { "merged" }.to_string(),
            theme::text_muted(),
        )
    } else {
        let txt = match ahead_behind {
            Some((a, b)) if a > 0 && b > 0 => format!("↑{} ↓{}", a, b),
            Some((a, 0)) if a > 0 => format!("↑{}", a),
            Some((0, b)) if b > 0 => format!("↓{}", b),
            _ => String::new(),
        };
        (txt, theme::text_muted())
    };

    let right_edge = row_rect.right() - widgets::ROW_ACTIONS_WIDTH;
    let right_margin = if ab_text.is_empty() { 8.0 } else { 4.0 };
    let ab_width = if ab_text.is_empty() {
        0.0
    } else if compact_merged {
        18.0
    } else if is_merged {
        46.0
    } else {
        52.0
    };

    let text_rect = Rect::from_min_max(
        Pos2::new(dot_x + 12.0, row_rect.min.y),
        Pos2::new(right_edge - ab_width - right_margin, row_rect.max.y),
    );
    ui.painter_at(text_rect).text(
        Pos2::new(text_rect.left() + 4.0, text_rect.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(13.0),
        text_color,
    );

    if !ab_text.is_empty() {
        let ab_rect = Rect::from_min_max(
            Pos2::new(right_edge - ab_width - 4.0, row_rect.min.y),
            Pos2::new(right_edge - 4.0, row_rect.max.y),
        );
        let ab_id = ui.make_persistent_id(("branch-status", name));
        let ab_resp = ui.interact(ab_rect, ab_id, egui::Sense::hover());
        if is_merged {
            ab_resp.on_hover_text("Merged into current branch");
        } else if let Some((ahead, behind)) = ahead_behind {
            ab_resp.on_hover_text(format!("{ahead} to push, {behind} to pull"));
        }
        ui.painter_at(ab_rect).text(
            ab_rect.center(),
            egui::Align2::CENTER_CENTER,
            &ab_text,
            egui::FontId::proportional(12.0),
            ab_color,
        );
    }

    resp
}

fn tag_row(ui: &mut Ui, name: &str) -> egui::Response {
    let row_h = 30.0;
    let avail = (ui.available_width() - kit::space::SM).max(1.0);
    let (row_rect, resp) = ui.allocate_exact_size(Vec2::new(avail, row_h), egui::Sense::click());

    resp.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            format!("Tag {name}"),
        )
    });
    if resp.hovered() || resp.has_focus() {
        ui.painter().rect_filled(row_rect, 0.0, theme::bg_hover());
    }

    let cx = row_rect.left() + 18.0;
    let cy = row_rect.center().y;
    let s = 4.0_f32;
    // Diamond icon
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            Pos2::new(cx, cy - s),
            Pos2::new(cx + s, cy),
            Pos2::new(cx, cy + s),
            Pos2::new(cx - s, cy),
        ],
        theme::accent_purple(),
        Stroke::NONE,
    ));

    let text_rect = Rect::from_min_max(
        Pos2::new(cx + 12.0, row_rect.min.y),
        Pos2::new(
            row_rect.max.x - widgets::ROW_ACTIONS_WIDTH - 8.0,
            row_rect.max.y,
        ),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(text_rect), |ui| {
        ui.horizontal_centered(|ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(name)
                        .color(theme::text_secondary())
                        .size(13.0),
                )
                .truncate(),
            );
        });
    });

    resp
}

fn empty_hint(ui: &mut Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(20.0);
        ui.add(egui::Label::new(RichText::new(text).color(theme::text_muted()).size(12.0)).wrap());
    });
}

#[cfg(test)]
mod interaction_tests {
    use super::*;

    #[test]
    fn narrow_sidebar_keeps_empty_hints_and_actions_inside_its_width() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::icons::font_definitions());
        let state = crate::state::new_shared_state();
        let mut graph = crate::git::graph::CommitGraph::default();
        graph.local_names.insert("main".into());
        {
            let mut state = state.lock().unwrap();
            state.commit_graph = Some(std::sync::Arc::new(graph));
            state.head_branch = Some("main".into());
        }
        let mut sidebar = Sidebar::new(state);
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(266.0, 800.0));
        for _ in 0..2 {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(rect),
                ..Default::default()
            });
            let mut ui = Ui::new(
                ctx.clone(),
                egui::Id::new("narrow_sidebar"),
                egui::UiBuilder::new().max_rect(rect),
            );
            assert!(sidebar.show(&mut ui).is_none());
            assert!(ui.min_rect().right() <= rect.right() + 0.5);
            let mut output = ctx.end_pass();
            output.textures_delta.clear();
            assert!(has_text(
                &output,
                "No tags yet. Create a tag to mark a commit."
            ));
        }
    }

    #[test]
    fn remote_groups_use_longest_configured_name_and_keep_branch_paths() {
        let remotes = vec!["team".into(), "team/origin".into(), "origin".into()];
        let groups = group_remote_branches(
            &[
                "origin/feature/ui",
                "team/main",
                "team/origin/topic",
                "orphan/topic",
            ],
            &remotes,
        );
        assert_eq!(groups["origin"], ["origin/feature/ui"]);
        assert_eq!(groups["team"], ["team/main"]);
        assert_eq!(groups["team/origin"], ["team/origin/topic"]);
        assert_eq!(groups["orphan"], ["orphan/topic"]);
    }
    use crate::git::test_support::TestRepo;

    fn frame(
        ctx: &egui::Context,
        sidebar: &mut Sidebar,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<SidebarAction>) {
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(320.0, 1200.0))),
            events,
            ..Default::default()
        });
        let action = egui::CentralPanel::default()
            .show(
                &mut egui::Ui::new(
                    ctx.clone(),
                    egui::Id::new("test_root"),
                    egui::UiBuilder::new().max_rect(ctx.content_rect()),
                ),
                |ui| sidebar.show(ui),
            )
            .inner;
        let mut output = ctx.end_pass();
        output.textures_delta.clear();
        (output, action)
    }

    fn pointer(pos: Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Default::default(),
        }
    }

    /// Press and release `button` at `pos`, returning any actions produced.
    fn click(
        ctx: &egui::Context,
        sidebar: &mut Sidebar,
        pos: Pos2,
        button: egui::PointerButton,
    ) -> Vec<SidebarAction> {
        let (_, pressed) = frame(
            ctx,
            sidebar,
            vec![egui::Event::PointerMoved(pos), pointer(pos, button, true)],
        );
        let (_, released) = frame(ctx, sidebar, vec![pointer(pos, button, false)]);
        pressed.into_iter().chain(released).collect()
    }

    fn text_position(output: &egui::FullOutput, label: &str) -> Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.galley.rect.translate(text.pos.to_vec2()).center())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing text: {label}"))
    }

    fn setup() -> (TestRepo, tempfile::TempDir, egui::Context, Sidebar) {
        let repo = TestRepo::new();
        repo.commit("base", b"base");
        let remote = tempfile::tempdir().unwrap();
        git2::Repository::init_bare(remote.path()).unwrap();
        repo.git_ok(&["remote", "add", "origin", remote.path().to_str().unwrap()]);
        repo.git_ok(&["checkout", "-b", "topic"]);
        repo.commit("topic", b"topic");
        repo.git_ok(&["push", "origin", "topic"]);
        repo.git_ok(&["checkout", "main"]);
        repo.git_ok(&["branch", "-D", "topic"]);
        repo.git_ok(&["branch", "feature"]);
        let state = crate::state::new_shared_state();
        {
            let mut s = state.lock().unwrap();
            s.repo_path = Some(repo.path().canonicalize().unwrap());
            s.head_branch = Some("main".into());
            s.commit_graph = Some(std::sync::Arc::new(
                crate::git::graph::CommitGraph::from_repo(&repo.open()).unwrap(),
            ));
        }
        let mut sidebar = Sidebar::new(state);
        sidebar.open.insert("REMOTE", true);
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::icons::font_definitions());
        (repo, remote, ctx, sidebar)
    }

    #[test]
    fn remote_branch_click_reveals_double_click_checks_out_and_menu_deletes() {
        let (repo, _remote, ctx, mut sidebar) = setup();
        let tip = repo
            .open()
            .refname_to_id("refs/remotes/origin/topic")
            .unwrap()
            .to_string();
        let (output, _) = frame(&ctx, &mut sidebar, vec![]);
        let row = text_position(&output, "origin/topic");

        let first = click(&ctx, &mut sidebar, row, egui::PointerButton::Primary);
        assert!(
            matches!(first.as_slice(), [SidebarAction::RevealCommit(oid)] if *oid == tip),
            "a single click reveals the branch tip"
        );
        let second = click(&ctx, &mut sidebar, row, egui::PointerButton::Primary);
        assert!(
            matches!(second.as_slice(), [SidebarAction::CheckoutRemoteBranch(name)] if name == "origin/topic"),
            "a double click checks out a tracking branch"
        );

        // Let the double-click window lapse before opening the menu.
        for _ in 0..40 {
            frame(&ctx, &mut sidebar, vec![]);
        }
        assert!(
            click(&ctx, &mut sidebar, row, egui::PointerButton::Secondary)
                .iter()
                .all(|action| !matches!(action, SidebarAction::DeleteRemoteBranch(_)))
        );
        let (output, _) = frame(&ctx, &mut sidebar, vec![]);
        let delete = text_position(&output, "Delete from Remote");
        let chosen = click(&ctx, &mut sidebar, delete, egui::PointerButton::Primary);
        assert!(
            matches!(chosen.as_slice(), [SidebarAction::DeleteRemoteBranch(name)] if name == "origin/topic"),
            "the menu requests confirmation for the clicked remote branch"
        );
    }

    fn has_text(output: &egui::FullOutput, label: &str) -> bool {
        output.shapes.iter().any(
            |shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == label),
        )
    }

    #[test]
    fn filter_matches_case_insensitively_and_expands_collapsed_sections() {
        let (_repo, _remote, ctx, mut sidebar) = setup();
        sidebar.open.insert("REMOTE", false);
        let (output, _) = frame(&ctx, &mut sidebar, vec![]);
        assert!(
            !has_text(&output, "origin/topic"),
            "remotes start collapsed"
        );
        assert!(has_text(&output, "feature"));

        sidebar.filter = " TOP ".into();
        let (output, _) = frame(&ctx, &mut sidebar, vec![]);
        assert!(has_text(&output, "origin/topic"));
        assert!(!has_text(&output, "feature"));
        assert!(!has_text(&output, "main"));
        assert!(has_text(&output, "No matches"), "empty sections say so");

        sidebar.filter.clear();
        let (output, _) = frame(&ctx, &mut sidebar, vec![]);
        assert!(
            !has_text(&output, "origin/topic"),
            "clearing restores the collapsed state"
        );
        assert!(has_text(&output, "feature") && has_text(&output, "main"));
    }

    #[test]
    fn local_branch_double_click_checks_out_other_branches_but_not_head() {
        let (_repo, _remote, ctx, mut sidebar) = setup();
        let (output, _) = frame(&ctx, &mut sidebar, vec![]);
        let feature = text_position(&output, "feature");
        let head = text_position(&output, "main");

        click(&ctx, &mut sidebar, feature, egui::PointerButton::Primary);
        let second = click(&ctx, &mut sidebar, feature, egui::PointerButton::Primary);
        assert!(
            matches!(second.as_slice(), [SidebarAction::CheckoutBranch(name)] if name == "feature")
        );

        for _ in 0..40 {
            frame(&ctx, &mut sidebar, vec![]);
        }
        click(&ctx, &mut sidebar, head, egui::PointerButton::Primary);
        let second = click(&ctx, &mut sidebar, head, egui::PointerButton::Primary);
        assert!(
            second
                .iter()
                .all(|action| !matches!(action, SidebarAction::CheckoutBranch(_))),
            "double-clicking the current branch does not check it out again"
        );
    }
}
