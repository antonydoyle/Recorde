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
cargo test --locked --all-targets
```

Tests create temporary Git repositories and cover graph structure, capabilities,
jobs, safe detached checkout, partial staging/unstaging bytes, cherry-pick/revert
conflicts and continuation, commit hooks, message-only amend, concurrent rebases,
draft retention and obsolete repository-load results. New Git fixtures set local
identity, hooks and newline defaults to avoid depending on developer settings.

Credential tests inject a vault backend to exercise migration, locked/unavailable
storage, exact host/account matching and secret redaction without touching personal
credentials. A local HTTP fixture checks token redirect isolation. Mutation queue
tests cover linked worktrees, ordered staging/commit and cancellation of a queued
push against a disposable bare remote. Native vaults require separate desktop
smoke tests on each supported platform.

`.github/workflows/ci.yml` runs `cargo check`, Clippy and tests with `--locked
--all-targets` on Linux, Windows and macOS. Release publication depends on that
workflow. Clippy warnings are currently reported rather than denied; a global
formatting gate is pending cleanup of the existing formatting baseline.

## Dependency Notes

The project uses:

- `eframe` and `egui` for UI.
- `git2` for repository operations.
- `rfd` for native file dialogs.
- `serde`, `toml`, and `dirs` for config persistence.
- `ureq` and `serde_json` for provider HTTP APIs.
- `tempfile` for isolated interactive-rebase todos and test repositories.
- `keyring` for native credential vaults, `fs2` for account-index locking, and
  `url` for credential destination parsing. Linux D-Bus is vendored at build time;
  a desktop Secret Service is still required at runtime.

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



## Extended fixtures and Linux GUI startup

```sh
# Requires git-lfs; creates only disposable local repositories/remotes.
cargo test --locked --lib lfs_stage_push_clone_round_trip -- --ignored

# Opt-in 100k-commit graph benchmark (reports timings, no machine-specific threshold).
cargo test --locked --lib benchmark_large_history_first_page -- --ignored --nocapture

# Requires Xvfb and libxkbcommon-x11-0; config/data/repo are isolated.
cargo build --locked
python3 scripts/gui_smoke.py target/debug/recorde /tmp/recorde-gui-artifacts
# Also requires libXtst; selects an image and checks both snapshot colors.
python3 scripts/gui_smoke.py target/debug/recorde /tmp/recorde-image-artifacts --image-preview
# Ctrl-selects two separated lines, stages them and checks actual index bytes.
python3 scripts/gui_smoke.py target/debug/recorde /tmp/recorde-selection-artifacts --line-selection
```

The GUI check verifies startup and captures a PNG plus logs for inspection.
The optional image case clicks a fixture row and checks rendered snapshot colors
in a fixed viewport. This does not prove general interactive workflows or native
Windows/macOS behavior. Linux CI
runs it and uploads artifacts. LFS has a separate Linux CI step.

New operations should use `mutation::spawn_operation` for scoped completion and
`process::git`/`process::run` for supervised CLI work. Do not mutate another
repository's active UI from a background completion. Native Git is used where
hooks, filters, credential helpers or sequencer behavior matters.

Additional dependencies: `command-group` for process groups/Windows jobs,
`notify` for filesystem events, `image` for bounded binary previews, and native Git/Git LFS for their respective
workflows. A per-file filter fixture checks actual index bytes.

Release tags must match the package version in Cargo.toml. Publishing downloads
only platform packaging artifacts and adds SHA256SUMS. These integrity hashes are
not signing or update authentication; signed distribution remains separate work.

## Security checks

Run `cargo audit --deny unsound` after dependency changes. CI installs cargo-audit
0.22.2 and runs this gate against the current RustSec database. Keep maintenance
warnings visible; the current paste/ttf-parser warnings are tracked in the gap
analysis, with no blanket advisory exclusions.

Treat remote responses, Git paths and commit metadata as untrusted data. Route
provider links through `external::open_url`, use argument arrays and literal Git
pathspecs, and validate exact OIDs where an operation expects a commit. App file
I/O uses `git::safe_path` checks; this is not a replacement for OS-level isolation
against a concurrent hostile local process. Bound decoded images and conflict
text before loading them into the UI. Security fixtures use disposable files and
local mock servers, never personal credentials or live provider writes.

## External editor

The editor setting accepts an executable and arguments, with quotes around paths
or arguments containing spaces. There is no shell expansion or redirection.
For example: `"C:\Program Files\Microsoft VS Code\Code.exe" --wait`.
Windows requires an .exe/.com, or an executable name found on PATH; .cmd/.bat
wrappers are refused so repository paths cannot become command-interpreter input.

## Repository sessions and recovery

Panel visibility and sizes are saved separately for each repository. In-memory
forms, conflict/rebase editors and provider forms follow their repository tab.
The Workspaces menu saves named sets of open tabs and opens favorite repositories.

Repository settings provide shallow-history expansion and cone-mode sparse
checkout. Changing sparse directories requires a clean index/worktree, including
untracked files. Root-level files remain in cone mode. Disable sparse checkout
before restoring a recovery snapshot.

Recovery includes the index and non-ignored tracked/untracked files, including
before the first commit. Subsequent edits invalidate automatic restore; each
successful restore saves an inverse snapshot. Ignored files are never captured.
`recorde.recoveryRetentionDays` is a local Git setting: 0 (default) keeps all
snapshots; the UI offers 30 or 90 days. Cleanup keeps the latest 20 snapshots,
pinned snapshots and incomplete operations. Deleting a snapshot removes its
protected ref; Git garbage collection later reclaims unreachable content.
Deleted branches also have protected recovery refs and survive restart.

Navigation (selected commit/file, graph search/filter/history extent and focused
pane) and split/search/whitespace preferences are saved per repository every two
seconds when changed and on normal exit. Missing commits or refs are discarded
when restoring; file contents and pending mutations are never restored from config.
Commit drafts are rate-limited to one save per 400 ms, with immediate saves on
submission, repository switch and shutdown.

Recovery also covers branch/detached checkout and app-started sequencer actions.
Checkout recovery refuses a moved original branch or one checked out elsewhere.
Continue/Abort/Skip finalize only a checkpoint matching native sequencer files and
HEAD; external sequencer changes leave a snapshot for manual recovery. Selected
range discard captures the confirmed diff and refuses changes before execution.
