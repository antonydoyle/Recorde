mod app;
mod auth;
mod config;
mod crash;
mod git;
mod icons;
mod providers;
mod state;
mod theme;
mod ui;

use app::RecordeApp;

fn main() -> eframe::Result<()> {
    crash::install();
    env_logger::init();

    let initial_repo = std::env::args().nth(1).map(std::path::PathBuf::from);

    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
        .expect("failed to load app icon");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Recorde")
            .with_app_id("recorde")
            .with_icon(icon)
            .with_inner_size([1240.0, 760.0])
            .with_min_inner_size([900.0, 600.0])
            .with_decorations(true),
        ..Default::default()
    };

    eframe::run_native(
        "Recorde",
        options,
        Box::new(move |cc| Ok(Box::new(RecordeApp::new(cc, initial_repo)))),
    )
}
