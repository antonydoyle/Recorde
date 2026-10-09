# Recorde

Recorde is a cross-platform desktop Git client built with Rust, egui, eframe, and
libgit2. It provides repository browsing, staging, history inspection, branch and
tag workflows, merge/rebase support, stashes, submodules, worktrees, and basic
GitHub/GitLab integrations from a native desktop UI.

## Documentation

The application wiki lives in [docs/wiki/Home.md](docs/wiki/Home.md).

Start with:

- [User Guide](docs/wiki/User-Guide.md) for everyday workflows.
- [Architecture](docs/wiki/Architecture.md) for how the app is structured.
- [Developer Guide](docs/wiki/Developer-Guide.md) for setup, testing, and release notes.

## Local test builds

Windows and Linux validation uses bounded task runners, isolated test repositories,
and process cleanup checks. See the [Developer Guide](docs/wiki/Developer-Guide.md)
for commands. Local build packages belong in the ignored `dist/` directory;
generated executables, process logs and smoke-test repositories are not committed.

The [gap analysis](docs/wiki/GitKraken-Gap-Analysis.md) records completed acceptance
checks and remaining limits. macOS acceptance and executable signing are parked.
