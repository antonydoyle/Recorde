# Configuration

## Commit identity and signing

Use **Identity / signing** in the commit form, or **Repository > Repository
Settings**, to set your name and email and configure commit signatures. Empty
identity fields and control characters are rejected. Missing-identity errors
point back to these settings and preserve the commit draft.

Commit signing supports Git's OpenPGP, SSH and X.509 formats, with a key ID,
public key, or existing key path. Signing operations use your installed signing
tool and agent, which handle any passphrase prompts. The separate SSH key
generator accepts an optional encryption passphrase, processes it in the app,
and does not persist the passphrase. Signing failures
retain the draft and never trigger an unsigned retry. A blank OpenPGP/X.509 key
removes this scope's key override so Git can use its inherited/default key.
Saving uses the current worktree's settings when `extensions.worktreeConfig`
is enabled, otherwise the shared repository's settings. Executable signing
remains deferred independently of commit signing.

See [Git signing configuration](https://git-scm.com/docs/git-config#Documentation/git-config.txt-gpgformat).

## History optimization

Under **Repository Settings > Checkout and history**, **Optimize history loading**
builds Git's commit-graph index. This only writes history acceleration metadata;
branches, staged changes and working files are preserved. It runs as a queued,
cancellable operation. Shallow repositories must fetch full history first;
replacement refs and a disabled `core.commitGraph` setting prevent optimization.

When an index is available, Recorde uses bounded native Git pages and caches the
loaded commits. Repositories without an index use the retained libgit2 traversal.
Building an index can take time on a large repository; repeat it after substantial
new history arrives. See [Git commit-graph](https://git-scm.com/docs/git-commit-graph).

## Preferences file

Recorde stores user preferences as TOML under the platform configuration
directory:

```text
<config-dir>/recorde/config.toml
```

The path is resolved with the `dirs` crate. On typical Linux systems this is
`~/.config/recorde/config.toml`.

## Application Settings

The `Config` model contains:

- `recent_repos`: up to ten recently opened repositories.
- `dark_theme`: enabled by default.
- `high_contrast`: high contrast UI mode.
- `motion`: `system` (default; follows the OS animation setting on Windows), `full` or `reduced`.
- `default_editor`: external editor command.
- `terminal`: external terminal executable and optional literal arguments.
- `diff_tool`: Git diff tool name; falls back to Git's `diff.tool` or `diff.guitool`.
- `merge_tool`: Git merge tool name; falls back to Git's `merge.tool` or `merge.guitool`.
- `signoff_default`: default commit sign-off behavior.
- `protected_branches`: defaults to `main` and `master`.
- `pull_strategy`: `GitConfig` (default), `Merge`, `Rebase`, or `FfOnly`. Existing saved choices
  are preserved. "Use Git configuration (recommended)" honours `branch.<name>.rebase` before
  `pull.rebase`, then `pull.ff` when rebase is disabled. `pull.ff=only` refuses divergence;
  `pull.ff=false` creates a merge commit even when a fast-forward is possible. Explicit app
  strategies override these Git pull settings.
- `fetch_prune`: fetch pruning preference.
- `auto_fetch` and `auto_fetch_interval_secs`.
- `font_scale`.
- `colorblind_graph`.
- `show_sidebar`, `show_right_panel`, and `show_diff_panel`.
- Panel sizing fields for sidebar, right panel, and diff panel.

## Per-Repository Settings

Recorde stores commit graph column widths per repository path:

- refs column width.
- graph column width.
- time column width.
- hash column width.

These are keyed by the repository path string.

## Accounts and tokens

Account metadata is stored in `config.toml`; provider tokens are stored in the
operating system credential vault. Legacy plaintext tokens are migrated only
after a successful vault write. A locked or unavailable vault produces a
retryable error. Linux needs a desktop Secret Service; Windows uses its native
credential store. macOS validation is parked.

HTTPS credential lookup binds the exact normalized host and port, repository
account selection, and username. Provider requests refuse redirects and redact
tokens from diagnostics. Native Git network operations use Recorde's credential
helper, with configured Git helpers available as fallback. SSH continues to use
Git's installed SSH client and agent.

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


### Keyboard shortcut overrides

`shortcuts` stores overrides keyed by command identity. Each value contains `key`, `shift`, and `alt`; the platform command modifier is always required (Control on Windows/Linux, Command on macOS). An absent entry uses the command default, and an empty key disables it. Settings > Keyboard validates conflicts and restores defaults. Unknown command identities are retained for compatibility but never dispatched; conflicting bindings in a hand-edited file do not execute either command.
