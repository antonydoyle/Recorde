"""Generate src/theme/vscode.rs from the published palettes of popular VS Code themes.

Each theme follows the Siege21 model: quiet layered surfaces, one interactive
accent (with a small-text variant adjusted for contrast), an accent-tinted
selection, diff tints drawn from the theme's own green and red, and graph lanes
from its palette. Seed colours are the themes' published values; derived and
adjusted colours are computed here so every text colour meets WCAG AA (4.5:1)
on every interactive surface, and lanes meet 3:1 against the app background.

Run: python scripts/generate_vscode_themes.py  (writes src/theme/vscode.rs)
"""

import colorsys
import pathlib


def rgb(h):
    h = h.lstrip("#")
    return tuple(int(h[i : i + 2], 16) for i in (0, 2, 4))


def hexs(c):
    return "#%02x%02x%02x" % c


def lum(c):
    def ch(v):
        v /= 255
        return v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4

    r, g, b = c
    return 0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b)


def contrast(a, b):
    la, lb = lum(a), lum(b)
    return (max(la, lb) + 0.05) / (min(la, lb) + 0.05)


def mix(a, b, t):
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


def adjust_lightness(c, delta):
    h, l, s = colorsys.rgb_to_hls(*(v / 255 for v in c))
    l = min(1.0, max(0.0, l + delta))
    return tuple(round(v * 255) for v in colorsys.hls_to_rgb(h, l, s))


def readable(c, surfaces, dark, target=4.5):
    """Nudge lightness (keeping hue) until `c` reaches `target` on all surfaces."""
    step = 0.01 if dark else -0.01
    for _ in range(100):
        if all(contrast(c, s) >= target for s in surfaces):
            return c
        c = adjust_lightness(c, step)
    raise SystemExit(f"cannot make {hexs(c)} readable")


