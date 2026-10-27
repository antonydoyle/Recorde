fn main() {
    println!("cargo:rerun-if-changed=../../Cargo.toml");
    let source = std::fs::read_to_string("../../Cargo.toml").expect("Recorde package metadata");
    let package: toml::Value = source.parse().expect("Recorde package TOML");
    let version = package["package"]["version"]
        .as_str()
        .expect("Recorde version");
    println!("cargo:rustc-env=RECORDE_INSTALLED_FLOOR={version}");
}
