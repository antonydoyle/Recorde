# Embedded UI fonts

Recorde loads these fonts from its executable, without relying on installed system
fonts or downloading assets at runtime. Original font files are unmodified.

| Font | Upstream | License |
| --- | --- | --- |
| Inter Regular 4.1 | [Inter 4.1 release](https://github.com/rsms/inter/releases/tag/v4.1), `extras/ttf/Inter-Regular.ttf` | `Inter-LICENSE.txt` |
| Noto Sans Symbols | [Google Fonts](https://github.com/google/fonts/tree/main/ofl/notosanssymbols) | `NotoSansSymbols-OFL.txt` |
| Noto Sans Symbols 2 | [Google Fonts](https://github.com/google/fonts/tree/main/ofl/notosanssymbols2) | `NotoSansSymbols2-OFL.txt` |
| Noto Emoji (monochrome outlines) | [Google Fonts](https://github.com/google/fonts/tree/main/ofl/notoemoji) | `NotoEmoji-OFL.txt` |
| Material Icons (`../MaterialIcons.ttf`) | [Google Material Design Icons](https://github.com/google/material-design-icons/tree/master/font) | `MaterialIcons-LICENSE.txt` |

Inter Regular is the primary proportional UI face; Hack remains the code font.
The Noto fallbacks support both proportional and monospace UI text. Material Icons
uses a separate named family. `icons::tests::embedded_fonts_cover_every_icon_and_ui_symbol`
checks all declared Material codepoints and the UI's common symbol repertoire
using the same egui font definitions as the application.

Font licenses are embedded in Settings > About and accompany preview archives.
