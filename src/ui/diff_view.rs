use egui::Ui;

use crate::state::SharedState;

pub struct DiffView {
    #[allow(dead_code)]
    state: SharedState,
}

impl DiffView {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        ui.label("Diff view — coming soon.");
    }
}
