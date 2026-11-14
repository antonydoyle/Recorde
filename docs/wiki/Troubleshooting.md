# Troubleshooting

## Offline support report and local logs

Open **Help > Offline support report** to generate and preview an offline report. Review the exact text before choosing Copy preview or Save preview. Nothing is uploaded. Network reachability is not checked. The report includes numeric Git/LFS versions, supported CLI capabilities, a safe representation of Git's location, bundled libgit2, compiled keyring backend, updater state and public update-key IDs. Custom Git paths are omitted. Keyring availability is explicitly **not probed**; diagnostics never read credential values.

For a report without opening the app window, run `recorde --diagnostics` to print the preview, or `recorde --diagnostics <new-output-file>` to save it locally. The CLI reports the updater as not running; it does not start its controller or run update cleanup. Use a new filename: existing files are preserved.

Local logs contain only fixed event codes and timestamps. Arbitrary application/dependency log messages, raw errors, Git output, private paths, remote addresses, credentials, file contents and panic payloads are excluded, including when `RUST_LOG` is set. Three event files are capped at 64 KiB each, plus an empty coordination file. Events may be dropped when the bounded queue or another writer is busy; the report includes the dropped count. An unwritable log folder does not prevent Recorde from running. Open local log folder in the report window to inspect these files.

Rust panic events are best effort and omit the payload and source location. This is not native crash capture; immediate aborts may leave no panic event. Earlier `crash-*.txt` reports are not read, attached or automatically removed.

## The App Opens the Wrong Repository

If no CLI path is provided, Recorde opens `active_repo`, falling back to the first entry in `recent_repos`.
Open the intended repository from the toolbar or edit the config file at:

```text
<config-dir>/recorde/config.toml
```

## Provider Panels Do Not Load

Check that:

- The repository has a supported GitHub, GitLab, Bitbucket Cloud or Azure DevOps Services remote URL.
- A matching account exists for the remote host.
- A token exists for that host and username.
- The token has permission to read the requested PR/MR, issue, CI, or release
  data.

Tokens are stored in the OS credential vault. Unlock it before retrying. On Linux,
check that a Secret Service is running in the desktop session. If migration failed,
use **Retry credential migration** in Accounts & SSH. The old `tokens.toml` file
is retained until migration succeeds, then contains account identities only.

## HTTPS Remote Operations Fail

Confirm the exact remote host and port match the saved account. Select the desired
account for the current repository in Accounts & SSH when there are multiple
accounts on that host. A username in the remote URL must agree with that selection.
Clone has no repository binding yet; use `https://username@host/owner/repo.git`
to select an account. HTTP remotes cannot use saved tokens.

Native Git network operations share Recorde's credential helper. If no saved
account matches, check Git's configured credential helper. An ambiguous saved
account selection deliberately stops authentication; choose an account first.

## Merge or Rebase Controls Appear

Recorde detects in-progress Git operations from repository state. If a merge,
rebase, cherry-pick, revert, or bisect was started outside the app, Recorde will
surface that state after refresh. Complete, abort, or reset the operation with
the appropriate Git workflow.

## Fonts Show Missing Symbols

Recorde embeds Inter, Material Icons, Noto Symbols and monochrome Noto Emoji.
No system font installation is required. If a symbol is missing, include the
symbol and a screenshot with the app version, platform and display scale in the
bug report. The font coverage test checks the bundled UI symbol repertoire.

## External Tools Do Not Open

Check the configured terminal, diff tool, merge tool, or editor command in
settings. Tool launch behavior is platform-specific and depends on the command
being available on `PATH` or configured with an absolute path.

## Git LFS Is Not Detected

Recorde detects Git LFS support from the local environment and repository
metadata. Confirm `git-lfs` is installed and that the repository has LFS tracking
configuration.

## Build Fails in the UI Stack

Confirm that platform graphics and windowing dependencies required by eframe are
installed. Linux builds commonly need OpenGL, GTK, GLib, fontconfig, and FreeType
development/runtime packages.



## External edits or older history are missing