THEMES = [
    # name, label, dark, surfaces(titlebar, app, panel, panel_alt, elevated),
    # border, border_strong, text(primary, secondary, muted), accent, fill,
    # semantic(green, red, yellow, purple, orange, teal), lanes
    dict(
        name="MONOKAI", label="Monokai", dark=True,
        titlebar="#1e1f1c", app="#232420", panel="#272822", panel_alt="#2f302a", elevated="#34352f",
        border="#3e3d32", border_strong="#75715e",
        primary="#f8f8f2", secondary="#e6e6dc", muted="#90908a",
        accent="#66d9ef", fill="#66d9ef",
        green="#a6e22e", red="#f92672", yellow="#e6db74", purple="#ae81ff", orange="#fd971f", teal="#a1efe4",
        lanes=["#66d9ef", "#f92672", "#ae81ff", "#a6e22e", "#e6db74", "#fd971f", "#a1efe4", "#f8f8f2"],
    ),
    dict(
        name="TOKYO_NIGHT", label="Tokyo Night", dark=True,
        titlebar="#16161e", app="#1a1b26", panel="#1e2030", panel_alt="#24283b", elevated="#292e42",
        border="#2f334d", border_strong="#565f89",
        primary="#c0caf5", secondary="#a9b1d6", muted="#737aa2",
        accent="#7aa2f7", fill="#3d59a1",
        green="#9ece6a", red="#f7768e", yellow="#e0af68", purple="#bb9af7", orange="#ff9e64", teal="#73daca",
        lanes=["#7aa2f7", "#bb9af7", "#7dcfff", "#9ece6a", "#e0af68", "#ff9e64", "#f7768e", "#73daca"],
    ),
    dict(
        name="DRACULA", label="Dracula", dark=True,
        titlebar="#191a21", app="#21222c", panel="#282a36", panel_alt="#303341", elevated="#383a4a",
        border="#44475a", border_strong="#6272a4",
        primary="#f8f8f2", secondary="#e2e2dc", muted="#6272a4",
        accent="#bd93f9", fill="#bd93f9",
        green="#50fa7b", red="#ff5555", yellow="#f1fa8c", purple="#ff79c6", orange="#ffb86c", teal="#8be9fd",
        lanes=["#bd93f9", "#ff79c6", "#8be9fd", "#50fa7b", "#f1fa8c", "#ffb86c", "#ff5555", "#f8f8f2"],
    ),
    dict(
        name="ONE_DARK_PRO", label="One Dark Pro", dark=True,
        titlebar="#21252b", app="#23272e", panel="#282c34", panel_alt="#2c313a", elevated="#333842",
        border="#3e4451", border_strong="#5c6370",
        primary="#d7dae0", secondary="#abb2bf", muted="#7f848e",
        accent="#61afef", fill="#4d78cc",
        green="#98c379", red="#e06c75", yellow="#e5c07b", purple="#c678dd", orange="#d19a66", teal="#56b6c2",
        lanes=["#61afef", "#c678dd", "#56b6c2", "#98c379", "#e5c07b", "#d19a66", "#e06c75", "#abb2bf"],
    ),
    dict(
        name="NORD", label="Nord", dark=True,
        titlebar="#242933", app="#2a2f3a", panel="#2e3440", panel_alt="#353c4a", elevated="#3b4252",
        border="#4c566a", border_strong="#616e88",
        primary="#eceff4", secondary="#d8dee9", muted="#9aa5b8",
        accent="#88c0d0", fill="#88c0d0",
        green="#a3be8c", red="#bf616a", yellow="#ebcb8b", purple="#b48ead", orange="#d08770", teal="#8fbcbb",
        lanes=["#88c0d0", "#81a1c1", "#b48ead", "#a3be8c", "#ebcb8b", "#d08770", "#bf616a", "#8fbcbb"],
    ),
    dict(
        name="GRUVBOX_DARK", label="Gruvbox Dark", dark=True,
        titlebar="#1d2021", app="#282828", panel="#32302f", panel_alt="#3c3836", elevated="#45403d",
        border="#504945", border_strong="#7c6f64",
        primary="#ebdbb2", secondary="#d5c4a1", muted="#a89984",
        accent="#fe8019", fill="#fe8019",
        green="#b8bb26", red="#fb4934", yellow="#fabd2f", purple="#d3869b", orange="#fe8019", teal="#8ec07c",
        lanes=["#83a598", "#d3869b", "#8ec07c", "#b8bb26", "#fabd2f", "#fe8019", "#fb4934", "#ebdbb2"],
    ),
    dict(
        name="SYNTHWAVE", label="SynthWave '84", dark=True,
        titlebar="#171520", app="#241b2f", panel="#262335", panel_alt="#2a2139", elevated="#34294f",
        border="#3d3354", border_strong="#848bbd",
        primary="#f4eee4", secondary="#e2d6f2", muted="#848bbd",
        accent="#ff7edb", fill="#ff7edb",
        green="#72f1b8", red="#fe4450", yellow="#fede5d", purple="#b893ce", orange="#ff8b39", teal="#36f9f6",
        lanes=["#ff7edb", "#36f9f6", "#fede5d", "#72f1b8", "#ff8b39", "#fe4450", "#b893ce", "#f97e72"],
    ),
    dict(
        name="GITHUB_LIGHT", label="GitHub Light", dark=False,
        titlebar="#eff2f5", app="#f6f8fa", panel="#ffffff", panel_alt="#f6f8fa", elevated="#ffffff",
        border="#d0d7de", border_strong="#8c959f",
        primary="#1f2328", secondary="#424a53", muted="#59636e",
        accent="#0969da", fill="#0969da",
        green="#1a7f37", red="#cf222e", yellow="#9a6700", purple="#8250df", orange="#bc4c00", teal="#1b7c83",
        lanes=["#0969da", "#8250df", "#1b7c83", "#1a7f37", "#9a6700", "#bc4c00", "#cf222e", "#bf3989"],
    ),
    dict(
        name="SOLARIZED_LIGHT", label="Solarized Light", dark=False,
        titlebar="#eee8d5", app="#f5efdc", panel="#fdf6e3", panel_alt="#eee8d5", elevated="#fffbf0",
        border="#ddd6c1", border_strong="#93a1a1",
        primary="#073642", secondary="#586e75", muted="#657b83",
        accent="#268bd2", fill="#268bd2",
        green="#859900", red="#dc322f", yellow="#b58900", purple="#6c71c4", orange="#cb4b16", teal="#2aa198",
        lanes=["#268bd2", "#d33682", "#2aa198", "#859900", "#b58900", "#cb4b16", "#dc322f", "#6c71c4"],
    ),
    dict(
        name="CATPPUCCIN_LATTE", label="Catppuccin Latte", dark=False,
        titlebar="#dce0e8", app="#e6e9ef", panel="#eff1f5", panel_alt="#e6e9ef", elevated="#f5f6f9",
        border="#ccd0da", border_strong="#8c8fa1",
        primary="#4c4f69", secondary="#5c5f77", muted="#6c6f85",
        accent="#8839ef", fill="#8839ef",
        green="#40a02b", red="#d20f39", yellow="#df8e1d", purple="#ea76cb", orange="#fe640b", teal="#179299",
        lanes=["#1e66f5", "#8839ef", "#179299", "#40a02b", "#df8e1d", "#fe640b", "#d20f39", "#ea76cb"],
    ),
]

ORDER = [
    "bg_app", "bg_titlebar", "bg_toolbar", "bg_panel", "bg_panel_alt", "bg_hover", "bg_selected",
    "bg_elevated", "border", "border_strong", "text_primary", "text_secondary", "text_muted",
    "bg_diff_add", "bg_diff_del", "bg_diff_hunk", "accent", "primary_fill", "on_primary",
    "green", "red", "yellow", "purple", "orange", "teal",
]


