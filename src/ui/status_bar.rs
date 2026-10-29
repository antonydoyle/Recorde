use egui::{RichText, Stroke};

use crate::icons;
use crate::state::SharedState;
use crate::theme;
use crate::ui::motion;

pub struct StatusBar {
    state: SharedState,
}

impl StatusBar {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub fn show(&self, root_ui: &mut egui::Ui) {
        egui::Panel::bottom("status_bar")
            .exact_size(28.0)
            .frame(egui::Frame::NONE.fill(theme::bg_titlebar()))
            .show(root_ui, |ui| {
                let r = ui.max_rect();
                ui.painter().line_segment(
                    [r.left_top(), r.right_top()],
                    Stroke::new(1.0, theme::border()),
                );

                ui.horizontal_centered(|ui| {
                    ui.add_space(12.0);

                    let s = self.state.lock().unwrap();

                    // ── Left: version + repo ───────────────────────────
                    ui.label(
                        RichText::new(concat!("Recorde v", env!("CARGO_PKG_VERSION")))
                            .color(theme::text_muted())
                            .size(12.0),
                    )
                    .on_hover_text("Recorde application version");

                    if let Some(ref p) = s.repo_path {
                        let name = p
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        ui.add_space(8.0);
                        ui.add(egui::Separator::default().vertical().spacing(2.0));
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(&name)
                                .color(theme::text_secondary())
                                .size(12.0),
                        )
                        .on_hover_text(crate::ui::kit::display_path(p));
                    }

                    // ── Right ─────────────────────────────────────────
                    let branch = s.head_branch.clone().unwrap_or_else(|| "—".into());
                    let ab = s.ahead_behind;
                    let upstream = s.head_upstream.clone();
                    let dirty = s.dirty_count;
                    let remote_op = s.remote_op.clone();
                    let fetching = s
                        .active_ops
                        .iter()
                        .any(|op| op.background && s.repo_path.as_ref() == Some(&op.repo_path));
                    let fetch_error = s
                        .background_fetch
                        .as_ref()
                        .filter(|status| s.repo_path.as_ref() == Some(&status.repo_path))
                        .map(|status| crate::auth::redact::text(&status.message));
                    let error = s
                        .error
                        .as_deref()
                        .or_else(|| s.errors.front().map(|error| error.message.as_str()))
                        .map(crate::auth::redact::text);
                    drop(s);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(12.0);

                        if let Some(ref e) = error {
                            ui.label(
                                RichText::new(format!("⚠ {}", truncate(e, 48)))
                                    .color(theme::accent_red())
                                    .size(12.0),
                            )
                            .on_hover_text(e);
                            ui.add_space(8.0);
                        } else if let Some(ref op) = remote_op {
                            ui.label(
                                RichText::new(truncate(op, 36))
                                    .color(theme::text_muted())
                                    .size(12.0),
                            )
                            .on_hover_text(op);
                            ui.add_space(8.0);
                        }

                        if fetching {
                            let pulse = motion::progress(
                                ui.ctx(),
                                ui.id().with("status_fetch_pulse"),
                                true,
                                motion::SLOW,
                            );
                            let color = motion::mix(
                                theme::text_muted(),
                                theme::accent_blue(),
                                pulse * 0.45,
                            );
                            ui.label(
                                RichText::new("Fetching…")
                                    .color(color)
                                    .size(12.0),
                            )
                            .on_hover_text("Checking the remote for new commits.");
                            ui.add_space(8.0);
                        } else if let Some(message) = &fetch_error {
                            ui.label(
                                RichText::new("Last fetch failed")
                                    .color(theme::accent_yellow())
                                    .size(12.0),
                            )
                            .on_hover_text(message);
                            ui.add_space(8.0);
                        }

                        if dirty > 0 {
                            let changed = format!("{} changed", counted(dirty, "file", "files"));
                            ui.label(
                                RichText::new(format!("● {changed}"))
                                    .color(theme::accent_yellow())
                                    .size(12.0),
                            )
                            .on_hover_text(format!("{changed} — open Changes to review"));
                            ui.add_space(8.0);
                        } else if error.is_none()
                            && remote_op.is_none()
                            && !fetching
                            && fetch_error.is_none()
                        {
                            ui.label(
                                RichText::new("✓ No uncommitted changes")
                                    .color(theme::accent_green())
                                    .size(12.0),
                            )
                            .on_hover_text("No uncommitted changes");
                            ui.add_space(8.0);
                        }

                        match ab {
                            Some((a, b)) if a > 0 && b > 0 => {
                                // Diverged — both ahead and behind: amber warning
                                let badge = egui::RichText::new(format!("↑{} ↓{}", a, b))
                                    .color(theme::accent_orange())
                                    .size(12.0);
                                ui.label(badge).on_hover_text(format!(
                                    "{} to push; {} to pull. \
                                     The branch has diverged from its upstream.",
                                    counted(a, "commit", "commits"),
                                    counted(b, "commit", "commits")
                                ));
                                ui.add_space(4.0);
                            }
                            Some((a, b)) => {
                                if b > 0 {
                                    ui.label(
                                        RichText::new(format!("↓{}", b))
                                            .color(theme::accent_yellow())
                                            .size(12.0),
                                    )
                                    .on_hover_text(format!(
                                        "{} to pull",
                                        counted(b, "commit", "commits")
                                    ));
                                }
                                if a > 0 {
                                    ui.label(
                                        RichText::new(format!("↑{}", a))
                                            .color(theme::accent_blue())
                                            .size(12.0),
                                    )
                                    .on_hover_text(format!(
                                        "{} to push",
                                        counted(a, "commit", "commits")
                                    ));
                                }
                                if a > 0 || b > 0 {
                                    ui.add_space(4.0);
                                }
                            }
                            None if !branch.is_empty() && branch != "—" => {
                                // Head branch with no upstream tracking
                                ui.label(
                                    RichText::new("no upstream")
                                        .color(theme::text_muted())
                                        .size(12.0),
                                )
                                .on_hover_text(
                                        "No upstream: this branch hasn't been published. Push to publish it.",
                                );
                                ui.add_space(4.0);
                            }
                            None => {}
                        }

                        ui.label(icons::rich(icons::COMMIT, 13.0).color(theme::accent_teal()));
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(&branch)
                                .color(theme::accent_teal())
                                .size(12.0),
                        )
                        .on_hover_text(format!("Current branch: {}", branch));
                        if let (Some(_), Some(upstream)) = (ab, upstream.as_deref()) {
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new(format!("→ {upstream}"))
                                    .color(theme::text_muted())
                                    .size(11.0),
                            )
                            .on_hover_text(upstream_tooltip(&branch, upstream));
                        }
                    });
                });
            });
    }
}

fn upstream_tooltip(branch: &str, upstream: &str) -> String {
    format!(
        "{branch} tracks {upstream}: Push sends your commits there and Pull brings its new commits here."
    )
}

fn counted(count: usize, singular: &str, plural: &str) -> String {
    let noun = if count == 1 { singular } else { plural };
    format!("{count} {noun}")
}

fn truncate(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let shortened: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}…", shortened)
    } else {
        shortened
    }
}

#[cfg(test)]
mod tests {
    use super::{counted, upstream_tooltip};

    #[test]
    fn upstream_tooltip_names_both_branches() {
        let tip = upstream_tooltip("main", "origin/main");
        assert!(tip.starts_with("main tracks origin/main:"), "{tip}");
    }

    #[test]
    fn file_and_commit_counts_use_singular_only_for_one() {
        for (singular, plural, expected) in [
            ("file", "files", ["0 files", "1 file", "2 files"]),
            ("commit", "commits", ["0 commits", "1 commit", "2 commits"]),
        ] {
            for (count, expected) in expected.into_iter().enumerate() {
                assert_eq!(counted(count, singular, plural), expected);
            }
        }
    }
}
