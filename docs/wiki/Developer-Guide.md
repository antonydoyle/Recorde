# Developer Guide

## Prerequisites

- Rust toolchain with Cargo.
- Native dependencies required by egui/eframe and the selected platform backend.
- Git.

The crate vendors libgit2 and OpenSSL through the `git2` dependency features, so
most Git library dependencies are built as part of the Rust build.

## Build

```bash
cargo build
```

For a release binary:

```bash
cargo build --release
```

The binary target is named `recorde`.

## Run

```bash
cargo run
```

To open a repository on startup:

```bash
cargo run -- /path/to/repo
```

## Test

```bash
cargo test
```

The current integration tests create temporary Git repositories and cover
capability detection plus background job completion behavior.

## Dependency Notes

The project uses:

- `eframe` and `egui` for UI.
- `git2` for repository operations.
- `rfd` for native file dialogs.
- `serde`, `toml`, and `dirs` for config persistence.
- `ureq` and `serde_json` for provider HTTP APIs.
- `tempfile` for tests.

`Cargo.toml` patches `eframe` to a vendored copy in `vendor/eframe`.

## Development Workflow

1. Make focused changes in the relevant module.
2. Run `cargo test` for behavior that can be tested without a UI.
3. Run `cargo check` or `cargo build` after UI or cross-module changes.
4. Manually exercise affected UI workflows with a disposable Git repository.
5. Update this wiki when behavior, configuration, packaging, or architecture
   changes.

## Adding a Git Operation

Use the existing operation style:

- Add low-level logic to the most specific `src/git/` module.
- Update `AppState` only with data the UI actually needs.
- For long-running remote work, route through `GitRunner` and `JobProgress`.
- For modal-driven flows, add or extend a `ModalKind` variant.
- Return user-facing errors into `AppState.error` and log useful context to
  `console_log`.

## Adding UI

Prefer a focused component in `src/ui/` when a panel or reusable control grows
beyond simple app-level modal handling. Components should render from shared
state and return explicit action enums. `RecordeApp` should remain responsible
for side effects and cross-module coordination.

## Adding Provider Support

Follow the GitHub and GitLab pattern:

- Add provider-specific API code in `src/providers/`.
- Convert provider responses into shared provider structs.
- Keep token lookup in the auth/config layer.
- Update `ProviderKind` and account UI where needed.
- Route load operations through `git/operations.rs` so state updates remain
  centralized.