Working changes refresh through native filesystem notifications, with focus and
30-second fallback refreshes. History starts with 2,000 commits; use Load More for
older entries. Search and current-branch ancestry cover the loaded history.

## Restore a reset or discarded tracked edit

Open Settings > Repository > Recovery snapshots, or use Undo last operation.
Restore checks the recorded branch, HEAD and exact post-operation file/index
state. Preserve newer edits before restoring; mismatching state or paths that
would be overwritten are refused. Snapshots cover the index and non-ignored
files, including untracked files and the state before the first commit.
Successful restores save an inverse snapshot for Redo undone operation.
See [Recovery](Recovery.md) for retention and limitations.

## Partial staging is unavailable for an LFS/filtered file

Stage or unstage the complete file. Native Git then runs its clean filter and
preserves the index representation. Install/configure Git LFS for LFS repositories.

## Linux startup reports a missing X11 library

Install `libxkbcommon-x11-0` alongside the documented graphics dependencies.
The Debian package declares this dependency; portable packages rely on system
libraries. The Linux CI smoke script starts the app under authenticated Xvfb.

## Cancelled operations and recovery

Cancel stops queued local work or terminates running Git commands and their child
processes. Completed writes remain. Inspect status and any active sequence before
retrying; Git may leave a lock file after forced termination. Only remove such a
lock after confirming no Git process is still using the repository.

Recovery snapshots check HEAD, branch and exact tracked index/worktree state.
Later edits and untracked/ignored-path collisions are refused. Each successful
restore creates an inverse snapshot for redo. Delete old snapshots explicitly
from Repository Settings when their recovery data is no longer needed.

Unfinished conflict edits are saved per worktree and restored when the same
conflict is reopened. A changed conflict requires explicitly recovering the older
draft. Draft save failures appear in the conflict editor.

## My commit uses the wrong identity

Check the repository's Git name/email override as well as the global identity.
A repository override wins over a global profile change. Saving a profile does
not apply it: choose Use globally or Use for this repository. Profile account
selections route to saved accounts per exact host; repository selections override
global defaults. Reconnect a missing saved account before applying its profile.
Profiles do not switch SSH sessions. A blank signing key leaves an existing key unchanged.

## The terminal command needs input or appears stuck

The integrated terminal runs noninteractive commands with no stdin and with Git
prompts disabled. Use Stop, then run an interactive program in the external
terminal. Each command starts a fresh shell in the repository folder. Output is
bounded, so long lines and old output may be shortened or removed. Clear only
clears visible output; Hide leaves commands running. Stop terminates the current
tab's command, and Close tab asks before stopping it. All tabs lists tabs in
other repositories. If all eight slots are in use, close an unneeded tab before
creating one for another repository.

## Git Flow finish stopped midway

Inspect the operation banner and resolve/commit any merge conflict. Then reopen
Git Flow and choose Finish again. Already integrated branches are skipped. A
release tag collision is refused if the existing tag identifies another commit.
Finishing can perform several local operations, so inspect both production and
development before retrying. Branches and tags are never pushed automatically.

## The file editor refuses to save or the app will not close

The editor checks that disk contents have not changed since the file was opened
or saved. Preserve your buffer before choosing Reload to read changes made by
another tool. A dirty buffer requires Discard and reload confirmation; Keep
editing cancels it. A failed reload preserves the buffer and reports the error.
Only UTF-8 text files up to 2 MiB are supported. Binary files need an external
editor. Save or explicitly discard unsaved editor buffers before closing the app
or restarting for an update, including buffers from earlier repositories.

## A shortcut does nothing

Close dialogs and move focus out of text inputs. Check Settings > Keyboard for
the effective binding, then use the command palette to see whether the operation
is unavailable because of repository state. Ctrl+F searches commits; Find in diff
is a separate command. Help also pauses repository shortcuts while open.

## Report a reproducible problem

Include the app version from Settings > About, operating system, display scale,
the action taken, the exact error and whether it reproduces in a small sample
repository. Remove tokens, private keys, private repository URLs and sensitive
file contents from logs or screenshots. Distinguish local Git failures from
provider/network failures and include the relevant provider type.
