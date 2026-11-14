use egui::{FontId, RichText, Ui};

use crate::git::operations::{load_blame, load_diff, load_file_history};
use crate::state::SharedState;
use crate::theme;
use crate::ui::{kit, motion, widgets};

pub struct DetailPanel {
    state: SharedState,
}

impl DetailPanel {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, theme::bg_surface());

        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(12, 0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                self.show_body(ui);
            });
    }

    fn show_body(&mut self, ui: &mut Ui) {
        if self.show_multi_selection(ui) {
            return;
        }

        // Snapshot everything we need, then release the lock.
        let (selected_commit, detail, selected_file) = {
            let s = self.state.lock().unwrap();
            (
                s.selected_commit.clone(),
                s.selected_commit_detail.clone(),
                s.selected_file.clone(),
            )
        };

        if selected_commit.is_none() {
            kit::empty_state(
                ui,
                crate::icons::COMMIT,
                "No commit selected",
                "Select a commit in the history to see what changed.",
                None,
            );
            return;
        }

        let Some(detail) = detail else {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("Loading…")
                        .color(theme::text_muted())
                        .size(13.0),
                );
            });
            return;
        };

        // Collect any file the user clicks on during this frame.
        let mut clicked_file: Option<String> = None;
        let body_width = ui.available_width();

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.set_width(ui.available_width().min(body_width));
                ui.add_space(12.0);

                let first_line = detail.message.lines().next().unwrap_or("");
                let rest = detail
                    .message
                    .trim_start_matches(first_line)
                    .trim_start_matches('\n')
                    .trim_end();

                kit::card(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(first_line)
                                .size(16.0)
                                .strong()
                                .color(theme::text_primary()),
                        )
                        .wrap(),
                    );
                    if !rest.is_empty() {
                        ui.add_space(kit::space::XS);
                        ui.add(
                            egui::Label::new(
                                RichText::new(rest)
                                    .size(13.0)
                                    .color(theme::text_secondary()),
                            )
                            .wrap(),
                        );
                    }
                    ui.add_space(kit::space::MD);
                    ui.horizontal(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::Vec2::splat(28.0), egui::Sense::hover());
                        widgets::avatar_for(
                            ui.painter(),
                            rect.center(),
                            14.0,
                            &detail.author,
                            &detail.author_email,
                        );
                        ui.vertical(|ui| {
                            ui.set_width(ui.available_width());
                            ui.add(
                                egui::Label::new(RichText::new(&detail.author).strong()).truncate(),
                            )
                            .on_hover_text(&detail.author);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&detail.author_email)
                                        .size(12.0)
                                        .color(theme::text_muted()),
                                )
                                .truncate(),
                            )
                            .on_hover_text(&detail.author_email);
                        });
                    });
                    ui.label(
                        RichText::new(theme::relative_time(detail.timestamp))
                            .color(theme::text_muted()),
                    )
                    .on_hover_text(theme::absolute_time(detail.timestamp));
                    ui.horizontal(|ui| {
                        let short = &detail.oid[..7.min(detail.oid.len())];
                        widgets::pill(ui, short, theme::bg_panel(), theme::accent_blue());
                        if ui
                            .add(kit::small_button("Copy"))
                            .on_hover_text("Copy full commit ID")
                            .clicked()
                        {
                            ui.ctx().copy_text(detail.oid.clone());
                        }
                    });
                });

                ui.add_space(10.0);
                ui.add(egui::Separator::default().spacing(0.0));
                ui.add_space(6.0);

                // --- Changed files ---
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("Changed files  ({})", detail.changed_files.len()))
                            .color(theme::text_muted())
                            .size(12.0)
                            .strong(),
                    );
                });
                ui.add_space(4.0);

                for file in &detail.changed_files {
                    let is_sel = selected_file.as_deref() == Some(&file.path);
                    let row_resp = changed_file_row(ui, &file.path, file.status, is_sel);

                    if row_resp.clicked() {
                        clicked_file = Some(file.path.clone());
                    }

                    widgets::row_actions_menu(ui, &row_resp, &file.path, is_sel, |ui| {
                        ui.set_min_width(175.0);
                        if ui.button("Open in External Diff Tool").clicked() {
                            let repo_path = self.state.lock().unwrap().repo_path.clone();
                            if let Some(repo_path) = repo_path {
                                if let Err(error) = crate::external::open_diff_tool(
                                    &repo_path,
                                    crate::external::DiffToolArgs::Commit {
                                        path: &file.path,
                                        oid: &detail.oid,
                                    },
                                    self.state.clone(),
                                ) {
                                    self.state.lock().unwrap().error = Some(error);
                                }
                            }
                            ui.close();
                        }
                        if ui.button("Show File History").clicked() {
                            load_file_history(&file.path, self.state.clone());
                            ui.close();
                        }
                        if ui.button("Blame").clicked() {
                            {
                                let mut s = self.state.lock().unwrap();
                                s.selected_file = Some(file.path.clone());
                                s.show_blame = true;
                            }
                            load_blame(&file.path, self.state.clone());
                            ui.close();
                        }
                    });
                }

                ui.add_space(16.0);
            });

        // Handle file click outside the borrow scope.
        if let Some(path) = clicked_file {
            {
                let mut s = self.state.lock().unwrap();
                s.selected_file = Some(path.clone());
                s.current_diff = None;
            }
            load_diff(&detail.oid, &path, self.state.clone());
        }
    }
}

