use std::collections::VecDeque;

use egui::{RichText, Stroke};

use crate::state::UserError;
use crate::theme;
use crate::ui::{kit, motion};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    GitReadiness,
    WorkflowScope,
    SignIn,
    HostKey,
    PushRejected,
    LocalChanges,
    Conflict,
    Hook,
    Network,
    Other,
}

/// Short messages (usually Recorde's own one-sentence refusals) are the useful
/// part of the notice, so they start expanded; long Git output stays collapsed
/// under a recognised title and guidance.
fn details_open_by_default(kind: ErrorKind, message: &str) -> bool {
    kind == ErrorKind::Other || (message.len() <= 240 && message.lines().count() <= 2)
}

pub fn classify(message: &str) -> ErrorKind {
    let message = message.to_ascii_lowercase();
    let rules: &[(ErrorKind, &[&str])] = &[
        (
            ErrorKind::GitReadiness,
            &["system git readiness", "retry git check"],
        ),
        (
            ErrorKind::WorkflowScope,
            &[
                "refusing to allow an oauth app to create or update workflow",
                "without `workflow` scope",
                "without workflow scope",
            ],
        ),
        (
            ErrorKind::SignIn,
            &[
                "authentication failed",
                "could not read username",
                "invalid username or password",
                "http 401",
                "the requested url returned error: 403",
                "permission denied (publickey)",
                "credential",
            ],
        ),
        (
            ErrorKind::HostKey,
            &["host key verification failed", "authenticity of host"],
        ),
        (
            ErrorKind::PushRejected,
            &[
                "[rejected]",
                "non-fast-forward",
                "fetch first",
                "updates were rejected",
                "stale info",
            ],
        ),
        (
            ErrorKind::LocalChanges,
            &[
                "would be overwritten",
                "please commit your changes or stash them",
                // Recorde's own refusals before multi-commit operations.
                "commit or stash",
            ],
        ),
        (ErrorKind::Conflict, &["conflict"]),
        (ErrorKind::Hook, &["hook"]),
        (
            ErrorKind::Network,
            &[
                "could not resolve host",
                "connection timed out",
                "connection refused",
                "unable to access",
                "no output from git",
                "network is unreachable",
                "timed out",
            ],
        ),
    ];
    for &(kind, patterns) in rules {
        if patterns.iter().any(|pattern| message.contains(*pattern)) {
            return kind;
        }
    }
    ErrorKind::Other
}

impl ErrorKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::GitReadiness => "Git needs attention",
            Self::SignIn => "Sign-in needed",
            Self::WorkflowScope => "GitHub needs the workflow permission",
            Self::HostKey => "Unknown SSH host",
            Self::PushRejected => "Push rejected",
            Self::LocalChanges => "Uncommitted changes are in the way",
            Self::Conflict => "Conflicts need resolving",
            Self::Hook => "A Git hook stopped the operation",
            Self::Network => "Network problem",
            Self::Other => "Operation failed",
        }
    }

    pub fn guidance(self) -> &'static str {
        match self {
            Self::GitReadiness => "Check the installed Git executable in Settings > Git, then retry the Git check. Repository browsing uses bundled libgit2.",
            Self::SignIn => {
                "Check the account for this remote in Settings > Accounts & SSH, or sign in again."
            }
            Self::WorkflowScope => concat!(
                "Sign in again in Settings > Accounts & SSH so Recorde can push changes to ",
                ".github/workflows, or use a token with the workflow scope."
            ),
            Self::HostKey => {
                "Connect once from a terminal to verify and save the host key, then retry."
            }
            Self::PushRejected => concat!(
                "The remote has commits you don't have. Pull first, or use Force Push with Lease ",
                "if you rewrote history on purpose."
            ),
            Self::LocalChanges => "Commit or stash your changes, then try again.",
            Self::Conflict => {
                "Resolve the conflicted files in the Changes panel, then Continue or Abort."
            }
            Self::Hook => "Read the hook output below, fix the problem, and retry.",
            Self::Network => "Check your connection, VPN or proxy, then retry.",
            Self::Other => "",
        }
    }
}

/// Gap from the window's right edge and the notice's widest text column.
const NOTICE_MARGIN: f32 = 16.0;
const NOTICE_WIDTH: f32 = 440.0;
/// Clears the status bar along the bottom of the window.
const NOTICE_BOTTOM: f32 = 40.0;

