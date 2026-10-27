//! Lightweight line coloring and Unicode-safe intraline emphasis.
use egui::{Color32, FontId, Pos2, TextFormat};
use std::ops::Range;

pub fn changed_ranges(a: &str, b: &str) -> (Range<usize>, Range<usize>) {
    let prefix: usize = a
        .chars()
        .zip(b.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let suffix: usize = a[prefix..]
        .chars()
        .rev()
        .zip(b[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    (prefix..a.len() - suffix, prefix..b.len() - suffix)
}

pub fn paint(
    painter: &egui::Painter,
    position: Pos2,
    text: &str,
    color: Color32,
    code: bool,
    emphasis: Option<&Range<usize>>,
    whitespace: bool,
) {
    let mut job = egui::text::LayoutJob::default();
    let append = |job: &mut egui::text::LayoutJob, range: Range<usize>, color: Color32| {
        let mut cuts = vec![range.start, range.end];
        if let Some(emphasis) = emphasis {
            for cut in [emphasis.start, emphasis.end] {
                if cut > range.start && cut < range.end {
                    cuts.push(cut);
                }
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        for pair in cuts.windows(2) {
            let selected = emphasis
                .is_some_and(|r| pair[0] >= r.start && pair[1] <= r.end && r.start != r.end);
            let raw = &text[pair[0]..pair[1]];
            let displayed = if whitespace {
                std::borrow::Cow::Owned(
                    raw.replace(' ', "\u{b7}")
                        .replace('\t', "\u{2192}   ")
                        .replace('\r', "\u{240d}"),
                )
            } else {
                std::borrow::Cow::Borrowed(raw)
            };
            job.append(
                &displayed,
                0.0,
                TextFormat {
                    font_id: FontId::monospace(13.0),
                    color: if selected {
                        crate::theme::text_primary()
                    } else {
                        color
                    },
                    background: if selected {
                        crate::theme::accent_blue().gamma_multiply(0.32)
                    } else {
                        Color32::TRANSPARENT
                    },
                    ..Default::default()
                },
            );
        }
    };
    if code && text.len() <= 8192 {
        static TOKENS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let tokens = TOKENS.get_or_init(|| regex::Regex::new(r#""(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|//.*|\#.*|\b[0-9][0-9A-Za-z_.]*\b|\b[A-Za-z_][A-Za-z_0-9]*\b"#).unwrap());
        let mut cursor = 0;
        for token in tokens.find_iter(text) {
            append(&mut job, cursor..token.start(), color);
            let value = token.as_str();
            let token_color = if value.starts_with(['"', '\'']) {
                crate::theme::accent_green()
            } else if value.starts_with("//") || value.starts_with('#') {
                crate::theme::text_muted()
            } else if value.as_bytes()[0].is_ascii_digit() {
                crate::theme::accent_yellow()
            } else if matches!(
                value,
                "fn" | "pub"
                    | "let"
                    | "mut"
                    | "const"
                    | "if"
                    | "else"
                    | "match"
                    | "return"
                    | "for"
                    | "while"
                    | "in"
                    | "impl"
                    | "struct"
                    | "enum"
                    | "use"
                    | "mod"
                    | "class"
                    | "def"
                    | "import"
                    | "from"
                    | "async"
                    | "await"
                    | "function"
                    | "var"
                    | "true"
                    | "false"
                    | "null"
                    | "None"
                    | "Some"
                    | "type"
                    | "interface"
                    | "export"
                    | "package"
                    | "func"
            ) {
                crate::theme::accent_blue()
            } else {
                color
            };
            append(&mut job, token.range(), token_color);
            cursor = token.end();
        }
        append(&mut job, cursor..text.len(), color);
    } else {
        append(&mut job, 0..text.len(), color);
    }
    let galley = painter.layout_job(job);
    painter.galley(
        Pos2::new(position.x, position.y - galley.size().y / 2.0),
        galley,
        color,
    );
}

pub fn supported(path: &str) -> bool {
    matches!(
        path.rsplit('.').next(),
        Some(
            "rs" | "js"
                | "jsx"
                | "ts"
                | "tsx"
                | "py"
                | "json"
                | "toml"
                | "yaml"
                | "yml"
                | "c"
                | "h"
                | "cpp"
                | "hpp"
                | "java"
                | "go"
                | "sh"
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_ranges_isolate_edits_on_character_boundaries() {
        assert_eq!(changed_ranges("let x = 1;", "let x = 2;"), (8..9, 8..9));
        assert_eq!(changed_ranges("same", "same"), (4..4, 4..4));
        assert_eq!(changed_ranges("", "new"), (0..0, 0..3));
        // Shared prefixes and suffixes never overlap on repeated text.
        assert_eq!(changed_ranges("aa", "aaa"), (2..2, 2..3));
        assert_eq!(changed_ranges("abcabc", "abc"), (3..6, 3..3));
        let (old, new) = ("naïve café ☕", "naïve cafe ☕");
        let (a, b) = changed_ranges(old, new);
        assert_eq!(&old[a], "é");
        assert_eq!(&new[b], "e");
    }

    #[test]
    fn syntax_coloring_uses_the_final_extension_only() {
        for path in ["src/main.rs", "web/app.test.tsx", "Cargo.toml", "ci.yml"] {
            assert!(supported(path), "{path}");
        }
        for path in ["README.md", "Makefile", "archive.tar.gz", "image.png"] {
            assert!(!supported(path), "{path}");
        }
    }
}
