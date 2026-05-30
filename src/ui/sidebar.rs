use egui::Ui;

use crate::state::SharedState;

pub struct Sidebar {
    state: SharedState,
}

impl Sidebar {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        let state = self.state.lock().unwrap();

        ui.heading("Branches");
        ui.separator();

        if let Some(ref graph) = state.commit_graph {
            // Collect unique ref names across all commits.
            let mut refs: Vec<&str> = graph
                .nodes
                .iter()
                .flat_map(|n| n.refs.iter().map(|r| r.as_str()))
                .collect();
            refs.sort_unstable();
            refs.dedup();

            egui::ScrollArea::vertical().show(ui, |ui| {
                for name in refs {
                    ui.label(name);
                }
            });
        } else {
            ui.label("No repository open.");
        }
    }
}
