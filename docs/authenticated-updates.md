# Authenticated application updates

Settings → Updates provides an explicit check, verified download, and installation confirmation. Automatic checks are off by default. Private releases require an explicitly selected saved GitHub account with access to `antonydoyle/Recorde`; its credential is sent only on the initial fixed GitHub API request. Redirect requests carry no credential. No other provider account is selected implicitly.

The release signing format and operator commands are in [the signing tool README](../tools/update-signing/README.md). The first updater-enabled release must be installed manually. Trust is bootstrapped from the public keys compiled into that installed application. Rotation requires shipping the new public key in an already trusted application release.

## Verification and storage

The client checks the Ed25519 signature over domain-separated original JSON bytes before parsing manifest fields. It then enforces the release repository, tag, asset name, platform, architecture, package kind, channel, time, size, and SHA-256. Downloaded keys are never trusted. The persisted highest accepted version and issuance time prevents replay across restarts and channel changes. Corrupt state is preserved and updates fail closed. This per-user state does not resist deletion or executable modification by the same operating-system user or an administrator.

Metadata and envelopes have explicit size limits. Requests have connection and idle deadlines; artifact streaming has an overall size-based deadline, at most two hours, and cancellation checked between reads. A blocked read ends at the ten-second idle deadline. Failed or cancelled downloads discard their temporary directory. Completed verified downloads remain available for manual installation and retry.

Private staging directories reject symlinks/reparse points. Each apply attempt uses a fresh directory with a copied, reverified artifact and signed envelope. The copied helper is created without overwriting an existing file and is bound to the captured running executable digest. Windows retains every directory ancestor with handles denying deletion while path-based replacement runs. Unix verifies ancestor ownership and writable modes and uses no-follow file opens.

## Installation behavior

| Package | Behavior |
| --- | --- |
| Windows portable | Replaces the marked executable with `ReplaceFileW`, preserves a verified backup, launches the new executable, and restores the backup if startup fails. |
| Windows MSI | Uses the fixed system Windows Installer with the verified MSI. An administrator prompt may be required. Installer failure or timeout never triggers automatic relaunch because installation state can be ambiguous. |
| Linux portable | Atomically replaces the marked executable, retains a verified backup, and rolls back when startup fails. |
| Linux AppImage | Requires the running executable to belong to the reported AppImage FUSE mount; an inherited `APPIMAGE` pointing at another application is refused. Replaces the image with backup/startup recovery. |
| Linux DEB | Uses only the protected installed `/usr/lib/recorde/recorde-update-installer` through `/usr/bin/pkexec`. The privileged helper independently verifies and copies the artifact into root-private storage before package-manager execution. If the helper is absent or unsafe, the UI offers the verified download for manual installation. |
| macOS | Downloads the signed universal DMG for manual installation. Automatic replacement is unavailable. |

Portable updates replace only the executable or AppImage. Use a full installer when adjacent bundled resources change. A valid installation marker is required for automatic portable/MSI/DEB handoff. Locations with unsafe ownership, writable permissions, hard links, or reparse points are refused.

AppImage detection follows the runtime's `APPDIR`/`APPIMAGE` contract and requires the mount source to match as an additional local identity check. Runtime variants that do not expose that identity are download-only. See the [AppImage environment documentation](https://docs.appimage.org/packaging-guide/environment-variables.html) and [type-2 runtime source](https://github.com/AppImage/type2-runtime/blob/main/src/runtime/runtime.c).

Before handoff, Recorde requires repository workers and clones to finish, rejects active merge/rebase/cherry-pick/revert/bisect sequences across open repositories, requires review/conflict editors to close, and saves current and inactive commit drafts plus navigation/configuration. Pending commits or any persistence error prevent restart. Other running Recorde instances prevent replacement. The helper waits for the exact parent process identity to exit; it never kills that parent.

Normal startup registers a shared instance guard even when automatic update checks are disabled. If that guard cannot be acquired because an update is active, or its storage is inaccessible/unsafe/full, startup stops with a native error dialog before opening user state. Allowing an unregistered instance would let another process replace its executable without detecting it. `--version` and the Git credential helper return before updater storage or GUI initialization.

The new portable process acknowledges renderer creation before loading or migrating user state and waits for the helper's continuation signal. Signals are flushed before atomic publication. Before continuation is published, a missing acknowledgment, early exit, or launch failure triggers backup verification and rollback. After continuation, automatic rollback is forbidden because new-version state may already have been written. Installer transactions use their package manager's recovery.

A durable pending-swap marker precedes executable replacement. If the helper is interrupted, the next normal launch reports the retained backup and attempt before loading user state; it does not blindly restore files. Successful startup or verified rollback clears the marker. A per-attempt outcome file is shown on the next launch. Completed attempts retain their receipts while duplicate artifacts/helpers are pruned only after their exact helper identity has exited; failed attempts remain available for recovery. MSI/DEB timeout messages require waiting for the installer before reopening Recorde.

## Validation limits

Local synthetic fixtures cover signature/tamper/replay guards, credential stripping on redirects, private-state corruption and lock contention, concurrent instance refusal, fresh retry staging, successful startup signaling, missing acknowledgment, child exit, executable rollback, and preservation of unrelated adjacent files/backups. Production hosted-provider writes are not used for validation. Actual Windows administrator prompt acceptance, macOS installation, and a signed production self-update require platform/release acceptance runs; fixture success does not establish those outcomes.
