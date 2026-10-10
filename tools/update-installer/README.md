# Privileged DEB update helper

This Linux-only binary is a separate, small package installer. It does not link
the GUI or expose a general command runner. The user-facing updater must already
have stopped repository operations, flushed drafts and closed Recorde before
requesting installation.

## Fixed interface

```text
/usr/bin/pkexec /usr/lib/recorde/recorde-update-installer --envelope /absolute/envelope.json --artifact /absolute/package.deb
```

These are the only arguments. There is no trust-key file, hash-only mode, target
directory override, alternate package command, test key, dry-run installation
mode or caller-provided installed version. Production public keys are compiled
from `packaging/update-keys.json`. The application must treat a missing helper or
unsafe helper path as **verified download only**, never as permission to invoke
`pkexec dpkg` directly.

Exit 0 means dpkg completed and its installed package record confirmed the exact
requested Debian version and native architecture. Any other result, cancellation
or missing response is unconfirmed: show the retained verified download and ask
the user to inspect package-manager status. Do not infer rollback or automatically
relaunch an old executable after an ambiguous privileged-install outcome.
The parent task deadline must leave room for metadata/copy preflight, the
900-second install and its 30-second termination grace (at least 1500 seconds
overall, plus any separately bounded administrator prompt wait).

## Security boundary

- The helper requires real and effective root credentials, a canonical nonzero
  `PKEXEC_UID`, and its exact installed executable path. Before privileged file
  changes, it checks that itself and each fixed package tool have root-owned,
  non-writable ancestor directories and safe regular-file metadata.
- Both caller files must be regular, singly linked files owned by `PKEXEC_UID`.
  Path traversal uses the caller's filesystem UID and primary group, restored
  and checked before privileged operations. Each component is opened with
  `O_PATH` and no-follow semantics against a retained parent descriptor. The
  final file must have owner-read permission. Only after checking its type and
  ownership is that retained descriptor reopened for reading and its identity
  rechecked; a device is never opened for I/O. No symlink or special file is
  accepted. Input paths may change without redirecting the retained descriptor.
  The app must write input files with explicit mode 0600, independent of umask.
- The helper verifies the signed envelope against compiled keys, Linux, native
  architecture, DEB kind, release URL, version and validity period. An independent
  version floor is compiled from the application's Cargo package version. The
  caller cannot weaken any of these checks.
- Signed bytes are copied from the opened artifact into a root-owned private
  directory under `/var/lib/recorde-update`, checked during copying, fsynced and
  independently hashed again before installation. dpkg only receives this
  private filename, never the user-controlled staging path. Source mutation
  either leaves the authenticated bytes unchanged or fails the digest check.
- Package metadata must name `recorde` and the native DEB architecture. Its
  upstream version must exactly match the signed SemVer, mapping prereleases to
  Debian `~`; a legitimate Debian revision is permitted. Prerelease identifiers
  containing a hyphen require an explicit Debian revision, as cargo-deb emits
  (for example `1.0.0~rc-1-1`). Epochs are refused.
  `dpkg --compare-versions` checks against the actual installed package, and
  `--refuse-downgrade --skip-same-version` enforce policy again under dpkg's own
  lock if another package manager races the preflight.
- A root-only lock and atomically persisted rollback checkpoint span channels.
  Corrupt or unsafe checkpoint files fail closed and are preserved. The new
  checkpoint is committed before installation; failed installs may retry that
  authenticated version but cannot silently reset the rollback floor.
  Stable and preview are both allowed when explicitly selected and signed;
  this is a root installation integrity boundary, not a second channel setting.
  Release signing timestamps must increase with release version across channels.
  After locking, only recognized stale root-private stages and checkpoint temp
  files are removed; unexpected contents or symlinks are preserved with an error.
