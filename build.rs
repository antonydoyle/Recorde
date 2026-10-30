//! Injects the public client IDs of Recorde's own OAuth apps.
//!
//! Each ID comes from the environment (release builds set them from GitHub
//! secrets) or, for local builds, from the untracked `recorde-oauth.env` at the
//! repository root (`NAME=value` lines). Without either, the build has no
//! built-in app and browser sign-in asks for a user-registered client ID.
use std::path::Path;

const NAMES: [&str; 2] = [
    "RECORDE_GITHUB_OAUTH_CLIENT_ID",
    "RECORDE_GITLAB_OAUTH_CLIENT_ID",
];
const LOCAL_FILE: &str = "recorde-oauth.env";

fn main() {
    let root = std::env::var("CARGO_MANIFEST_DIR").expect("Cargo sets CARGO_MANIFEST_DIR");
    let local = Path::new(&root).join(LOCAL_FILE);
    // Watching a missing path, or the whole root, would rerun this script on
    // every build. After creating the file once, run `cargo clean -p recorde`.
    if local.is_file() {
        println!("cargo:rerun-if-changed={LOCAL_FILE}");
    }
    println!("cargo:rerun-if-changed=build.rs");
    let file = std::fs::read_to_string(&local).unwrap_or_default();
    for name in NAMES {
        println!("cargo:rerun-if-env-changed={name}");
        let value = std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| from_file(&file, name));
        if let Some(value) = value {
            let value = value.trim();
            if !valid_client_id(value) {
                panic!("{name} is not a valid public OAuth client ID");
            }
            println!("cargo:rustc-env={name}={value}");
        }
    }
}

fn from_file(file: &str, name: &str) -> Option<String> {
    file.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim().trim_matches('"').to_owned())
        .filter(|value| !value.is_empty())
}

/// Client IDs are short opaque tokens; reject anything that could carry
/// whitespace, quotes or control characters into the binary.
fn valid_client_id(value: &str) -> bool {
    (1..=256).contains(&value.len())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}