def build(t):
    d = t["dark"]
    c = {k: rgb(v) for k, v in t.items() if isinstance(v, str) and v.startswith("#")}
    out = {}
    out["bg_titlebar"] = c["titlebar"]
    out["bg_app"] = c["app"]
    out["bg_toolbar"] = c["panel"]
    out["bg_panel"] = c["panel"]
    out["bg_panel_alt"] = c["panel_alt"]
    out["bg_elevated"] = c["elevated"]
    out["bg_hover"] = mix(c["panel"], c["primary"], 0.07 if d else 0.06)
    # Siege21 tints selection with the accent rather than using a grey.
    out["bg_selected"] = mix(c["panel"], c["accent"], 0.20 if d else 0.14)
    out["border"] = c["border"]
    out["bg_diff_add"] = mix(c["app"], c["green"], 0.16 if d else 0.14)
    out["bg_diff_del"] = mix(c["app"], c["red"], 0.16 if d else 0.12)
    out["bg_diff_hunk"] = mix(c["app"], c["accent"], 0.12 if d else 0.10)
    text_surfaces = [out[k] for k in ("bg_app", "bg_panel", "bg_panel_alt", "bg_hover", "bg_selected", "bg_elevated")]
    # Strong borders outline radios and other controls: 3:1 on cards and panels.
    out["border_strong"] = readable(c["border_strong"], text_surfaces, d, 3.0)
    for key, seed in (("text_primary", "primary"), ("text_secondary", "secondary"), ("text_muted", "muted"), ("accent", "accent")):
        out[key] = readable(c[seed], text_surfaces, d)
    # Status colours label files and rows, which sit on the app, panel or hover
    # surfaces; checking them against every card colour washed out their hue.
    status_surfaces = [out[k] for k in ("bg_app", "bg_panel", "bg_hover")]
    for key in ("green", "red", "yellow", "purple", "orange", "teal"):
        out[key] = readable(c[key], status_surfaces, d)
    # Primary buttons keep the theme's exact fill when a label can reach 4.5:1;
    # otherwise the fill moves away from the label until it does.
    fill = c["fill"]
    light_label, dark_label = (255, 255, 255), c["titlebar"] if d else c["primary"]
    label = light_label if contrast(fill, light_label) >= contrast(fill, dark_label) else dark_label
    for _ in range(100):
        if contrast(fill, label) >= 4.5:
            break
        fill = adjust_lightness(fill, -0.01 if label == light_label else 0.01)
    out["primary_fill"], out["on_primary"] = fill, label
    lanes = [readable(rgb(l), [out["bg_app"], out["bg_panel"]], d, 3.0) for l in t["lanes"]]
    return out, lanes


def rust_color(c):
    return f"Color32::from_rgb(0x{c[0]:02x}, 0x{c[1]:02x}, 0x{c[2]:02x})"


def main():
    lines = [
        "//! Palettes modelled on popular VS Code themes, built the way Siege21 is: layered",
        "//! surfaces, one accent, accent-tinted selection and theme-coloured diffs.",
        "//! Generated by `scripts/generate_vscode_themes.py`; edit the seeds there.",
        "",
        "use egui::Color32;",
        "",
        "use super::ThemeTokens;",
        "",
    ]
    for t in THEMES:
        out, lanes = build(t)
        mode = "dark" if t["dark"] else "light"
        lines.append(f"/// {t['label']} ({mode}).")
        lines.append(f"pub const {t['name']}: ThemeTokens = ThemeTokens {{")
        lines.append(f"    dark: {'true' if t['dark'] else 'false'},")
        for key in ORDER:
            lines.append(f"    {key}: {rust_color(out[key])},")
        lines.append("    lanes: [")
        for lane in lanes:
            lines.append(f"        {rust_color(lane)},")
        lines.append("    ],")
        lines.append("};")
        lines.append("")
        changed = [
            f"{k} {t.get({'text_primary':'primary','text_secondary':'secondary','text_muted':'muted'}.get(k,k))}->{hexs(out[k])}"
            for k in ("text_primary", "text_secondary", "text_muted", "accent", "green", "red", "yellow", "purple", "orange", "teal", "border_strong")
            if t.get({"text_primary": "primary", "text_secondary": "secondary", "text_muted": "muted"}.get(k, k)) != hexs(out[k])
        ]
        print(f"{t['label']}: fill {t['fill']}->{hexs(out['primary_fill'])} label {hexs(out['on_primary'])}; adjusted: {', '.join(changed) or 'none'}")
    path = pathlib.Path(__file__).resolve().parent.parent / "src" / "theme" / "vscode.rs"
    path.parent.mkdir(exist_ok=True)
    path.write_text("\n".join(lines), encoding="utf-8", newline="\n")
    print(f"wrote {path}")


if __name__ == "__main__":
    main()
