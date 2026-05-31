/// Material Icons font family name for use with egui::FontId.
pub const FAMILY: &str = "material-icons";

pub fn font(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(FAMILY.into()))
}

/// Render an icon as egui RichText using the Material Icons font.
pub fn rich(c: char, size: f32) -> egui::RichText {
    egui::RichText::new(c).font(font(size))
}

// ── Codepoints ────────────────────────────────────────────────────────────────

pub const SEARCH:          char = '\u{E8B6}';
pub const SETTINGS:        char = '\u{E8B8}';
pub const CLOSE:           char = '\u{E5CD}';
pub const ADD:             char = '\u{E145}';
pub const ADD_CIRCLE:      char = '\u{E147}';
pub const MENU:            char = '\u{E5D2}';
pub const MORE_VERT:       char = '\u{E5D4}';
pub const CHEVRON_RIGHT:   char = '\u{E5CC}';
pub const EXPAND_MORE:     char = '\u{E5CF}';

pub const DOWNLOAD:        char = '\u{F090}'; // Fetch
pub const UPLOAD:          char = '\u{F09B}'; // Push
pub const SWAP_VERT:       char = '\u{E8D5}'; // Pull
pub const REFRESH:         char = '\u{E5D5}';
pub const CLOUD_UPLOAD:    char = '\u{E2C3}';
pub const CLOUD_DOWNLOAD:  char = '\u{E2C0}';

pub const HUB:             char = '\u{E9F4}'; // New Branch
pub const MERGE:           char = '\u{EB98}';
pub const COMMIT:          char = '\u{EAF5}';
pub const SOURCE:          char = '\u{F1C4}';
pub const COMPARE_ARROWS:  char = '\u{E915}';

pub const HISTORY:         char = '\u{E889}'; // Stash
pub const STAR:            char = '\u{E838}'; // Tag
pub const TAG:             char = '\u{E9EF}';
pub const FOLDER_OPEN:     char = '\u{E2C8}';

pub const PERSON:          char = '\u{E7FD}';
pub const ACCOUNT_CIRCLE:  char = '\u{E853}';
pub const LOGIN:           char = '\u{EA77}';
pub const LOGOUT:          char = '\u{E9BA}';
pub const KEY:             char = '\u{E73C}';
pub const LOCK:            char = '\u{E897}';

pub const TERMINAL:        char = '\u{EB8E}';
pub const CODE:            char = '\u{E86F}';
pub const TUNE:            char = '\u{E429}';
pub const FILTER:          char = '\u{E3D3}';
pub const SORT:            char = '\u{E164}';

pub const CHECK:           char = '\u{E5CA}';
pub const ERROR_ICON:      char = '\u{E000}';
pub const WARNING:         char = '\u{E002}';
pub const INFO:            char = '\u{E88E}';
pub const HELP:            char = '\u{E887}';

pub const EDIT:            char = '\u{E3C9}';
pub const DELETE:          char = '\u{E872}';
pub const COPY:            char = '\u{E14D}';
pub const VISIBILITY:      char = '\u{E8F4}';
pub const OPEN_IN_NEW:     char = '\u{E89E}';

pub const BRIGHTNESS:      char = '\u{E3A6}'; // brightness_6 — theme toggle
pub const DARK_MODE:       char = '\u{E51C}'; // dark_mode
pub const LIGHT_MODE:      char = '\u{E518}'; // light_mode

pub const BUG_REPORT:      char = '\u{E868}'; // provider integrations / issues
pub const ROCKET_LAUNCH:   char = '\u{EA9C}'; // releases
pub const CHECK_CIRCLE:    char = '\u{E86C}'; // CI success
pub const CANCEL:          char = '\u{E5C9}'; // CI failure