pub fn show(ctx: &egui::Context, errors: &mut VecDeque<UserError>) -> bool {
    let visible_id = egui::Id::new("error_overlay_visible");
    // The last dismissed notice, kept only while it fades out.
    let closing_id = egui::Id::new("error_overlay_closing");
    let (error, live) = match errors.front() {
        Some(error) => {
            ctx.data_mut(|data| data.remove::<UserError>(closing_id));
            (error.clone(), true)
        }
        None => match ctx.data(|data| data.get_temp::<UserError>(closing_id)) {
            Some(error) => (error, false),
            None => {
                motion::progress(ctx, visible_id, false, motion::FAST);
                return false;
            }
        },
    };
    let seconds = if live { motion::SLOW } else { motion::FAST };
    let progress = motion::progress(ctx, visible_id, live, seconds);
    if !live && progress <= 0.0 {
        ctx.data_mut(|data| data.remove::<UserError>(closing_id));
        return false;
    }
    let error = &error;
    let mut dismiss = false;
    let mut dismiss_all = false;
    let mut retry_git = false;
    // A toast in the bottom-right corner, above the status bar. Anchored over
    // the window's top it covered the commit rows an error usually refers to.
    egui::Area::new(egui::Id::new("error_overlay"))
        .anchor(
            egui::Align2::RIGHT_BOTTOM,
            [
                -NOTICE_MARGIN,
                -NOTICE_BOTTOM + motion::slide(progress, 12.0),
            ],
        )
        .order(egui::Order::Foreground)
        .interactable(live)
        .show(ctx, |ui| {
            if progress < 1.0 {
                ui.multiply_opacity(progress);
            }
            egui::Frame::NONE
                .fill(theme::bg_diff_del())
                .stroke(Stroke::new(1.5, theme::accent_red()))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    ui.set_max_width(NOTICE_WIDTH);
                    ui.horizontal(|ui| {
                        if error.kind == ErrorKind::GitReadiness {
                            retry_git = ui.add(kit::secondary_button("Retry Git check")).clicked();
                        }
                        ui.label(RichText::new("! ").color(theme::accent_red()).strong());
                        ui.label(
                            RichText::new(error.kind.title())
                                .color(theme::text_primary())
                                .size(13.5)
                                .strong(),
                        );
                    });
                    let guidance = error.kind.guidance();
                    if !guidance.is_empty() {
                        ui.add(
                            egui::Label::new(RichText::new(guidance).color(theme::text_primary()))
                                .wrap(),
                        );
                    }
                    kit::disclosure(
                        ui,
                        "error_details",
                        "Details",
                        details_open_by_default(error.kind, &error.message),
                        |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(160.0)
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&error.message)
                                                .color(theme::text_primary())
                                                .size(13.0),
                                        )
                                        .wrap()
                                        .selectable(true),
                                    );
                                });
                        },
                    );
                    ui.horizontal(|ui| {
                        if ui.add(kit::secondary_button("Copy details")).clicked() {
                            ctx.copy_text(error.message.clone());
                        }
                        dismiss = ui.add(kit::primary_button("Dismiss")).clicked();
                        if errors.len() > 1 {
                            dismiss_all = ui.add(kit::secondary_button("Dismiss all")).clicked();
                            ui.label(format!("1 of {}", errors.len()));
                        }
                    });
                });
        });
    if !live {
        return false;
    }
    if dismiss_all {
        errors.clear();
    } else if dismiss {
        errors.pop_front();
    }
    if (dismiss || dismiss_all) && errors.is_empty() {
        ctx.data_mut(|data| data.insert_temp(closing_id, error.clone()));
    }
    retry_git
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_messages_start_expanded_and_long_git_output_collapsed() {
        assert!(details_open_by_default(
            ErrorKind::LocalChanges,
            "Commit or stash staged and tracked working-tree changes before applying multiple commits."
        ));
        let long = "error: line\n".repeat(5);
        assert!(!details_open_by_default(ErrorKind::PushRejected, &long));
        assert!(details_open_by_default(ErrorKind::Other, &long));
    }

    #[test]
    fn classifies_git_failures_with_specific_rules_first() {
        let cases = [
            (
                "System Git readiness: install Git, then choose Retry Git check",
                ErrorKind::GitReadiness,
            ),
            (
                "fatal: Authentication failed for 'https://github.com/o/r.git/'",
                ErrorKind::SignIn,
            ),
            (
                concat!(
                    "remote: refusing to allow an OAuth App to create or update workflow ",
                    "`.github/workflows/ci.yml` without `workflow` scope"
                ),
                ErrorKind::WorkflowScope,
            ),
            (
                "credential rejected: without workflow scope",
                ErrorKind::WorkflowScope,
            ),
            ("Host key verification failed.", ErrorKind::HostKey),
            (
                "! [rejected] main -> main (fetch first)\nerror: failed to push some refs",
                ErrorKind::PushRejected,
            ),
            (
                "error: Your local changes to the following files would be overwritten by checkout",
                ErrorKind::LocalChanges,
            ),
            (
                "CONFLICT (content): Merge conflict in src/main.rs",
                ErrorKind::Conflict,
            ),
            ("remote: pre-receive hook declined", ErrorKind::Hook),
            (
                "No output from Git for 5 minutes; the transfer was stopped.",
                ErrorKind::Network,
            ),
            (
                "fatal: unable to access remote: The requested URL returned error: 403",
                ErrorKind::SignIn,
            ),
            ("fatal: unexpected object format", ErrorKind::Other),
        ];
        for (message, expected) in cases {
            assert_eq!(classify(message), expected, "{message}");
            assert_eq!(
                classify(&message.to_ascii_uppercase()),
                expected,
                "{message}"
            );
        }
    }

    #[test]
    fn dismissed_last_notice_fades_out_then_disappears() {
        let ctx = egui::Context::default();
        let mut errors = VecDeque::from([UserError {
            kind: ErrorKind::Network,
            message: "fatal: Could not resolve host: github.com".into(),
        }]);
        let frame = |errors: &mut VecDeque<UserError>, time: f64, events| {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 700.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            });
            show(&ctx, errors);
            let mut output = ctx.end_pass();
            output.textures_delta.clear();
            output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Dismiss" => {
                    Some(text.galley.rect.translate(text.pos.to_vec2()).center())
                }
                _ => None,
            })
        };
        let _ = frame(&mut errors, 0.0, vec![]);
        let position = frame(&mut errors, 1.0, vec![]).expect("Dismiss is rendered");
        let pointer = |pressed| egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let _ = frame(
            &mut errors,
            1.1,
            vec![egui::Event::PointerMoved(position), pointer(true)],
        );
        let _ = frame(&mut errors, 1.2, vec![pointer(false)]);
        // Dismissal empties the queue at once; the notice only lingers visually.
        assert!(errors.is_empty());
        let lingering = frame(&mut errors, 1.21, vec![]).is_some();
        assert!(lingering || motion::reduced());
        let _ = frame(&mut errors, 2.0, vec![]);
        assert!(
            frame(&mut errors, 3.0, vec![]).is_none(),
            "the faded notice is removed"
        );
    }

    #[test]
    fn rendered_dismiss_keeps_the_next_error() {
        let ctx = egui::Context::default();
        let mut errors = VecDeque::from([
            UserError {
                kind: ErrorKind::Network,
                message: "fatal: Could not resolve host: github.com".into(),
            },
            UserError {
                kind: ErrorKind::Hook,
                message: "pre-commit hook declined".into(),
            },
        ]);
        let frame = |errors: &mut VecDeque<UserError>, events| {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 700.0),
                )),
                events,
                ..Default::default()
            });
            show(&ctx, errors);
            let mut output = ctx.end_pass();
            output.textures_delta.clear();
            output
        };
        // Let the anchored area's size and position settle before locating the button.
        let _ = frame(&mut errors, vec![]);
        let output = frame(&mut errors, vec![]);
        let position = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Dismiss" => {
                    Some(text.galley.rect.translate(text.pos.to_vec2()).center())
                }
                _ => None,
            })
            .expect("Dismiss is rendered");
        let pointer = |pressed| egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let _ = frame(
            &mut errors,
            vec![egui::Event::PointerMoved(position), pointer(true)],
        );
        assert_eq!(errors.len(), 2);
        let _ = frame(&mut errors, vec![pointer(false)]);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors.front().unwrap().kind, ErrorKind::Hook);
    }
}
