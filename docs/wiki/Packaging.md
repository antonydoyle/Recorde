# Packaging

Recorde includes packaging metadata for Linux desktop integration, Debian-style
packages, macOS app metadata, and Windows WiX installers.

## Linux Desktop Files

`packaging/recorde.desktop` provides the desktop entry for Linux environments.
`packaging/install.sh` contains install helper logic.

The application icon is stored at `assets/icon.png` and referenced by packaging
metadata.

## Debian Metadata

`Cargo.toml` contains `package.metadata.deb`:

- Section: `devel`.
- Priority: `optional`.
- Dependencies include OpenGL, GTK, GLib, fontconfig, and FreeType libraries.
- Assets install the release binary, icon, and desktop entry.

## macOS Metadata

`packaging/Info.plist` contains macOS bundle metadata.

## Windows Installer

Windows installer metadata is split between:

- `Cargo.toml` `package.metadata.wix` values for upgrade/path GUIDs.
- `wix/main.wxs` for WiX installer layout.

`src/main.rs` uses `windows_subsystem = "windows"` on Windows builds so launching
the GUI does not also open a console window.

## Runtime Assets

The app icon is embedded for the native window icon and title bar texture.
`assets/MaterialIcons.ttf` is embedded into the binary at compile time so
installed builds do not need that font file beside the executable.

