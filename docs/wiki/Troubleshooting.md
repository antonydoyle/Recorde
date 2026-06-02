# Troubleshooting

## The App Opens the Wrong Repository

If no CLI path is provided, Recorde opens the first entry in `recent_repos`.
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

Tokens are stored in:

```text
<data-dir>/recorde/tokens.toml
```

## HTTPS Remote Operations Fail

For authenticated HTTPS remotes, Recorde looks up a token by host. Confirm the
remote host matches the stored account host. Enterprise hosts can match by suffix,
but mismatched custom host names will not resolve to a token.

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

