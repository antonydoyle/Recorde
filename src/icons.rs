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

pub const SEARCH: char = '\u{E8B6}';
pub const SETTINGS: char = '\u{E8B8}';
pub const CLOSE: char = '\u{E5CD}';
pub const ADD: char = '\u{E145}';
pub const ADD_CIRCLE: char = '\u{E147}';
pub const MENU: char = '\u{E5D2}';
pub const MORE_VERT: char = '\u{E5D4}';
pub const CHEVRON_RIGHT: char = '\u{E5CC}';
pub const EXPAND_MORE: char = '\u{E5CF}';

pub const DOWNLOAD: char = '\u{F090}'; // Fetch
pub const UPLOAD: char = '\u{F09B}'; // Push
pub const SWAP_VERT: char = '\u{E8D5}'; // Pull
pub const REFRESH: char = '\u{E5D5}';
pub const CLOUD_UPLOAD: char = '\u{E2C3}';
pub const CLOUD_DOWNLOAD: char = '\u{E2C0}';

pub const HUB: char = '\u{E9F4}'; // New Branch
pub const MERGE: char = '\u{EB98}';
pub const COMMIT: char = '\u{EAF5}';
pub const SOURCE: char = '\u{F1C4}';
pub const COMPARE_ARROWS: char = '\u{E915}';

pub const HISTORY: char = '\u{E889}'; // Stash
pub const STAR: char = '\u{E838}'; // Tag
pub const TAG: char = '\u{E9EF}';
pub const FOLDER_OPEN: char = '\u{E2C8}';

pub const PERSON: char = '\u{E7FD}';
pub const ACCOUNT_CIRCLE: char = '\u{E853}';
pub const LOGIN: char = '\u{EA77}';
pub const LOGOUT: char = '\u{E9BA}';
pub const KEY: char = '\u{E73C}';
pub const LOCK: char = '\u{E897}';

pub const TERMINAL: char = '\u{EB8E}';
pub const CODE: char = '\u{E86F}';
pub const TUNE: char = '\u{E429}';
pub const FILTER: char = '\u{E152}';
pub const SORT: char = '\u{E164}';

pub const CHECK: char = '\u{E5CA}';
pub const ERROR_ICON: char = '\u{E000}';
pub const WARNING: char = '\u{E002}';
pub const INFO: char = '\u{E88E}';
pub const HELP: char = '\u{E887}';

pub const EDIT: char = '\u{E3C9}';
pub const UNDO: char = '\u{E166}';
pub const REDO: char = '\u{E15A}';
pub const DELETE: char = '\u{E872}';
pub const COPY: char = '\u{E14D}';
pub const VISIBILITY: char = '\u{E8F4}';
pub const OPEN_IN_NEW: char = '\u{E89E}';

pub const BRIGHTNESS: char = '\u{E3AB}'; // brightness_6 — theme toggle
pub const DARK_MODE: char = '\u{E51C}'; // dark_mode
pub const LIGHT_MODE: char = '\u{E518}'; // light_mode

pub const BUG_REPORT: char = '\u{E868}'; // provider integrations / issues
pub const ROCKET_LAUNCH: char = '\u{EB9B}'; // releases
pub const CHECK_CIRCLE: char = '\u{E86C}'; // CI success
pub const CANCEL: char = '\u{E5C9}'; // CI failure
pub const KEYBOARD: char = '\u{E312}';
pub const PALETTE: char = '\u{E40A}'; // appearance
pub const UPDATE: char = '\u{E923}';
pub const FOLDER: char = '\u{E2C7}';
pub const LINK: char = '\u{E157}';
pub const CLOUD: char = '\u{E2BD}';
pub const VPN_KEY: char = '\u{E0DA}'; // SSH keys
pub const ACCOUNT_TREE: char = '\u{E97A}'; // branches
pub const PUBLIC: char = '\u{E80B}'; // remotes
pub const VERIFIED_USER: char = '\u{E8E8}';
pub const BADGE: char = '\u{EA67}'; // commit identity
pub const MANAGE_ACCOUNTS: char = '\u{F02E}';
pub const CALL_SPLIT: char = '\u{E0B6}';
pub const ARCHIVE: char = '\u{E149}'; // stash
pub const UNARCHIVE: char = '\u{E169}';
pub const EXPAND_LESS: char = '\u{E5CE}';
pub const REMOVE: char = '\u{E15B}';
pub const DONE_ALL: char = '\u{E877}';
pub const SYNC: char = '\u{E627}';
pub const BLOCK: char = '\u{E14B}';
pub const CONSTRUCTION: char = '\u{EA3C}'; // tools
pub const DESKTOP: char = '\u{E30C}';
pub const DIFFERENCE: char = '\u{EB7D}';
pub const PLAYLIST_ADD: char = '\u{E03B}';
pub const LIGHTBULB: char = '\u{E0F0}'; // tips

