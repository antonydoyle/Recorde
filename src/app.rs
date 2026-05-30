use egui::Context;

use crate::state::{new_shared_state, SharedState};
use crate::ui::{commit_graph::CommitGraphView, sidebar::Sidebar, toolbar::Toolbar};

pub struct RecordeApp {
    state: SharedState,
    sidebar: Sidebar,
    commit_graph_view: CommitGraphView,
    toolbar: Toolbar,
}

impl RecordeApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let state = new_shared_state();
        Self {
            sidebar: Sidebar::new(state.clone()),
            commit_graph_view: CommitGraphView::new(state.clone()),
            toolbar: Toolbar::new(state.clone()),
            state,
        }
    }
}

impl eframe::App for RecordeApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.toolbar.show(ctx);

        egui::SidePanel::left("sidebar")
            .resizable(true)
            .default_width(220.0)
            .show(ctx, |ui| {
                self.sidebar.show(ui);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            self.commit_graph_view.show(ui);
        });
    }
}
