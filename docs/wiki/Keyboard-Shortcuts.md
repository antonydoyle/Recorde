# Keyboard shortcuts

## Discover commands

Open the command palette with Ctrl+K. On macOS, use Command wherever this page says Ctrl. Type a command name, use Up and Down to select a result, Enter to run it, or Escape to close the palette. Unavailable commands explain the required repository, selection or operation state.

The palette includes repository lifecycle, remote operations, branch/history actions, Git Flow, provider views, searches, settings and panel focus. Context menus expose additional actions for a particular file, branch, tag or commit.

## Default bindings

- Ctrl+K: command palette.
- Ctrl+Shift+F: fetch all remotes.
- Ctrl+Shift+P: pull.
- Ctrl+Shift+U: push.
- Ctrl+B: switch branch.
- Ctrl+F: search commits.
- Ctrl+1: focus sidebar.
- Ctrl+2: focus commit graph.
- Ctrl+3: focus changes.
- Ctrl+4: focus diff.
- Ctrl+Z: undo last operation.
- Ctrl+Shift+Z: redo undone operation.
- Ctrl+Backtick: toggle integrated terminal.

Search commits and Find in diff are separate commands. Find in diff can be opened from the diff header or assigned a shortcut in Settings. Ctrl+F is not a universal search shortcut for every panel.

## Customize bindings

Open Settings > Keyboard. Each command shows its current effective binding. Choose Change, then press a combination with Ctrl, or Command on macOS. Escape cancels recording. Clear disables the command's binding; Reset all shortcuts to defaults restores the defaults above.

Duplicate bindings are rejected without replacing the previous choice. Reserved window-closing and system combinations cannot be assigned. Changes are saved across restarts.

Global repository shortcuts pause while editing text or using dialogs. This lets ordinary text editing retain its undo and selection behavior. Close a dialog or move focus out of a text field before invoking a repository command.

## Keyboard navigation

Use Tab and Shift+Tab to move between controls and Enter or Space to activate focused buttons. Graph rows and branch labels expose accessible names and selection states. The command palette offers alternatives to graph drag gestures.

In Terminal commands, Up and Down recall the selected tab's command history. Enter runs one command when that tab is idle. Stop terminates a running command; Hide leaves it running. Close tab asks before stopping a running command.

See [Themes and accessibility](Themes-and-Accessibility.md) for scale, contrast and graph colours.