impl DetailPanel {
    /// Summary and actions for two or more selected commits.
    fn show_multi_selection(&mut self, ui: &mut Ui) -> bool {
        let rows: Vec<(String, String, String)> = {
            let s = self.state.lock().unwrap();
            if s.selected_commits.len() < 2 {
                return false;
            }
            let graph = s.commit_graph.clone();
            s.selected_commits_oldest_first()
                .into_iter()
                .rev()
                .map(|oid| {
                    let node = graph
                        .as_ref()
                        .and_then(|g| g.oid_to_row.get(&oid).map(|&row| &g.nodes[row]));
                    (
                        oid.clone(),
                        node.map(|n| n.summary.clone()).unwrap_or_default(),
                        node.map(|n| n.author.clone()).unwrap_or_default(),
                    )
                })
                .collect()
        };
        egui::ScrollArea::vertical()
            .id_salt("multi_selection")
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                egui::Frame::NONE
                    .inner_margin(egui::Margin::symmetric(0, 12))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(format!("{} commits selected", rows.len()))
                                .color(theme::text_primary())
                                .size(14.0)
                                .strong(),
                        );
                        ui.label(
                            RichText::new(
                                "Ctrl/Cmd-click toggles a commit; Shift-click selects a range.",
                            )
                            .color(theme::text_muted())
                            .size(12.0),
                        );
                        ui.add_space(8.0);
                        for (oid, summary, author) in &rows {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(&oid[..7.min(oid.len())])
                                        .font(FontId::monospace(12.0))
                                        .color(theme::accent_blue()),
                                );
                                ui.label(
                                    RichText::new(summary)
                                        .color(theme::text_secondary())
                                        .size(13.0),
                                )
                                .on_hover_text(author);
                            });
                        }
                        ui.add_space(10.0);
                        ui.separator();
                        crate::ui::commit_graph::multi_selection_actions(ui, &self.state);
                    });
            });
        true
    }
}

fn changed_file_row(ui: &mut Ui, path: &str, status: char, selected: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), egui::Sense::click());
    let fill = if selected {
        theme::bg_selected()
    } else {
        motion::mix(
            egui::Color32::TRANSPARENT,
            theme::bg_hover(),
            motion::hover(ui, &response),
        )
    };
    ui.painter().rect_filled(rect, 4.0, fill);
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            4.0,
            egui::Stroke::new(1.5, theme::accent_teal()),
            egui::StrokeKind::Inside,
        );
    }

    let badge_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left(), rect.center().y - 9.0),
        egui::Vec2::splat(18.0),
    );
    let mut badge_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(response.id.with("status"))
            .max_rect(badge_rect),
    );
    widgets::status_badge(&mut badge_ui, status).on_hover_text(status_tooltip(status));

    let text_left = badge_rect.right() + kit::space::SM;
    let text_right = (rect.right() - widgets::ROW_ACTIONS_WIDTH - 4.0).max(text_left);
    let text_rect = egui::Rect::from_min_max(
        egui::pos2(text_left, rect.top()),
        egui::pos2(text_right, rect.bottom()),
    );
    let font = FontId::proportional(13.0);
    let display = left_elided_path(path, text_rect.width(), |text| {
        ui.painter()
            .layout_no_wrap(text.to_owned(), font.clone(), theme::text_primary())
            .size()
            .x
    });
    let (directory, filename) = display
        .rfind('/')
        .map(|index| display.split_at(index + 1))
        .unwrap_or(("", &display));
    let mut job = egui::text::LayoutJob::default();
    for (text, color) in [
        (directory, theme::text_muted()),
        (filename, theme::text_primary()),
    ] {
        job.append(
            text,
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color,
                ..Default::default()
            },
        );
    }
    let galley = ui.painter().layout_job(job);
    ui.painter()
        .with_clip_rect(text_rect.intersect(ui.clip_rect()))
        .galley(
            egui::pos2(text_left, rect.center().y - galley.size().y / 2.0),
            galley,
            theme::text_primary(),
        );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            ui.is_enabled(),
            selected,
            path.rsplit('/').next().unwrap_or(path),
        )
    });
    response.on_hover_text(format!("{path}\nClick to view this file's changes"))
}

