# Provider Integrations

Recorde has provider-neutral models for repository metadata and provider-specific
adapters for GitHub and GitLab.

## Supported Providers

- GitHub.com.
- GitHub Enterprise.
- GitLab.com.
- Self-hosted GitLab.

The account model stores provider kind, host, and username. Tokens are retrieved
from the token store when provider panels or HTTPS remote operations need them.

## Remote Parsing

Provider detection starts from the Git remote URL. Supported forms include:

```text
git@github.com:owner/repo
https://github.com/owner/repo
https://gitlab.com/group/project
```

The parser extracts owner or namespace, repository/project name, and host.

## Pull Requests and Merge Requests

Recorde models GitHub pull requests and GitLab merge requests as `PrItem`:

- Number.
- Title.
- Author.
- State.
- Head and base refs.
- URL.
- Last updated timestamp.
- Labels.
- Optional check state.
- Optional review state.
- Body.
- Comment count.
- Optional changed file count.

Filters support open, closed, and all.

## Issues

Issues are represented as provider-neutral `IssueItem` values with number, title,
author, state, URL, labels, body, comment count, and update timestamp.

Filters support open, closed, and all.

## CI Runs

CI runs are represented as `CiRun`:

- ID.
- Name.
- Status.
- URL.
- Branch.
- Last updated timestamp.

Statuses are normalized to success, failure, running, pending, or cancelled.

## Releases

Releases are represented as `ReleaseItem`:

- Tag name.
- Release name.
- Body.
- URL.
- Published timestamp.
- Prerelease flag.

## Implementation Notes

The provider modules use `ureq` and `serde_json` directly. Timestamps are parsed
from ISO-8601 strings into Unix timestamps by the shared provider module.

Provider loading is initiated from `git/operations.rs`, which checks the current
repository remote, finds a matching account/token, calls the relevant adapter,
and updates `AppState`.

