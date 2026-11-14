# Visual design

## Direction

The redesign takes its visual direction from the calm surfaces, spacing and
navigation in [Nulis light](https://dribbble.com/shots/19726217-Nulis-Notes-Desktop-App),
[Nulis dark](https://dribbble.com/shots/19751267-Nulis-Notes-Desktop-App-Dark), and
this [developer workspace](https://dribbble.com/shots/27177143-Developer-workspace-interface-of-the-desktop-app-for-B2C-coding).
No artwork from these references is shipped.

- Graphite dark and soft white light palettes, plus ten single-mode themes based
  on popular VS Code themes (`src/theme/vscode.rs`, generated from published seed
  colours by `scripts/generate_vscode_themes.py`, which adjusts shades to meet
  WCAG AA). Each palette has one interactive accent (Classic blue, Siege21 orange,
  Monokai cyan and so on) for focus, selection, active tabs and the current branch; branch identity colours come only from graph lanes, and the
  sidebar's current-branch dot uses its lane colour. Disabled primary buttons use a
  neutral surface so their label stays readable when faded.
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
- Two header bands, not three: repository tabs and Workspaces share the title bar
  with the menus, and the Sidebar/Changes/Diff/Terminal toggles sit quietly at its
  right edge (no border or accent; shown panels use primary text on a subtle fill).
  The + menu after the tabs opens, clones, creates or reopens repositories, so the
  toolbar no longer repeats the repository name. Tabs keep the same 12-pixel
  padding whether idle, hovered or active, so hovering never shifts them.
  Workspaces is a quiet text menu; favourites are toggled inside it or from a
  tab's right-click menu rather than by a separate star button.
- Single-line text fields use `kit::text_field`, which is 32 pixels tall like a
  standard button (`theme::CONTROL_HEIGHT`) with its text centred, so fields
  beside Save or Browse buttons line up. Dense panels (the branch filter, commit
  summary and history search) use the 28 pixel `kit::compact_text_field`.
  Sliders use `kit::slider` so the
  rail stays visible on light cards. History and Integrations show text labels
  when the toolbar has room and fall back to icons in narrower windows.
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

## Layout kit and motion (October 2026)

- `src/ui/kit.rs` is the shared vocabulary for screens: spacing scale
  (4/8/12/16/24), titled section cards, labelled setting rows that stack when
  narrow, disclosures, navigation items, status chips, callouts, empty states, a
  toggle switch, menu items with right-aligned shortcuts, and one button hierarchy
  (primary, secondary, danger, small, icon).
- `src/ui/motion.rs` provides short ease-out transitions: 0.12 s for hover and fades,
  0.18 s for accordion reveals and disclosures, 0.24 s for panels and banners. Panels
  slide, stage/unstage controls and batch actions expand in place, chevrons rotate and
  hover fills fade. Direct results (staging, selection, scrolling) are immediate.
  Settings > Appearance > Motion chooses Match system setting, Always animate or
  Reduce motion; reduced motion shows every end state instantly.
- Settings uses a searchable left category list (General, Appearance, Accounts & SSH,
  Git, Tools, Keyboard, Repository, Updates, About) with carded pages.
- The Changes panel pins the commit composer to the bottom at its measured height; the
  file list uses the remaining space and scrolls by itself.
- Design inputs and the work plan: `docs/reviews/2026-10-28-ui-polish-plan.md`,
  `2026-10-28-ui-audit.md` and `2026-10-28-git-client-ux-benchmark.md`.

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
