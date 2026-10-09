use crate::git::capabilities::ProviderType;
use crate::providers::write::{self, Action, Context, Decision, MergeMethod};
use crate::state::SharedState;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

#[path = "provider_draft.rs"]
mod draft;

pub struct ProviderEditor {
    persisted: Option<Action>,
    draft_error: Option<String>,
    path: PathBuf,
    context: Option<Context>,
    preparing: Option<Receiver<Result<Context, String>>>,
    submitting: Option<Receiver<Result<String, String>>>,
    action: Action,
    confirming: bool,
    error: Option<String>,
    success: Option<String>,
}

impl ProviderEditor {
    pub fn new(path: PathBuf, remote: Option<(String, String)>, action: Action) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let source = path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(write::prepare(source, remote));
        });
        Self {
            persisted: None,
            draft_error: None,
            path,
            context: None,
            preparing: Some(rx),
            submitting: None,
            action,
            confirming: false,
            error: None,
            success: None,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, state: SharedState) -> bool {
        if let Some(rx) = &self.preparing {
            match rx.try_recv() {
                Ok(Ok(context)) => {
                    if let Action::Create {
                        source_project,
                        target_project,
                        ..
                    } = &mut self.action
                    {
                        let project = format!("{}/{}", context.owner, context.project);
                        if source_project.is_empty() {
                            *source_project = project.clone();
                        }
                        if target_project.is_empty() {
                            *target_project = project;
                        }
                    }
                    match draft::load(&context, &self.action) {
                        Ok(Some(saved)) => self.action = saved,
                        Ok(None) => {}
                        Err(error) => self.draft_error = Some(error),
                    }
                    self.persisted = Some(self.action.clone());
                    self.context = Some(context);
                    self.preparing = None;
                }
                Ok(Err(error)) => {
                    self.error = Some(error);
                    self.preparing = None;
                }
                Err(TryRecvError::Disconnected) => {
                    self.error =
                        Some("Could not load provider account. Reopen the form to retry.".into());
                    self.preparing = None;
                }
                _ => {}
            }
        }
        if let Some(rx) = &self.submitting {
            match rx.try_recv() {
                Ok(result) => {
                    self.submitting = None;
                    self.confirming = false;
                    match result {
                        Ok(message) => {
                            self.success = Some(message.clone());
                            if let Some(context) = &self.context {
                                if let Err(error) = draft::remove(context, &self.action) {
                                    self.draft_error = Some(error);
                                }
                            }
                            let current = {
                                let mut s = state.lock().unwrap();
                                s.console_log
                                    .push(format!("{}: {message}", self.path.display()));
                                let current = s.repo_path.as_ref() == Some(&self.path);
                                s.invalidate_provider_data();
                                current
                            };
                            if current {
                                crate::git::operations::load_prs(state.clone());
                            }
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.submitting = None;
                    self.confirming = false;
                    self.error = Some("No confirmed result. Refresh the provider before retrying; the action may have succeeded.".into());
                }
                _ => {}
            }
        }
        let title = match self.action {
            Action::Create { .. } => "Create pull / merge request",
            Action::Review { .. } => "Submit review",
            Action::Merge { .. } => "Merge review",
        };
        let mut open = true;
        let mut submit = false;
        egui::Window::new(title).id(egui::Id::new("provider_write_form")).open(&mut open).default_width(560.0).collapsible(false).show(ctx, |ui| {
            ui.label(self.path.display().to_string());
            if let Some(error) = &self.error { ui.colored_label(crate::theme::accent_red(), error); }
            if let Some(error) = &self.draft_error { ui.colored_label(crate::theme::accent_red(), error); }
            if let Some(message) = &self.success { ui.label(message); return; }
            if self.preparing.is_some() { ui.spinner(); ui.label("Loading provider account..."); return; }
            let Some(context) = &self.context else { return };
            ui.label(format!("{} / {}/{} via {}", context.host, context.owner, context.project, context.remote));
            ui.label(format!("Submit as @{}", context.account));
            ui.separator();
            if self.submitting.is_some() { ui.spinner(); ui.label("Submitting. Waiting for the provider's result..."); }
            let github = context.provider == ProviderType::GitHub;
            ui.add_enabled_ui(!self.confirming && self.submitting.is_none(), |ui| {
                match &mut self.action {
                    Action::Create { title, body, source, target, draft, source_project, target_project } => {
                        for (label, project) in [("Source project", source_project), ("Target project", target_project)] {
                            egui::ComboBox::from_label(label).selected_text(project.as_str()).show_ui(ui, |ui| {
                                for choice in &context.projects { ui.selectable_value(project, choice.clone(), choice); }
                            });
                            ui.text_edit_singleline(project);
                        }
                        ui.small("Projects must share this provider host and belong to a compatible fork network.");
                        ui.label("Source branch (must already be pushed)");
                        ui.text_edit_singleline(source);
                        ui.label("Target branch"); ui.text_edit_singleline(target);
                        ui.label("Title"); ui.text_edit_singleline(title);
                        ui.label("Description");
                        ui.add(egui::TextEdit::multiline(body).desired_rows(8).desired_width(f32::INFINITY));
                        ui.checkbox(draft, "Create as draft");
                    }
                    Action::Review { number, head, target, body, decision, inline } => {
                        ui.label(format!("Review #{number}, commit {head}, target {target}"));
                        ui.horizontal(|ui| {
                            ui.selectable_value(decision, Decision::Comment, "Comment");
                            ui.selectable_value(decision, Decision::Approve, "Approve");
                            if github { ui.selectable_value(decision, Decision::RequestChanges, "Request changes"); }
                        });
                        if *decision == Decision::Comment {
                            let mut enabled = inline.is_some();
                            if ui.checkbox(&mut enabled, "Comment on a diff line").changed() {
                                *inline = enabled.then(crate::providers::write::InlineComment::default);
                            }
                            if let Some(comment) = inline {
                                ui.label("File path in the diff"); ui.text_edit_singleline(&mut comment.path);
                                if !github { ui.label("Previous path if renamed (otherwise leave blank)"); ui.text_edit_singleline(&mut comment.old_path); }
                                ui.horizontal(|ui| {
                                    ui.label("New line (0 for deletion)"); ui.add(egui::DragValue::new(&mut comment.new_line).range(0..=10000000));
                                    ui.label("Old line (0 for addition)"); ui.add(egui::DragValue::new(&mut comment.old_line).range(0..=10000000));
                                });
                                ui.small("Use line numbers from this review's diff. For unchanged GitLab lines, supply both numbers.");
                            }
                        } else { *inline = None; }
                        if github || *decision == Decision::Comment {
                            ui.add(egui::TextEdit::multiline(body).desired_rows(8).desired_width(f32::INFINITY));
                        } else { body.clear(); ui.label("Approval applies to the displayed commit. Submit any comment separately."); }
                    }
                    Action::Merge { number, head, target, method, queue } => {
                        ui.label(format!("Merge #{number} into {target}"));
                        ui.label(format!("Expected source commit: {head}"));
                        ui.checkbox(queue, if github { "Use merge queue" } else { "Use merge train" });
                        if *queue { ui.label("The repository must have a merge queue/train enabled. The provider runs checks before merging; source branches are retained."); }
                        ui.add_enabled_ui(!(*queue && github), |ui| { ui.horizontal(|ui| {
                            ui.selectable_value(method, MergeMethod::Merge, if github { "Merge commit" } else { "Project merge strategy" });
                            ui.selectable_value(method, MergeMethod::Squash, "Squash");
                            if github { ui.selectable_value(method, MergeMethod::Rebase, "Rebase" ); }
                        }); });
                        if *queue && github { ui.small("The repository's merge queue configuration chooses the merge method."); }
                        ui.label("The provider enforces permissions and branch rules. The source branch is retained.");
                    }
                }
            });
            if self.submitting.is_none() {
                if self.confirming {
                    ui.separator();
                    ui.label("Confirm the destination, account and action above.");
                    ui.horizontal(|ui| {
                        submit = ui.button("Confirm and submit").clicked();
                        if ui.button("Back to editing").clicked() { self.confirming = false; }
                    });
                } else if ui.button("Review action").clicked() {
                    match write::validate(&self.action, &context.provider) {
                        Ok(()) => { self.error = None; self.confirming = true; }
                        Err(error) => self.error = Some(error),
                    }
                }
            }
        });
        if self.success.is_none() && self.persisted.as_ref() != Some(&self.action) {
            if let Some(context) = &self.context {
                match draft::save(context, &self.action) {
                    Ok(()) => {
                        self.persisted = Some(self.action.clone());
                        self.draft_error = None;
                    }
                    Err(error) => self.draft_error = Some(error),
                }
            }
        }
        if submit {
            let context = self.context.clone().unwrap();
            let action = self.action.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            self.submitting = Some(rx);
            self.error = None;
            std::thread::spawn(move || {
                let _ = tx.send(write::execute(&context, &action));
            });
        }
        if self.preparing.is_some() || self.submitting.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        !open && self.submitting.is_none()
    }
}
