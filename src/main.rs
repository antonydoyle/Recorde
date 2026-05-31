mod app;
mod config;
mod git;
mod state;
mod theme;
mod ui;

use app::RecordeApp;

fn main() -> eframe::Result<()> {
    env_logger::init();

    // Optional: pass a repo path as the first CLI argument for quick testing.
    let initial_repo = std::env::args().nth(1).map(std::path::PathBuf::from);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Recorde")
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Recorde",
        options,
        Box::new(move |cc| Ok(Box::new(RecordeApp::new(cc, initial_repo)))),
    )
}
