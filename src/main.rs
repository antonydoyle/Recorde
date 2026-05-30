mod app;
mod git;
mod state;
mod ui;

use app::RecordeApp;

fn main() -> eframe::Result<()> {
    env_logger::init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Recorde")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Recorde",
        options,
        Box::new(|cc| Ok(Box::new(RecordeApp::new(cc)))),
    )
}