/// Embedded fonts: no dependency on the operating system font installation.
pub fn font_definitions() -> egui::FontDefinitions {
    // Embedded outline fonts render symbols identically on every platform.
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "inter-regular".into(),
        egui::FontData::from_static(include_bytes!("../assets/fonts/Inter-Regular.ttf")).into(),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "inter-regular".into());
    for (name, bytes) in [
        (
            "noto-symbols-basic",
            include_bytes!("../assets/fonts/NotoSansSymbols.ttf").as_slice(),
        ),
        (
            "noto-symbols",
            include_bytes!("../assets/fonts/NotoSansSymbols2-Regular.ttf").as_slice(),
        ),
        (
            "noto-emoji",
            include_bytes!("../assets/fonts/NotoEmoji.ttf").as_slice(),
        ),
    ] {
        fonts
            .font_data
            .insert(name.into(), egui::FontData::from_static(bytes).into());
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(family).or_default().push(name.into());
        }
    }
    // Material Icons — embedded at compile time so the installed binary
    // doesn't need assets/ alongside it (fixes the deb/AppImage install crash).
    fonts.font_data.insert(
        "material-icons".into(),
        egui::FontData::from_static(include_bytes!("../assets/MaterialIcons.ttf")).into(),
    );
    let mut material_family = vec!["material-icons".into()];
    // egui also needs a replacement glyph when initializing an icon font.
    // Keep text fallbacks after Material so all declared icons still use its outlines.
    material_family.extend(
        fonts.families[&egui::FontFamily::Proportional]
            .iter()
            .cloned(),
    );
    fonts
        .families
        .insert(egui::FontFamily::Name(FAMILY.into()), material_family);
    fonts
}

#[cfg(test)]
mod tests {
    #[test]
    fn embedded_fonts_cover_every_icon_and_ui_symbol() {
        let ctx = egui::Context::default();
        ctx.set_fonts(super::font_definitions());
        ctx.begin_pass(Default::default());
        let re = regex::Regex::new(r"pub const (\w+): char = '\\u\{([0-9A-F]+)\}'").unwrap();
        let mut count = 0;
        // Inspect actual family charmaps: egui 0.36 has_glyph compares face
        // IDs with the replacement face and can reject its valid symbols.
        ctx.fonts_mut(|fonts| {
            for capture in re.captures_iter(include_str!("icons.rs")) {
                let c = char::from_u32(u32::from_str_radix(&capture[2], 16).unwrap()).unwrap();
                assert!(
                    fonts
                        .fonts
                        .font(&super::font(18.0).family)
                        .characters()
                        .contains_key(&c),
                    "Missing Material icon: {} ({c})",
                    &capture[1]
                );
                count += 1;
            }
            assert!(count > 40, "Icon audit must enumerate the constants");
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                let font = egui::FontId::new(14.0, family);
                for c in "⚠✓✕×→←↑↓▼▶▾●★⌘⌥⇧⎇📁🔒📋".chars() {
                    assert!(
                        fonts.fonts.font(&font.family).characters().contains_key(&c),
                        "Missing UI symbol {c} in {font:?}"
                    );
                }
            }
        });
        ctx.end_pass().textures_delta.clear();
    }
}
