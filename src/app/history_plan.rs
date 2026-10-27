use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};

pub(super) enum Request {
    Rebase(String),
    Squash(Vec<String>),
    Drop(Vec<String>),
}
pub(super) struct Task {
    target: rebase::PlanTarget,
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<RebaseModal, String>>,
}
impl Drop for Task {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn prepare(
    target: &rebase::PlanTarget,
    request: Request,
    cancel: &Arc<AtomicBool>,
) -> Result<RebaseModal, String> {
    rebase::check_plan_cancel(cancel)?;
    target.validate()?;
    let (upstream, entries) = match request {
        Request::Rebase(upstream) => {
            let entries = rebase::load_rebase_entries_cancellable(&target.path, &upstream, cancel)?;
            (upstream, entries)
        }
        Request::Squash(oids) => rebase::squash_plan_cancellable(&target.path, &oids, cancel)?,
        Request::Drop(oids) => rebase::drop_plan_cancellable(&target.path, &oids, cancel)?,
    };
    let (summary, preserve) = target.review_metadata(&upstream, &entries, cancel)?;
    rebase::check_plan_cancel(cancel)?;
    target.validate()?;
    let mut modal = RebaseModal::new(upstream, summary, entries);
    modal.preserve_merges = preserve;
    Ok(modal)
}

impl RecordeApp {
    pub(super) fn start_history_plan(&mut self, request: Request) {
        if self.history_plan.is_some() {
            return;
        }
        let (path, epoch) = {
            let s = self.state.lock().unwrap();
            (s.repo_path.clone(), s.repo_epoch)
        };
        let Some(path) = path else { return };
        let target = match rebase::PlanTarget::capture(&path, epoch) {
            Ok(target) => target,
            Err(error) => {
                self.state.lock().unwrap().error = Some(error);
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let worker_target = target.clone();
        std::thread::spawn(move || {
            let result = crate::git::process::with_cancellation(worker_cancel.clone(), || {
                prepare(&worker_target, request, &worker_cancel)
            });
            let _ = sender.send(result);
        });
        self.history_plan = Some(Task {
            target,
            cancel,
            receiver,
        });
    }

    pub(super) fn poll_history_plan(&mut self, ctx: &Context) {
        let Some(task) = &self.history_plan else {
            return;
        };
        let result = match task.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                "History preparation stopped unexpectedly. Please retry.".into(),
            )),
        };
        if let Some(result) = result {
            let task = self.history_plan.take().unwrap();
            let mut state = self.state.lock().unwrap();
            if task.cancel.load(Ordering::Relaxed)
                || state.repo_path.as_ref() != Some(&task.target.path)
                || state.repo_epoch != task.target.epoch
            {
                return;
            }
            match result.and_then(|modal| task.target.validate().map(|_| modal)) {
                Ok(modal) => {
                    self.repo_ui.rebase_target = Some(task.target.clone());
                    self.repo_ui.rebase_modal = Some(modal);
                }
                Err(error) => state.error = Some(error),
            }
            return;
        }
        egui::Modal::new(egui::Id::new("prepare_history_plan")).show(ctx, |ui| {
            ui.heading("Preparing history plan");
            ui.spinner();
            ui.label("Reading the selected history. The repository is not being changed.");
            if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                task.cancel.store(true, Ordering::Relaxed);
            }
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::TestRepo;
    #[test]
    fn background_plan_rejects_head_changes_and_keeps_git_failures_specific() {
        let repo = TestRepo::new();
        let first = repo.commit("a", b"first");
        repo.commit("a", b"second");
        let target = rebase::PlanTarget::capture(repo.path(), 1).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        assert!(prepare(&target, Request::Rebase(first.to_string()), &cancel).is_ok());
        let error = prepare(
            &target,
            Request::Rebase("ffffffffffffffffffffffffffffffffffffffff".into()),
            &cancel,
        )
        .err()
        .unwrap();
        assert!(error.contains("Cannot load the rebase plan"), "{error}");
        repo.commit("a", b"third");
        assert!(
            prepare(&target, Request::Rebase(first.to_string()), &cancel)
                .err()
                .unwrap()
                .contains("HEAD changed")
        );
    }

    #[test]
    fn cancelled_history_plan_never_produces_a_review_modal() {
        let repo = TestRepo::new();
        let first = repo.commit("a", b"first");
        let second = repo.commit("a", b"second");
        let target = rebase::PlanTarget::capture(repo.path(), 1).unwrap();
        let cancel = Arc::new(AtomicBool::new(true));
        for request in [
            Request::Rebase(first.to_string()),
            Request::Squash(vec![second.to_string()]),
            Request::Drop(vec![second.to_string()]),
        ] {
            assert_eq!(
                prepare(&target, request, &cancel).err().unwrap(),
                "History preparation cancelled."
            );
        }
        assert_eq!(repo.head(), second);
    }
}