- Fixed `/usr/bin/dpkg-query`, `/usr/bin/dpkg-deb` and `/usr/bin/dpkg` children
  receive a cleared environment, fixed PATH/locale/home, no stdin and bounded
  output/time. Install output is drained while retaining a bounded diagnostic
  tail; verbose output never aborts dpkg. Cancellation and the 900-second install
  deadline send SIGTERM, allow 30 seconds to finish, then enforce cleanup. A forced
  termination can leave package state incomplete; its diagnostic explicitly
  requires checking dpkg/package-manager repair instructions before retrying.
  No shell is used. Signal handling, process groups and Linux subreaper cleanup
  cover descendants that create a new session. Task PIDs and cleanup are reported.

Administrator policy and the signed Recorde release are trust boundaries:
authenticated DEB maintainer scripts and system dpkg configuration retain their
normal privileges. This helper is not a sandbox for a compromised signing key or
administrator. Filesystem I/O can still be delayed by a stalled kernel/filesystem;
timeouts are cooperative around reads and hard-bounded around package children.

## Package integration (owned by the release pipeline)

Build this crate with the same source version and production key file as Recorde:

```text
cargo build --release --locked --manifest-path tools/update-installer/Cargo.toml
```

Install the resulting binary as root:root mode 0755 at
`/usr/lib/recorde/recorde-update-installer`. Install
`packaging/org.recorde.update.policy` as root:root mode 0644 at
`/usr/share/polkit-1/actions/org.recorde.update.policy`. No setuid mode or sudoers
entry is used. The polkit action requires fresh administrator authentication for
an active local session; inactive/other sessions are refused. Include `pkexec`
as a package dependency if automatic DEB application is advertised. Private
state is created mode 0700 only after an authenticated request; it must survive
ordinary package upgrades. Do not remove/reset it on a failed installation.

## Validation and limits

Local tests must use the repository's supervised Linux task wrapper, never sudo,
pkexec or package installation. They exercise only pure policy, temporary files,
read-only package version queries/comparisons and an owned child-tree fixture.
Production verification intentionally rejects test signing keys. No configurable
test trust bypass is compiled into the installed helper.

`scripts/linux_update_installer_acceptance.py` provides a separate disposable
GitHub-hosted acceptance path. It requires a checksum-verified previous DEB,
candidate DEB and binary, production helper, signing CLI, and fresh external
artifact directory. Invoke it under sudo through the root Linux task wrapper,
preserving `GITHUB_ACTIONS`, `RUNNER_ENVIRONMENT`, `GITHUB_WORKSPACE`, `RUNNER_TEMP`
and the normal `SUDO_UID`. It refuses existing packages, helper or updater state.
Run ordinary installer acceptance first and let it remove its package; existing
user-data fixture sentinels are preserved.

The script builds copied, byte-identical runtime source under the runner UID,
changing only its disposable compiled key and prior version floor. It proves the
production helper rejects that key, then tests a real signed upgrade, invalid
envelope/architecture/package/version/expiry/digest rejection, corrupt checkpoint
preservation, concurrent lock refusal, safe stale-stage cleanup, and input path
replacement. Fixture keys remain in a private temporary directory that is
deleted, never uploaded. Child commands have purpose/deadline/cleanup ledgers.
The candidate must replace the fixture helper with the packaged production
helper; the script checks the installed binaries and checkpoint then removes
only its owned package/state. None of these privileged tests runs locally.

Headless acceptance supplies `PKEXEC_UID` after sudo authorization; it does not
claim to exercise the desktop polkit prompt or user cancellation in that prompt.
Ordinary installer acceptance separately checks the packaged polkit policy and
root ownership. Desktop prompt/cancel acceptance remains a separate check.

References: [polkit pkexec security and action binding](https://polkit.pages.freedesktop.org/polkit/pkexec.1.html),
[dpkg options and downgrade policy](https://manpages.debian.org/bookworm/dpkg/dpkg.1.en.html),
[dpkg-deb metadata queries](https://manpages.debian.org/bookworm/dpkg/dpkg-deb.1.en.html).
