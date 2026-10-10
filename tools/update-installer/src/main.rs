//! No service, network, shell, configurable command, test-key mode or trust-key argument.
#[cfg(target_os = "linux")]
mod installer;
#[cfg(target_os = "linux")]
mod process;

fn main() {
    #[cfg(target_os = "linux")]
    let result = installer::run(std::env::args_os().skip(1).collect());
    #[cfg(not(target_os = "linux"))]
    let result: Result<(), String> = Err("This installer is only available on Linux.".into());
    if let Err(error) = result {
        eprintln!("Recorde update was not completed: {error}");
        std::process::exit(1);
    }
}

