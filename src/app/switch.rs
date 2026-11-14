use super::*;
use crate::git::switch::{Choice, Preview};

impl RecordeApp {
    pub(super) fn render_switch_work(&mut self, ctx: &Context) {
        let (pending, restore, show_saved, records, warnings) = {
            let s = self.state.lock().unwrap();
            (
                s.pending_switch.clone(),
                s.pending_restore.clone(),
                s.show_saved_work,
                s.saved_work.clone(),
                s.saved_work_warnings.clone(),
            )
        };
        if !matches!(self.state.lock().unwrap().modal, ModalKind::None) {
            return;
        }
        if let Some(preview) = pending {
            self.render_switch_preview(ctx, preview);
            return;
        }
        if let Some(preview) = restore {
            let mut open = true;
            let mut apply = false;
            let mut cancel = false;
            let dirty_editor = self
                .repo_ui
                .file_editor
                .as_ref()
                .is_some_and(|e| e.is_dirty());
            egui::Window::new("Restore saved work").open(&mut open).collapsible(false).resizable(false).show(ctx,|ui|{
                ui.label(format!("Restore into {}",preview.destination_label()));
                ui.label(format!("Current work: {} tracked changes · {} untracked paths · {} ignored paths",preview.tracked,preview.untracked,preview.ignored));
                ui.label(format!("Saved on {}",preview.record.source_label()));
                ui.label(format!("Stash {}",preview.record.stash_oid.as_deref().unwrap_or("unverified")));
                ui.label("Restores staged, unstaged and untracked work. Conflicts can leave partial changes; the stash is kept. Inspect the current branch before continuing.");
                if dirty_editor {ui.colored_label(theme::accent_yellow(),"Save or discard the open editor draft before restoring work.");}
                ui.horizontal(|ui|{
                    apply=ui.add_enabled(!dirty_editor,primary_btn("Restore saved work")).clicked();
                    cancel=ui.add(secondary_btn("Cancel")).clicked();
                });
            });
            if !open || cancel || ctx.input(|i| i.key_pressed(Key::Escape)) || apply {
                self.state.lock().unwrap().pending_restore = None;
            }
            if apply {
                crate::git::switch::submit_restore(preview, self.state.clone());
            }
            return;
        }
        if show_saved {
            let mut open = true;
            let mut restore = None;
            let mut reconcile = None;
            egui::Window::new("Saved work").open(&mut open).default_width(520.0).show(ctx,|ui|{
                ui.label("Saved work stays in its stash and recovery pin. Restore is explicit and never drops a stash. Interrupted or failed restores require inspection before another apply.");
                for warning in &warnings {ui.colored_label(theme::accent_yellow(),crate::auth::redact::text(warning));}
                if records.is_empty() {ui.label("No branch-switch saved work is recorded for this worktree.");}
                egui::ScrollArea::vertical().max_height(420.0).show(ui,|ui|{
                    for record in &records {
                        ui.separator();
                        ui.label(format!("{} → {}",record.source_label(),record.target_label()));
                        ui.label(record.phase.label());
                        if let Some(oid)=&record.stash_oid {ui.monospace(format!("Stash {oid}"));}
                        if let Some(outcome)=&record.outcome {ui.label(crate::auth::redact::text(outcome));}
                        if record.can_restore() && ui.add(secondary_btn("Review restore")).clicked() {restore=Some(record.clone());}
                        if record.phase==crate::git::switch::Phase::StashSubmitted && ui.add(secondary_btn("Recover saved-work record")).clicked() {reconcile=Some(record.clone());}
                    }
                });
            });
            if !open {
                self.state.lock().unwrap().show_saved_work = false;
            }
            if let Some(record) = restore {
                crate::git::switch::request_restore(record, self.state.clone());
            }
            if let Some(record) = reconcile {
                crate::git::switch::reconcile(record, self.state.clone());
            }
        }
    }

    fn render_switch_preview(&mut self, ctx: &Context, preview: Preview) {
        let mut open = true;
        let mut choice = None;
        let mut cancel = false;
        let dirty_editor = self
            .repo_ui
            .file_editor
            .as_ref()
            .is_some_and(|e| e.is_dirty());
        egui::Window::new("Switch branch").open(&mut open).collapsible(false).resizable(false).default_width(500.0).show(ctx,|ui|{
            ui.label(format!("Repository {}",crate::ui::kit::display_path(&preview.path)));
            ui.label(format!("From {}",preview.source_label()));
            ui.label(format!("To {}",preview.target_label()));
            ui.monospace(format!("Commit {}",preview.target_oid()));
            ui.label(format!("{} tracked changes · {} untracked paths · {} ignored paths",preview.tracked,preview.untracked,preview.ignored));
            ui.label("Bring changes keeps local work where Git can carry it safely. Stash and switch saves staged, unstaged and untracked work, then switches with a clean worktree. Restore the saved work explicitly later.");
            ui.label("Ignored files are never stashed. A switch that would overwrite one is refused. Created branches and tracking settings are not promised automatic Undo.");
            if preview.is_detached() {ui.colored_label(theme::accent_yellow(),"The destination is a detached HEAD. Create a branch before keeping new commits.");}
            if dirty_editor {ui.colored_label(theme::accent_yellow(),"Save or discard the open editor draft before switching branches.");}
            ui.horizontal_wrapped(|ui|{
                if ui.add_enabled(!dirty_editor,primary_btn(if preview.has_changes(){"Bring changes"}else{"Switch branch"})).clicked(){choice=Some(Choice::Bring);}
                if ui.add_enabled(!dirty_editor&&preview.has_changes(),secondary_btn("Stash and switch")).clicked(){choice=Some(Choice::Stash);}
                cancel=ui.add(secondary_btn("Cancel")).clicked();
            });
        });
        if !open || cancel || ctx.input(|i| i.key_pressed(Key::Escape)) || choice.is_some() {
            self.state.lock().unwrap().pending_switch = None;
        }
        if let Some(choice) = choice {
            crate::git::switch::submit(preview, choice, self.state.clone());
        }
    }
}
