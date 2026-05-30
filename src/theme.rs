use egui::Color32;

// --- Background layers ---
pub const BG_BASE: Color32 = Color32::from_rgb(0x1C, 0x1E, 0x23);
pub const BG_SURFACE: Color32 = Color32::from_rgb(0x23, 0x26, 0x2D);
pub const BG_ELEVATED: Color32 = Color32::from_rgb(0x2A, 0x2D, 0x35);
pub const BG_SELECTED: Color32 = Color32::from_rgb(0x1E, 0x2D, 0x4A);

// --- Text ---
pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xE2, 0xE4, 0xE9);
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(0x8B, 0x8F, 0xA8);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x55, 0x59, 0x6A);

// --- Accents ---
pub const ACCENT_BLUE: Color32 = Color32::from_rgb(0x4D, 0x88, 0xFF);
pub const ACCENT_GREEN: Color32 = Color32::from_rgb(0x5C, 0xB8, 0x5C);
pub const ACCENT_RED: Color32 = Color32::from_rgb(0xD9, 0x41, 0x3D);
pub const ACCENT_YELLOW: Color32 = Color32::from_rgb(0xEC, 0xB9, 0x1C);
pub const ACCENT_PURPLE: Color32 = Color32::from_rgb(0x9B, 0x59, 0xB6);

// --- Borders ---
pub const BORDER: Color32 = Color32::from_rgb(0x33, 0x37, 0x48);

// --- Graph lane colors (8-color palette, cycles) ---
pub const LANE_COLORS: &[Color32] = &[
    Color32::from_rgb(0x4D, 0x88, 0xFF), // blue
    Color32::from_rgb(0xF0, 0x88, 0x3E), // orange
    Color32::from_rgb(0x5C, 0xB8, 0x5C), // green
    Color32::from_rgb(0xEC, 0xB9, 0x1C), // yellow
    Color32::from_rgb(0x9B, 0x59, 0xB6), // purple
    Color32::from_rgb(0x1A, 0xBC, 0x9C), // teal
    Color32::from_rgb(0xE9, 0x1E, 0x63), // pink
    Color32::from_rgb(0x00, 0xBC, 0xD4), // cyan
];

pub fn lane_color(lane: usize) -> Color32 {
    LANE_COLORS[lane % LANE_COLORS.len()]
}

/// Deterministic color for an author name — hashed into the lane palette.
pub fn author_color(name: &str) -> Color32 {
    let hash = name
        .bytes()
        .fold(5381u32, |acc, b| acc.wrapping_mul(33).wrapping_add(b as u32));
    LANE_COLORS[hash as usize % LANE_COLORS.len()]
}

/// Up-to-2-character initials from an author name.
pub fn author_initials(name: &str) -> String {
    let mut words = name.split_whitespace();
    let first = words.next().and_then(|s| s.chars().next());
    let last = words.last().and_then(|s| s.chars().next());
    match (first, last) {
        (Some(f), Some(l)) => format!("{}{}", f.to_uppercase(), l.to_uppercase()),
        (Some(f), None) => f.to_uppercase().to_string(),
        _ => "?".to_string(),
    }
}

/// Human-readable relative timestamp: "3h ago", "2d ago", etc.
pub fn relative_time(unix_ts: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let diff = (now - unix_ts).max(0);
    if diff < 60 {
        "just now".into()
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86_400 {
        format!("{}h ago", diff / 3600)
    } else if diff < 86_400 * 7 {
        format!("{}d ago", diff / 86_400)
    } else if diff < 86_400 * 30 {
        format!("{}w ago", diff / (86_400 * 7))
    } else if diff < 86_400 * 365 {
        format!("{}mo ago", diff / (86_400 * 30))
    } else {
        format!("{}y ago", diff / (86_400 * 365))
    }
}
