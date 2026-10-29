# Developer Guide

## Current local validation

macOS implementation and native acceptance are parked by request. Windows and
Linux remain the local validation targets; existing hosted macOS definitions
remain available. Executable signing is separately deferred.

Initial local checks and CI use `CARGO_BUILD_JOBS=1` and `RUST_TEST_THREADS=1`.
Windows validation exposed two console-launch bugs: `command-group` replaces
`Command` creation flags, and the GUI subsystem attribute also applied to binary
test harnesses. Set `CREATE_NO_WINDOW` on the group builder and exclude tests
from the subsystem attribute. Regression fixtures check the actual console
handle, test executable header, and grandchild termination.

### Windows process safety

Run Windows builds and tests through the bounded task wrapper. Start with the
source guard, cleanup fixture, compilation, then the process fixtures. Do not
repeat a full suite after a timeout or unexpected console window.

```powershell
$env:CARGO_BUILD_JOBS = '1'
$env:RUST_TEST_THREADS = '1'
python scripts/check_process_safety.py
./scripts/windows_task_selftest.ps1 -Artifacts "$env:TEMP/recorde-process-safety"
./scripts/windows_task.ps1 -Executable cargo -Arguments @('check', '--locked', '--all-targets') -Purpose 'Compile safety changes' -Artifacts "$env:TEMP/recorde-process-safety" -TimeoutSeconds 180
./scripts/windows_task.ps1 -Executable cargo -Arguments @('test', '--locked', '--lib', 'git::process::tests', '--', '--test-threads=1') -Purpose 'Process regression fixtures' -Artifacts "$env:TEMP/recorde-process-safety" -TimeoutSeconds 180
./scripts/windows_task.ps1 -Executable cargo -Arguments @('test', '--locked', '--bin', 'recorde') -Purpose 'Test executable subsystem' -Artifacts "$env:TEMP/recorde-process-safety" -TimeoutSeconds 180
```

Use an installed Python interpreter (or `python3` in WSL), not the Windows Store
alias. A cold build may need a longer explicit deadline after reviewing its log.
The wrapper creates a private Windows job before releasing the worker, limits it
to 32 processes and 4 GiB by default, captures output, and stops the owned job on
success, failure, timeout or console detection. Closing the wrapper also closes
the job. Each run records its purpose, PIDs, parent/command-line ownership and
listening ports in an external artifact directory. `result.json` reports cleanup;
`stdout.log` and `stderr.log` hold command output. Arguments and command lines are
recorded: never pass credentials in arguments.

New Git/tool window titles stop the task for investigation. Titles alone do not
establish ownership, so the wrapper reports such windows without terminating a
shared Windows Terminal host. Preserve unrelated services. CI retains the ledgers.
The timeout fixture verifies an owned grandchild's TCP listener is released.

Local Windows verification (2026-10-09): the timeout/listener fixture, source
guard, `cargo check --locked --all-targets`, and all seven binary process-safety
tests passed. Cleanup reported zero task workers, task listeners and new tool
windows. The full suite was not rerun during this safety change; earlier aborted
Windows suites are not successful acceptance runs.

All unattended Git/SSH probes and test helpers use `background_command` and the
supervised runner with a deadline. Credential-helper input stays in pipes;
process logs omit arguments and input. Settings diagnostics run once per opening
on a background worker and cache their results. User-opened editors and terminals
remain user-owned.

### Linux process safety

Use `python3 scripts/linux_task.py --purpose 'Tests' --artifacts /tmp/recorde-checks
--timeout 300 -- cargo test --locked --all-targets` (on one line). This runner
uses Linux child adoption to track orphaned descendants, including tools that
start a separate session. Cleanup checks PID identity before signalling and
records owned processes and TCP/UDP listeners. Restricted `/proc` inspection is
reported explicitly. It never stops a process based on its name alone.

`python3 scripts/linux_task_selftest.py /tmp/recorde-runner-tests` verifies deadline
cleanup, early parent exit, and protected child processes. Each fixture starts
its own ephemeral TCP listener and confirms that cleanup releases it. Logs are
external to the repo. Linux CI uses this runner for validation and GUI fixtures.

Additional opt-in fixtures:

```bash
cargo test --locked --lib ssh_signed_commit_verifies_with_disposable_key -- --ignored
RECORDE_PERF_COMMIT_GRAPH=1 RECORDE_PERF_COMMITS=1000000 cargo test --locked --lib benchmark_large_history_first_page -- --ignored --nocapture
python3 scripts/gui_smoke.py target/debug/recorde /tmp/recorde-settings-artifacts --repository-settings
```

Signing uses a newly generated temporary SSH key and verifies both the commit and
amend. It requires `ssh-keygen` and never uses personal keys. The indexed history
fixture reports index-build time separately, then checks native paging, cached
metadata and paging across the 4,096-ID response boundary. Settings mode captures
the repository menu, top of Settings and scrolled signing controls for visual
inspection; it does not modify signing configuration.

