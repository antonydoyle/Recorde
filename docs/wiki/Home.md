# Recorde Wiki

Recorde is a native desktop Git client focused on common repository workflows:
opening repositories, reviewing history, staging changes, committing, branching,
syncing with remotes, resolving conflicts, and checking provider metadata from
GitHub or GitLab.

This wiki documents the application from both user and developer perspectives.

## Contents

- [User Guide](User-Guide.md): using the app day to day.
- [Feature Reference](Feature-Reference.md): detailed capability map.
- [Architecture](Architecture.md): module boundaries and runtime flow.
- [Configuration](Configuration.md): persisted settings, accounts, and tokens.
- [Provider Integrations](Provider-Integrations.md): GitHub and GitLab support.
- [Developer Guide](Developer-Guide.md): local setup, tests, and release process.
- [Packaging](Packaging.md): Linux, macOS, and Windows packaging assets.
- [Troubleshooting](Troubleshooting.md): common runtime and Git issues.
- [GitKraken Gap Analysis](GitKraken-Gap-Analysis.md): audit findings, priorities, and implementation milestones.

## Repository Map

| Path | Purpose |
| --- | --- |
| `src/main.rs` | Native application entry point and window setup. |
| `src/app.rs` | Main egui application, layout, modal handling, command routing. |
| `src/state.rs` | Shared application state consumed by UI and Git workers. |
| `src/git/` | Git operations, background jobs, graph loading, staging, remotes, history, conflicts. |
| `src/ui/` | Reusable panels and widgets for toolbar, sidebar, graph, diff, staging, status. |
| `src/auth/` | Provider account metadata, token storage, and SSH diagnostics. |
| `src/providers/` | GitHub and GitLab API adapters. |
| `src/config.rs` | User configuration loading and saving. |
| `assets/` | Application icon and embedded Material Icons font. |
| `packaging/` | Desktop integration and platform packaging metadata. |
| `wix/` | Windows installer definition. |
| `tests/` | Integration tests for Git capability and job behavior. |

## Current Implementation Stack

- Rust 2021.
- `eframe` and `egui` for the native UI.
- `git2` with vendored libgit2/OpenSSL for repository access.
- `ureq` and `serde_json` for provider HTTP APIs.
- `toml`, `serde`, and `dirs` for local configuration.
- A vendored `eframe` patch under `vendor/eframe`.

