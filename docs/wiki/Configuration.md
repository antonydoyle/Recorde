# Configuration

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
- `default_editor`: external editor command.
- `terminal`: external terminal command.
- `diff_tool`: external diff tool command.
- `merge_tool`: external merge tool command.
- `signoff_default`: default commit sign-off behavior.
- `protected_branches`: defaults to `main` and `master`.
- `pull_strategy`: `Merge`, `Rebase`, or `FfOnly`.
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

## Accounts and Tokens

Non-secret provider account records are stored in `config.toml`. Each account
contains:

- Provider type.
- Host.
- Username.

Supported provider kinds are:

- GitHub.com.
- GitHub Enterprise.
- GitLab.com.
- Self-hosted GitLab.

Tokens are intentionally kept separate from the main config file. They are stored
under the platform data directory:

```text
<data-dir>/recorde/tokens.toml
```

On Unix, Recorde attempts to set this token file to mode `0600`.

Important: tokens are currently stored in plaintext in a user-readable local file.
The source includes a note that a future hardening step should replace this with
an OS keyring-backed credential store.

## Remote Credential Lookup

For HTTPS remote operations, Recorde can look up a token by host. It accepts exact
host matches and suffix matches for enterprise-style hosts. The stored username is
used as the HTTP username and the token is used as the password.

