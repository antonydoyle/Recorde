# Packaging

The Release workflow can also be run manually from a branch to validate and build
every platform without publishing. After that run passes, push the matching
`vX.Y.Z` tag to publish; the tag must match `Cargo.toml`. Publishing waits for all
platform builds and the shared CI/security checks. Font licences are included in
portable archives and installers as well as the application's About panel.

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

## Authenticated update artifacts

The release pipeline builds the standalone `tools/update-signing` program before
exposing `RECORDE_UPDATE_SIGNING_KEY` to the signing step. Each updater target has
an Ed25519-signed manifest binding its exact release URL, version, platform,
architecture, package kind, byte length, SHA-256 digest and validity period.
Verification uses the public keys in `packaging/update-keys.json`; private keys
must never enter source control, build artifacts, command arguments or logs.
The signing step verifies each resulting envelope and artifact before publishing
and refuses pre-existing update envelopes in the artifact directory.

Release binaries use clean compilation rather than restored compiled dependency
caches. Portable archives are assembled outside the final artifact directory;
the raw portable executable is a separate signed update target. Its narrow
replacement preserves adjacent files. Releases that change adjacent resources
must use a complete package installation.

Updater signatures authenticate update bytes; they do not provide Windows
Authenticode or macOS notarization. Those distribution signatures remain parked.
`SHA256SUMS` and manual ZIP/tar archives are integrity aids rather than separately
signed update targets. The updater accepts only its verified signed envelopes.

Repository maintainers who can change release workflows or trusted keys are part
of the publishing trust boundary. Projects with multiple publishers should use
protected release tags and a restricted GitHub Environment for the signing
secret. Keep an offline backup of the private key and introduce a replacement
public key in a trusted application release before retiring an old key.

## Installer acceptance

The Windows and Linux release jobs exercise a previous installer, upgrade it to
the candidate, verify the installed binary, then uninstall it while checking
that user-data sentinels survive. Windows additionally checks product
registration and downgrade refusal. These scripts refuse to run outside a
disposable GitHub-hosted runner or over an existing Recorde installation.
They are supervised with explicit timeouts and upload their logs and process
ledgers. Passing these checks does not substitute for hands-on macOS acceptance.

