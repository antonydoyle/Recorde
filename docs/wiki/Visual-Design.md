# Visual design

## Direction

The redesign takes its visual direction from the calm surfaces, spacing and
navigation in [Nulis light](https://dribbble.com/shots/19726217-Nulis-Notes-Desktop-App),
[Nulis dark](https://dribbble.com/shots/19751267-Nulis-Notes-Desktop-App-Dark), and
this [developer workspace](https://dribbble.com/shots/27177143-Developer-workspace-interface-of-the-desktop-app-for-B2C-coding).
No artwork from these references is shipped.

- Graphite dark and soft white light palettes, with independent accent colours.
- Consistent controls, spacing, typography, focus states and restrained borders.
- Graph navigation has a 10-pixel top inset. Custom-coloured and icon buttons
  retain visible hover, pressed and keyboard-focus states; disabled controls
  remain inactive. Shared button rendering lives in `src/ui/button.rs`.
- Embedded Inter Regular replaces the framework's Ubuntu Light default. The
  normal body size is 14 logical pixels, secondary labels are 12, and code is 13.
  Light-theme secondary text has stronger contrast. UI zoom multiplies the native
  monitor scale instead of replacing its pixel density; text retains pixel snapping.
- Full-height navigation and Changes/Details panels; the graph and diff share the
  centre column. Panels remain resizable and their preferences are saved.
- A separate scrolling change list keeps the commit form accessible. Additional
  authors, signing and reset controls are grouped under expandable headings.
- Diff display options live in the sliders menu. Search remains directly available.
- A welcome screen provides Open, Clone and Create actions.
- Commit graph topology, branching geometry and history actions are preserved.
  Neon lane and node colours use a stable shuffle based on the full repository
  path, cached when switching repositories. Refreshing or restarting keeps the
  same order; relocating a repository can change it. Light mode uses saturated,
  darker counterparts for contrast, and the colourblind palette remains available.
  Narrower initial columns give commit messages more space. Lane colours adapt to
  light/dark themes; high contrast and colourblind options remain available.
- Icon fonts and symbol fallbacks are bundled. Incorrect Filter, Brightness and
  Rocket codepoints were corrected against the upstream Material icon map.

## Verification

Run `scripts/ui_review.py <binary> <external-artifacts>` through `linux_task.py`.
It creates disposable branching repositories and captures dark, light, compact
and welcome layouts using a private authenticated Xvfb display. It never uses the
user's repository or configuration. These captures supplement the staging and
image assertions in `gui_smoke.py`; they are visual review artifacts, not pixel
comparison assertions.

Use the native Windows smoke fixture for startup and the font coverage test for
embedded glyph availability on Windows. Linux screenshots do not establish Windows
DPI behaviour; include native DPI checks during manual testing. macOS validation
remains parked.

## GUI maintenance validation (v0.4 candidate)

The eframe/egui 0.36.2 migration was checked on Linux with Rust 1.99. The native
AccessKit feature is enabled; the development inspector remains optional and is
absent from production builds. The old eframe vendor patch is replaced by making
the native window visible in the app creation callback before its first swap.

- All-target compilation and six process-supervision fixtures pass.
- Eighteen focused UI/font/shortcut tests pass, including embedded glyph
  coverage, monitor density changes, button states and graph drag cancellation.
- Eight native X11 captures cover dark/light at 100%, 125%, 150% and 200% display
  scale. Each checks reported pixel density, screenshot dimensions, commit and
  branch labels, action names, six distinct Tab focuses, commit selection, and
  arrow navigation that moves selection and accessible focus together.
- A native WSLg Wayland run with `DISPLAY` removed renders its first frame and
  passes the AccessKit/Tab checks. This checks the available compositor; it does
  not establish behavior on every COSMIC compositor version.
- `cargo audit --deny unsound` passes with no advisory or maintenance warnings;
  the lock no longer contains `ab_glyph`, `ttf-parser`, `paste` or `sctk-adwaita`.

Reproduce the matrix with `scripts/ui_review.py ... --dpi-matrix`, using the
supervised fixture and inspector described in the Developer Guide. Captures are
native framebuffer output; they are reviewed visually rather than compared to a
fixed golden image. Repository-specific lane colors may differ by fixture path.
`scripts/windows_smoke.ps1 -InspectAccessibility` additionally checks the real
Windows UI Automation bridge and native focus on an interactive Windows test
desktop. That hosted check and native Windows/macOS DPI acceptance are separate
from the Linux evidence above.
