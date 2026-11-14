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

## Roadmap

Planned work is tracked on the Recorde Product Roadmap
board. The `T1`–`T13` cards in its Backlog list are ordered by run sequence and
derive from the [gap analysis](docs/wiki/GitKraken-Gap-Analysis.md) and the
[parity review](docs/reviews/2026-10-27-parity-review.md). Each card states its
evidence, target files, and the local command that verifies it, since hosted CI is
currently paused.

## Local test builds

Windows and Linux validation uses bounded task runners, isolated test repositories,
and process cleanup checks. See the [Developer Guide](docs/wiki/Developer-Guide.md)
for commands. Local build packages belong in the ignored `dist/` directory;
generated executables, process logs and smoke-test repositories are not committed.

The [gap analysis](docs/wiki/GitKraken-Gap-Analysis.md) records completed acceptance
checks and remaining limits. macOS acceptance and executable signing are parked.

## Roadmap analysis

The preserved GitKraken analysis and review provenance are indexed in [docs/analysis/gitkraken-2026-11-04/README.md](docs/analysis/gitkraken-2026-11-04/README.md). Read the limitations before using historical findings.