## Shared Help documentation

The offline Help browser embeds the user-facing pages in `docs/wiki` at compile
time. Edit these Markdown sources to update both the repository wiki and the
app. Register a new in-app topic in `PAGES` in `src/ui/help.rs`. The
`ui::help::tests::embedded_pages_have_unique_names_and_resolvable_wiki_links`
source check verifies unique topic IDs and local wiki link targets; the remaining
`ui::help::tests` cover search, navigation, rendering and readable topic/article
geometry. Run these tests through the platform's supervised validation wrapper
described above. Native inspector `--review` additionally opens Help pages and
captures them for visual inspection.

## Prerequisites

- Rust 1.95 or newer with Cargo.
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

Local test handoff: build `cargo build --locked --bin recorde` through the platform
runner, copy the executable and `LICENSE` to an ignored `dist/` package, and smoke
test that copy with an isolated repository/configuration. Include the source
commit, build profile, platform requirements and SHA-256 checksums. Debug builds
are for functionality testing; release-performance measurements require a release
build. Do not describe a portable test build as installer or upgrade acceptance.

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

`.github/workflows/ci.yml` runs `cargo check`, Clippy and tests with `--locked`
on Linux, Windows and macOS. Release publication depends on that workflow.
The executable imports the library; it no longer declares a second copy of the
application modules or duplicates their tests. Windows also checks the executable
test harness subsystem. CI enforces `cargo fmt --all --check`. Clippy warnings
are reported rather than denied; remaining style warnings are reported.

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

The UI uses upstream `eframe`/`egui` 0.36.2 with the native AccessKit bridge
explicitly enabled. Font rendering uses `skrifa` and `vello_cpu`; the bundled
Inter, Noto Symbols and Material Icons assets and logical sizes remain the same.
The old vendored eframe patch has been removed. The native app creator makes the
window visible before rendering begins, preserving the early visibility needed
by compositors that wait for a mapped surface during the first EGL buffer swap.

The optional `ui-inspection` feature is a development tool. Build the app and
`inspect_ui` example with that feature, then run `gui_smoke.py` through the task
supervisor with `RECORDE_UI_INSPECTOR` set to the example binary. The inspector
asserts meaningful AccessKit labels, selected commit state, Tab focus changes and
arrow navigation, and writes the tree and native framebuffer to the external
artifact directory. `--dpi=1`, `1.25`, `1.5` and `2` vary the X11 native monitor
scale; `--light` chooses the other theme. The fixture checks reported density and
screenshot dimensions. `wayland_smoke.py` uses an existing compositor with X11
fallback disabled to check that a first frame is rendered. Production builds do
not enable the inspector feature or its listener.

For design review, set `RECORDE_UI_TOUR=1` when running the inspector (for example
through `windows_smoke.ps1 -Inspector ...`). Instead of acceptance checks it captures
the main window, working-tree and file selection, batch selection, a selected commit,
every top-level menu and every Settings page (with scrolled captures) into the artifact
directory, and writes `tour.log` listing any step that could not be reached.

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

Additional GUI coverage: `scripts/gui_smoke.py <binary> <artifacts> --split-selection`
selects separated lines in the split renderer and verifies the exact resulting
index bytes and unchanged working file. CI runs it alongside unified selection
and the image snapshot fixture. Recovery fixtures now cover unborn HEAD,
untracked files, renames, ignored collisions, retention, and deleted branches
across Git garbage collection. Sparse fixtures use native Git status semantics.

Native Windows validation: invoke `scripts/windows_smoke.ps1 -Binary <exe> -Artifacts <directory>`
through `windows_task.ps1` with PowerShell as its executable, as CI does.
This creates a disposable repo, isolates Git and application configuration, starts
the app hidden, checks a native window exists, retains logs, and stops only its
own process. CI runs this, LFS and the isolated Windows vault round trip. This
verifies startup, not rendered pixels or interaction.

On Windows, absolute `APPDATA` overrides relocate config, account metadata and
crash reports. Native credential-vault, repository and updater coordination paths
remain shared between profiles, so changing the override cannot bypass their
cross-process locks. The startup smoke uses empty private profile storage, refuses
enabled automatic update checks or pending update recovery, and does not exercise
real provider accounts. It still opens native coordination locks; a separate OS
user or disposable runner is required for complete isolation.

History benchmarks accept `RECORDE_PERF_COMMITS=1000000` and report cold page,
next-page, edge-query and graph UI+tessellation timings. Full native app/GPU
latency is not measured by that fixture. No hosted CI run is claimed locally.

`scripts/gui_smoke.py <binary> <artifacts> --frame-profile` builds a disposable
100,001-commit workspace and sets `RECORDE_PROFILE_FRAMES=1`. After 20 warmup frames,
the app logs 220 eframe CPU-frame samples, including app update and native rendering
but excluding vsync waiting and asynchronous GPU completion. The normal app does
not force continuous repaint or record these samples. CI retains the result log.
