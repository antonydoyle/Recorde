# Themes and accessibility

## Choose a colour theme

Open Settings > Appearance and select Classic or Siege21 in Colour palette. Dark theme switches the selected palette between light and dark. Choose Siege21 with Dark theme off for warm beige and orange; turn Dark theme on for orange and white on dark surfaces. The orange is based on siege21.com's published brand colour, with readable foreground variations for small text.

Theme changes update the workspace immediately and are saved for the next launch. High contrast is a separate preference and can be combined with the selected theme. Diff additions, deletions, errors and warnings retain distinct semantic colours.

## Scale and readability

UI scale 1.0 follows your display's native scaling. Increase the value for larger controls and text, then choose Apply. Scaling is relative to the monitor, so moving between displays can also change the effective size.

Use high contrast when you need stronger separation between text, controls and backgrounds. Colour-blind-safe graph colours change the graph lane palette. Branch labels, commit messages and selection states provide additional information beyond colour alone.

Recorde bundles its UI font, Material Icons, Noto Symbols and monochrome emoji fallbacks; no font installation is required. Report any missing symbol with the app version, platform and display scale.

## Motion

Panels slide, lists and options expand like an accordion, and hover highlights fade, so you can see what changed. Animations are short (about a fifth of a second) and never delay an action: staging a file or selecting a commit happens immediately. Settings > Appearance > Motion chooses Match system setting (the default), Always animate or Reduce motion. On Windows, Match system setting follows Settings > Accessibility > Visual effects > Animation effects. Reduce motion shows every change instantly.

## Arrange the workspace

Use the View menu and command palette to focus or show the sidebar, graph, changes and diff areas. Resize panel dividers to suit the task. The graph's Columns menu, also available by right-clicking a header, hides columns, moves them left/right and changes the date format. Relative dates, ISO dates and UTC date/time are available. The Commit column remains visible. Column choices, order, date format and widths are saved per repository.

The Changes file list scrolls separately from the commit form. The diff header offers Unified and Split layouts, whitespace options and Find. Narrow windows can benefit from hiding a panel or using the unified diff.

## Keyboard and assistive technology

Native accessibility is provided through AccessKit. Commit rows and branch labels expose names, selection and keyboard focus. Tab moves through controls; the command palette and context menus provide alternatives to graph drag gestures.

Global repository shortcuts pause while a text field or dialog is active. Help offers search, Back, Forward and Home navigation, selectable text and copy buttons for command examples. Its topics collapse into a selector on narrow windows.

See [Keyboard shortcuts](Keyboard-Shortcuts.md) for defaults and custom bindings.

## Author pictures and privacy

Author pictures are off by default. Enabling them in Appearance requests Gravatar images using a SHA-256 hash of author email addresses. GitHub no-reply addresses use the GitHub account number instead. Images are cached for a week; initials are shown when pictures are unavailable or disabled.