/// Keep the end of the path, removing whole Unicode characters from the left.
fn left_elided_path(path: &str, width: f32, measure: impl Fn(&str) -> f32) -> String {
    if measure(path) <= width {
        return path.to_owned();
    }
    if measure("…") > width {
        return String::new();
    }
    let boundaries: Vec<_> = path
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(path.len()))
        .collect();
    let mut first = 1;
    let mut last = boundaries.len() - 1;
    while first < last {
        let middle = first + (last - first) / 2;
        if measure(&format!("…{}", &path[boundaries[middle]..])) <= width {
            last = middle;
        } else {
            first = middle + 1;
        }
    }
    format!("…{}", &path[boundaries[last]..])
}

fn status_tooltip(status: char) -> &'static str {
    match status {
        'A' => "Added",
        'D' => "Deleted",
        'R' => "Renamed",
        'C' => "Copied",
        _ => "Modified",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_card_respects_the_side_margins_in_a_narrow_panel() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::icons::font_definitions());
        let state = crate::state::new_shared_state();
        {
            let mut state = state.lock().unwrap();
            state.selected_commit = Some("1234567890".into());
            state.selected_commit_detail = Some(crate::state::CommitDetail {
                oid: "1234567890".into(),
                author: "An author name that is longer than the card".into(),
                author_email: String::new(),
                timestamp: 0,
                message: "A commit summary that wraps inside the narrow card".into(),
                changed_files: vec![crate::state::ChangedFile {
                    path: "second.txt".into(),
                    status: 'A',
                }],
            });
        }
        let mut panel = DetailPanel::new(state);
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(340.0, 600.0));
        for _ in 0..2 {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(rect),
                ..Default::default()
            });
            let mut ui = Ui::new(
                ctx.clone(),
                egui::Id::new("narrow_details"),
                egui::UiBuilder::new().max_rect(rect),
            );
            panel.show(&mut ui);
            let mut output = ctx.end_pass();
            output.textures_delta.clear();
            let card = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(shape)
                        if shape.stroke.width == 1.0 && shape.rect.width() > 200.0 =>
                    {
                        Some(shape.rect)
                    }
                    _ => None,
                })
                .expect("Commit card is rendered");
            assert!(card.left() >= 12.0);
            assert!(card.right() <= 328.0);
        }
    }

    #[test]
    fn path_elision_preserves_the_filename_and_unicode_boundaries() {
        let measure = |text: &str| text.chars().count() as f32;
        assert_eq!(left_elided_path("second.txt", 10.0, measure), "second.txt");
        assert_eq!(
            left_elided_path("src/deep/second.txt", 12.0, measure),
            "…/second.txt"
        );
        assert_eq!(left_elided_path("目录/文档.txt", 7.0, measure), "…文档.txt");
        assert_eq!(left_elided_path("second.txt", 1.0, measure), "…");
        assert_eq!(left_elided_path("second.txt", 0.0, measure), "");
        assert_eq!(left_elided_path("", 0.0, measure), "");
    }

    #[test]
    fn file_row_starts_the_path_eight_points_after_the_badge() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::icons::font_definitions());
        let rect = egui::Rect::from_min_max(egui::pos2(12.0, 0.0), egui::pos2(328.0, 200.0));
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(340.0, 200.0),
            )),
            ..Default::default()
        });
        let mut ui = Ui::new(
            ctx.clone(),
            egui::Id::new("details_row_test"),
            egui::UiBuilder::new().max_rect(rect),
        );
        let row = changed_file_row(&mut ui, "second.txt", 'A', false);
        let actions = widgets::row_actions_menu(&mut ui, &row, "second.txt", false, |_| {});
        assert_eq!(row.rect.width(), rect.width());
        assert!(actions.rect.right() <= rect.right());
        let mut output = ctx.end_pass();
        output.textures_delta.clear();
        let path_left = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == "second.txt" => Some(text.pos.x),
            _ => None,
        });
        assert_eq!(path_left, Some(12.0 + 18.0 + 8.0));
    }
}
