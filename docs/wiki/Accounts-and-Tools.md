# Accounts, identities and tools

## Commit identity and profiles

Open Settings > Accounts & SSH to configure your Git name and email and save named profiles such as Work or Personal. A profile contains a commit name, email, optional signing key, saved provider-account selections and optional appearance/tool preferences.

Choose Use globally to change your global Git identity and default provider-account selections. If the profile includes preferences, it also restores the captured palette, light/dark mode, high contrast and external editor, terminal, diff and merge tool commands. Choose Use for this repository to set its identity and provider-account overrides without changing the global appearance/tools. Existing repository overrides take precedence over global defaults.

When both `~/.gitconfig` and the XDG Git configuration exist, global identity changes write to `~/.gitconfig`, which Git reads with higher precedence. An installation using only the XDG file keeps using it. Repository-local identity still takes precedence over either global file.

In Add or update a profile, select an existing account for each exact host, or leave automatic selection. Include current colours and external tools when used globally captures those preferences; Capture current preferences updates that snapshot. Save Profile stores the selection but does not apply it. A selected account must still be connected when the profile is applied.

Profiles route to saved accounts; tokens remain in the OS vault. They do not create separate credential stores or SSH sessions. Older identity-only profiles remain supported. Applying a profile replaces the account bindings at its chosen scope; hosts without an explicit selection use the normal account-routing rules. Deleting a saved profile does not erase an identity already written to Git configuration. A blank signing key leaves the existing key unchanged; applying a key does not itself enable commit signing.

Commit options offers sign-off, co-authors and identity/signing controls. Sign-off adds a trailer to the message; it is separate from a cryptographic commit signature. Signing requires the appropriate installed signing tool and key configuration.

## Provider accounts

Accounts & SSH supports GitHub, GitHub Enterprise, GitLab.com, self-hosted GitLab, Bitbucket Cloud and Azure DevOps Services. Add the account for the remote's exact host and port. GitHub/GitLab browser sign-in requires a registered public client ID; token entry remains available.

Tokens are stored in the operating system credential vault. Unlock it before using remote/provider features. Linux requires a running Secret Service. If credential migration failed, Retry credential migration preserves the old token file until migration succeeds.

When several accounts share a host, choose the desired account for the current repository. A username in the remote URL must match that selection. Clone has no existing repository binding: an HTTPS URL with a username can select the account. Saved tokens are used only for HTTPS, not unencrypted HTTP.

Provider panels offer pull/merge requests, issues, CI runs and releases according to provider capabilities. Bitbucket issues/releases and in-app merge are unavailable. Azure Boards and classic releases are project-scoped. Read [Provider integrations](Provider-Integrations.md) for permissions, individual actions and limits.

## SSH

The SSH section displays detected key and agent information, lets you refresh diagnostics, generate a key and copy its public key. Each supported saved account has an Add to button showing its account label; choose that button to upload the public key to that account. The provider's Open SSH keys page button is also available where a settings URL is supported, including as a fallback after an upload refusal. Keep the private key private. Diagnostics are cached and refreshed on request rather than probed continuously.

GitLab uploads support both token accounts and browser sign-in accounts using Bearer authentication with `api` scope. GitHub browser sign-in does not request SSH-key write permission. Use Copy and the GitHub SSH keys page to register the public key, or connect a token with `write:public_key` for a classic PAT, or Git SSH keys write permission for a fine-grained token. Recorde reports permission failures without automatically broadening sign-in scopes or retrying the upload.

Authentication using SSH depends on your SSH configuration and agent. A commit profile is not an SSH identity switch. Use an external terminal for interactive SSH setup or prompts that cannot be completed in the app.

## Integrated terminal

Toggle integrated terminal from the command palette or Ctrl+Backtick. The Terminal commands panel keeps separate command tabs for each repository or worktree. New tab creates another tab for the current repository, with up to eight tabs across the app. Each tab retains its own input, command history and output during the app session. Switching repositories selects that repository's tabs; commands already running keep their original directory.

Each command runs in the tab's repository folder in a fresh shell: cmd.exe on Windows, sh on Linux/macOS. A tab runs one command at a time and streams standard output and errors. Other tabs can run independently. These are retained command tabs, not persistent shell sessions.

This is a noninteractive command runner, not a terminal emulator. There is no keyboard input to a running process, full-screen editor, persistent shell session or interactive password prompt. Git prompts and pagers are disabled. Use explicit messages such as git commit -m when a command would otherwise open an editor. Commands such as cd do not persist a new directory; include a path in the command instead.

Up and Down recall the tab's recent command history. Clear output removes visible output; Copy output copies retained output. Stop terminates the running process group and refreshes its repository. Close tab asks before stopping a running command. All tabs lists tabs across repositories and lets you close those no longer needed.

Hide closes the panel while commands keep running; their output is still collected. Quit waits for running commands and other background work. Choose Keep working to cancel the pending quit and stop a command explicitly. Output is limited to recent lines and 2 MiB per tab, very long lines are shortened, and history retains the latest 200 commands. Use a file when you need a complete log. No tabs or output survive restarting the app.

After a shell exits, Recorde continues draining buffered output while progress is being made. Descendants holding its output pipes open are stopped after two seconds without output or a 30-second post-exit limit. Buffered output can still drain after that cleanup, subject to another idle timeout and 30-second maximum. A collection timeout is reported rather than silently treated as complete output.

Commands you enter can modify files and history directly and do not automatically get the app's operation confirmations or recovery snapshots. Review the active repository path before running them. Completion refreshes the associated repository. Saved HTTPS accounts can be used by Git commands through Recorde's credential helper.

Command tabs pass the credential-helper configuration to the commands and scripts they start. Treat repository scripts as trusted code before running them: they can invoke Git with the saved account routing, just as other programs running as your operating-system user can.

## Editors and external tools

Use the external terminal action for interactive shells, editors, credential prompts and tools that need a real terminal. Settings stores configured terminal, editor, diff and merge tool commands. They must be installed and available on PATH or addressed by an absolute path.

The built-in file editor supports reviewing and saving text changes. Unsaved buffers must be saved or discarded before closing the application or restarting for an update, including buffers from previously opened repositories. External edits are checked before saving so newer disk contents are not silently overwritten.

Closing a repository tab or loading a workspace checks for unsaved editor drafts before removing their tabs. The confirmation identifies each affected repository and file. Keep editing preserves the drafts; Discard drafts and continue abandons the listed unsaved edits and completes the tab/workspace action. Save in the editor first if you want to retain those edits on disk.

Reload reads the latest contents from disk. If you have unsaved changes, choose Discard and reload to replace them, or Keep editing to preserve your buffer. A failed reload keeps the buffer intact and displays the error. Reload does not undo a Git commit or restore a file from history.

See [Troubleshooting](Troubleshooting.md) for authentication, missing tools and refresh issues.
