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

## Security boundary

- The helper requires real and effective root credentials, a canonical nonzero
  `PKEXEC_UID`, and its exact installed executable path. Before privileged file
  changes, it checks that itself and each fixed package tool have root-owned,
  non-writable ancestor directories and safe regular-file metadata.
- Both caller files must be regular, singly linked files owned by `PKEXEC_UID`.
  Each pathname component is opened with no-follow semantics against a retained
  parent descriptor; no symlink or special file is accepted. Input file paths
  may change after opening without redirecting the retained descriptor.
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
  Debian `~`; a legitimate Debian revision is permitted. Epochs are refused.
  `dpkg --compare-versions` checks against the actual installed package, and
  `--refuse-downgrade --skip-same-version` enforce policy again under dpkg's own
  lock if another package manager races the preflight.
- A root-only lock and atomically persisted rollback checkpoint span channels.
  Corrupt or unsafe checkpoint files fail closed and are preserved. The new
  checkpoint is committed before installation; failed installs may retry that
  authenticated version but cannot silently reset the rollback floor.
- Fixed `/usr/bin/dpkg-query`, `/usr/bin/dpkg-deb` and `/usr/bin/dpkg` children
  receive a cleared environment, fixed PATH/locale/home, no stdin and bounded
  output/time. No shell is used. Process groups and Linux subreaper cleanup cover
  descendants that create a new session. Task PIDs and cleanup are reported.

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

Hosted acceptance must separately validate actual DEB contents and root ownership,
polkit policy installation, cancellation, a legitimate signed upgrade, invalid
signature/architecture/package/version rejection, input replacement while the
admin prompt is open, private-stage hash rejection, concurrent invocation and
post-install package state. No local root/package installation acceptance is
claimed by the unit fixtures. Packaging and production signing are integrated
outside this crate.

References: [polkit pkexec security and action binding](https://polkit.pages.freedesktop.org/polkit/pkexec.1.html),
[dpkg options and downgrade policy](https://manpages.debian.org/bookworm/dpkg/dpkg.1.en.html),
[dpkg-deb metadata queries](https://manpages.debian.org/bookworm/dpkg/dpkg-deb.1.en.html).
