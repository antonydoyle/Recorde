// On Windows, suppress the console window that would otherwise open alongside the GUI.
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

use recorde::{app::RecordeApp, auth, crash};

fn main() -> eframe::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("Recorde {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("--credential-helper") {
        auth::helper::run(&std::env::args().nth(2).unwrap_or_default());
        return Ok(());
    }
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
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([980.0, 680.0])
            .with_decorations(true),
        ..Default::default()
    };

    eframe::run_native(
        "Recorde",
        options,
        Box::new(move |cc| {
            // COSMIC/Wayland can wait indefinitely on the first eglSwapBuffers
            // for a hidden window. Show before painting, using upstream's API.
            if let Some(window) = cc.winit_window() {
                window.set_visible(true);
            }
            Ok(Box::new(RecordeApp::new(cc, initial_repo)))
        }),
    )
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn test_executable_uses_console_subsystem() {
        let bytes = std::fs::read(std::env::current_exe().unwrap()).unwrap();
        let offset = u32::from_le_bytes(bytes[60..64].try_into().unwrap()) as usize;
        let subsystem = u16::from_le_bytes(bytes[offset + 92..offset + 94].try_into().unwrap());
        assert_eq!(
            subsystem, 3,
            "The GUI subsystem attribute must exclude test executables"
        );
    }
}
