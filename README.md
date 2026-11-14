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
- [Packaging](docs/wiki/Packaging.md) for platform prerequisites, including the
  Microsoft Visual C++ x64 runtime required by Windows MSVC builds.

## What's new

See the [0.5.0 release notes](docs/release-notes/0.5.0.md).

## Building and testing

Windows and Linux validation uses bounded task runners, isolated test repositories,
and process cleanup checks. See the [Developer Guide](docs/wiki/Developer-Guide.md)
for commands. Local build packages belong in the ignored `dist/` directory;
generated executables, process logs and smoke-test repositories are not committed.

Windows builds are not Authenticode-signed and macOS builds are not notarized yet.

## Licence

Recorde is released under the [MIT licence](LICENSE).
