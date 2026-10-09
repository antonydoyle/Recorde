# Troubleshooting

## The App Opens the Wrong Repository

If no CLI path is provided, Recorde opens `active_repo`, falling back to the first entry in `recent_repos`.
Open the intended repository from the toolbar or edit the config file at:

```text
<config-dir>/recorde/config.toml
```

## Provider Panels Do Not Load

Check that:

- The repository has a GitHub or GitLab remote URL in a supported format.
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

Recorde embeds Material Icons and attempts to load Noto Symbols from:

```text
/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf
```

If the system font is absent, some non-Material symbols may render differently.
Install the Noto Symbols package for your platform if needed.

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

Open Repository Settings > Recovery snapshots. Restore requires the same recorded
branch and post-operation HEAD, no active Git sequence, and no current changes
(including untracked files). Commit/stash current work before restoring.
Snapshots cover tracked files and the index. General redo, unborn commits and
untracked-file recovery are not implemented.

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
