use std::io::Write;

/// Install a panic hook that writes a crash report to the user's data directory.
pub fn install() {
    std::panic::set_hook(Box::new(|info| {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let report = format!(
            "Recorde {} crash report\nTimestamp: {}\nOS: {} {}\n\n{}\n",
            env!("CARGO_PKG_VERSION"),
            ts,
            std::env::consts::OS,
            std::env::consts::ARCH,
            info,
        );

        if let Some(data_dir) = dirs::data_dir() {
            let crash_dir = data_dir.join("recorde");
            let _ = std::fs::create_dir_all(&crash_dir);
            let path = crash_dir.join(format!("crash-{}.txt", ts));
            if let Ok(mut f) = std::fs::File::create(&path) {
                let _ = f.write_all(report.as_bytes());
                eprintln!("Recorde: crash report written to {}", path.display());
            }
        }

        eprintln!("{}", report);
    }));
}
