//! Offline help, compiled from the same Markdown published in the repository wiki.
use egui::{RichText, Ui};

struct Page {
    id: &'static str,
    title: &'static str,
    markdown: &'static str,
}

macro_rules! page {
    ($id:literal, $title:literal) => {
        Page {
            id: $id,
            title: $title,
            markdown: include_str!(concat!("../../docs/wiki/", $id, ".md")),
        }
    };
}

const PAGES: &[Page] = &[
    page!("Home", "Welcome to Recorde"),
    page!("User-Guide", "Getting started"),
    page!("Feature-Reference", "Feature reference"),
    page!("Branching-and-Git-Flow", "Branches and Git Flow"),
    page!("Recovery", "Undo, redo and recovery"),
    page!("Accounts-and-Tools", "Accounts, identities and tools"),
    page!("Keyboard-Shortcuts", "Keyboard shortcuts"),
    page!("Themes-and-Accessibility", "Themes and accessibility"),
    page!("Troubleshooting", "Troubleshooting"),
    page!("Advanced-History", "Advanced history"),
    page!("Provider-Integrations", "Provider integrations"),
];

/// A self-contained, offline documentation window. The application should pause
/// global repository shortcuts while `open` to keep reading and searching safe.
#[derive(Default)]
pub struct HelpBrowser {
    pub open: bool,
    selected: usize,
    query: String,
    back: Vec<usize>,
    forward: Vec<usize>,
    focus_search: bool,
    reset_scroll: bool,
    #[cfg(test)]
    topic_rects: Vec<egui::Rect>,
    #[cfg(test)]
    article_rect: Option<egui::Rect>,
}

impl HelpBrowser {
    /// Accepts a wiki stem, e.g. `User-Guide`, or its `.md` filename.
    pub fn open_page(&mut self, id: &str) {
        self.open = true;
        self.query.clear();
        self.navigate(page_index(id).unwrap_or(0));
        self.focus_search = true;
    }

    fn navigate(&mut self, index: usize) {
        if self.selected != index {
            self.back.push(self.selected);
            self.forward.clear();
            self.selected = index;
        }
        self.reset_scroll = true;
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        #[cfg(test)]
        {
            self.topic_rects.clear();
            self.article_rect = None;
        }
        if !self.open {
            return;
        }
        let mut open = true;
        let mut close = false;
        let available = ctx.content_rect().size();
        egui::Window::new("Recorde Help")
            .id(egui::Id::new("recorde_help"))
            .open(&mut open)
            .default_size(egui::vec2(880.0, 640.0).min(available * 0.9))
            .min_size(egui::vec2(300.0, 240.0))
            .max_size(available * 0.96)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(!self.back.is_empty(), egui::Button::new("Back")).clicked() {
                        if let Some(index) = self.back.pop() {
                            self.forward.push(self.selected);
                            self.selected = index;
                            self.reset_scroll = true;
                        }
                    }
                    if ui.add_enabled(!self.forward.is_empty(), egui::Button::new("Forward")).clicked() {
                        if let Some(index) = self.forward.pop() {
                            self.back.push(self.selected);
                            self.selected = index;
                            self.reset_scroll = true;
                        }
                    }
                    if ui.button("Home").clicked() {
                        self.query.clear();
                        self.navigate(0);
                    }
                    close = ui.button("Close").clicked();
                });
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text("Search all help topics...")
                        .desired_width(f32::INFINITY),
                );
                if std::mem::take(&mut self.focus_search) {
                    search.request_focus();
                }
                ui.label(RichText::new("Available offline. These pages are also in docs/wiki in the repository.").small().weak());
                ui.separator();
                let results = matching_pages(&self.query);
                let mut target = None;
                let height = ui.available_height().max(120.0);
                if ui.available_width() < 620.0 {
                    egui::ComboBox::from_id_salt("help_topics_compact")
                        .selected_text(PAGES[self.selected].title)
                        .width(ui.available_width().min(340.0))
                        .show_ui(ui, |ui| {
                            for &index in &results {
                                if ui.selectable_label(self.selected == index, PAGES[index].title).clicked() {
                                    target = Some(index);
                                }
                            }
                        });
                    if results.is_empty() {
                        ui.label("No matching topics. Try a shorter search.");
                    }
                    self.article(ui, &mut target, height - 32.0);
                } else {
                    ui.horizontal_top(|ui| {
                        ui.allocate_ui_with_layout(egui::vec2(210.0, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.set_width(210.0);
                            egui::ScrollArea::vertical().id_salt("help_topics").max_height(height).auto_shrink([false, false]).show(ui, |ui| {
                                ui.label(RichText::new(format!("{} topics", results.len())).small().weak());
                                for &index in &results {
                                    let response = ui.add(egui::Button::selectable(self.selected == index, PAGES[index].title).wrap());
                                    #[cfg(test)]
                                    self.topic_rects.push(response.rect);
                                    if response.clicked() {
                                        target = Some(index);
                                    }
                                }
                                if results.is_empty() {
                                    ui.label("No matching topics. Try a shorter search or clear the search field.");
                                }
                            });
                        });
                        ui.separator();
                        ui.vertical(|ui| self.article(ui, &mut target, height));
                    });
                }
                if let Some(index) = target {
                    self.navigate(index);
                }
            });
        self.open = open && !close;
    }

    fn article(&mut self, ui: &mut Ui, target: &mut Option<usize>, height: f32) {
        #[cfg(test)]
        {
            self.article_rect = Some(ui.available_rect_before_wrap());
        }
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(("help_article", self.selected))
            .auto_shrink([false, false])
            .max_height(height.max(80.0));
        if std::mem::take(&mut self.reset_scroll) {
            scroll = scroll.vertical_scroll_offset(0.0);
        }
        scroll.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            render_markdown(ui, PAGES[self.selected].markdown, target);
        });
    }
}

fn page_index(link: &str) -> Option<usize> {
    let path = link.split('#').next().unwrap_or(link);
    let id = path.strip_suffix(".md").unwrap_or(path);
    PAGES.iter().position(|page| page.id == id)
}

fn matching_pages(query: &str) -> Vec<usize> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut results: Vec<usize> = PAGES
        .iter()
        .enumerate()
        .filter_map(|(i, page)| {
            let text = format!("{} {}", page.title, page.markdown).to_lowercase();
            words.iter().all(|word| text.contains(word)).then_some(i)
        })
        .collect();
    // A matching title should be easier to discover than a passing mention.
    results.sort_by_key(|&i| {
        let title = PAGES[i].title.to_lowercase();
        !words.iter().all(|word| title.contains(word))
    });
    results
}

#[derive(Debug, PartialEq)]
enum Block {
    Heading(usize, String),
    Text(String),
    Item(String, String),
    Code(String),
    Table(Vec<Vec<String>>),
}

fn blocks(markdown: &str) -> Vec<Block> {
    let mut result = Vec::new();
    let mut lines = markdown.lines().peekable();
    while let Some(raw) = lines.next() {
        let line = raw.trim();
        if line.is_empty() || line == "---" {
            continue;
        }
        if line.starts_with("```") {
            let mut code = String::new();
            for line in lines.by_ref() {
                if line.trim().starts_with("```") {
                    break;
                }
                code.push_str(line);
                code.push('\n');
            }
            result.push(Block::Code(code.trim_end().to_string()));
        } else if line.starts_with('|') {
            let mut rows = vec![table_cells(line)];
            while lines
                .peek()
                .is_some_and(|line| line.trim().starts_with('|'))
            {
                let row = table_cells(lines.next().unwrap().trim());
                if !row
                    .iter()
                    .all(|cell| cell.chars().all(|c| "-: ".contains(c)))
                {
                    rows.push(row);
                }
            }
            result.push(Block::Table(rows));
        } else if let Some((prefix, text)) = line
            .split_once(' ')
            .filter(|(prefix, _)| prefix.chars().all(|c| c == '#'))
        {
            result.push(Block::Heading(prefix.len(), text.to_owned()));
        } else {
            let (bullet, first) = if let Some(text) = line.strip_prefix("- ") {
                (Some("•".to_string()), text)
            } else if let Some((number, text)) = line.split_once(". ").filter(|(number, _)| {
                !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
            }) {
                (Some(format!("{number}.")), text)
            } else {
                (None, line.strip_prefix("> ").unwrap_or(line))
            };
            let mut paragraph = first.to_string();
            while let Some(next) = lines.peek() {
                let next = next.trim();
                if next.is_empty()
                    || next.starts_with(['#', '|', '>'])
                    || next.starts_with("```")
                    || next.starts_with("- ")
                    || next.split_once(". ").is_some_and(|(n, _)| {
                        !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())
                    })
                {
                    break;
                }
                paragraph.push(' ');
                paragraph.push_str(lines.next().unwrap().trim());
            }
            result.push(match bullet {
                Some(bullet) => Block::Item(bullet, paragraph),
                None => Block::Text(paragraph),
            });
        }
    }
    result
}

fn table_cells(line: &str) -> Vec<String> {
    line.trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_owned())
        .collect()
}

/// Small, deterministic inline formatter for the wiki's emphasis, code and links.
/// Unsupported markup remains text; embedded HTML or executable URLs never run.
fn inline(text: &str) -> (String, Vec<(String, String)>) {
    let mut plain = String::new();
    let mut links = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(label_end) = rest.strip_prefix('[').and_then(|s| s.find("](")) {
            let label_end = label_end + 1;
            let destination = &rest[label_end + 2..];
            if let Some(end) = destination.find(')') {
                let label = &rest[1..label_end];
                plain.push_str(label);
                links.push((label.to_string(), destination[..end].to_string()));
                rest = &destination[end + 1..];
                continue;
            }
        }
        if let Some(after) = rest.strip_prefix("**") {
            if let Some(end) = after.find("**") {
                plain.push_str(&after[..end]);
                rest = &after[end + 2..];
                continue;
            }
        }
        if let Some(after) = rest.strip_prefix('`') {
            if let Some(end) = after.find('`') {
                plain.push_str(&after[..end]);
                rest = &after[end + 1..];
                continue;
            }
        }
        let ch = rest.chars().next().unwrap();
        plain.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    (plain, links)
}

fn render_text(ui: &mut Ui, text: &str, target: &mut Option<usize>) {
    let (plain, links) = inline(text);
    ui.add(egui::Label::new(plain).wrap().selectable(true));
    if !links.is_empty() {
        ui.horizontal_wrapped(|ui| {
            for (label, destination) in links {
                if let Some(index) = page_index(&destination) {
                    if ui.link(label).clicked() {
                        *target = Some(index);
                    }
                } else if let Some(url) = external_url(&destination) {
                    ui.hyperlink_to(label, url)
                        .on_hover_text("Open in your browser");
                }
            }
        });
    }
}

fn external_url(destination: &str) -> Option<String> {
    if destination.starts_with("https://") {
        Some(destination.to_string())
    } else if !destination.contains([':', '\\']) && !destination.starts_with(['/', '#']) {
        url::Url::parse("https://github.com/antonydoyle/Recorde/blob/main/docs/wiki/")
            .ok()?
            .join(destination)
            .ok()
            .map(|url| url.to_string())
    } else {
        None
    }
}

fn render_markdown(ui: &mut Ui, markdown: &str, target: &mut Option<usize>) {
    for (index, block) in blocks(markdown).into_iter().enumerate() {
        match block {
            Block::Heading(level, text) => {
                if level > 1 {
                    ui.add_space(10.0);
                }
                ui.label(RichText::new(inline(&text).0).strong().size(if level == 1 {
                    25.0
                } else if level == 2 {
                    19.0
                } else {
                    16.0
                }));
            }
            Block::Text(text) => render_text(ui, &text, target),
            Block::Item(bullet, text) => {
                ui.horizontal_top(|ui| {
                    ui.label(bullet);
                    ui.vertical(|ui| render_text(ui, &text, target));
                });
            }
            Block::Code(code) => {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&code).monospace())
                            .wrap()
                            .selectable(true),
                    );
                    if ui.small_button("Copy code").clicked() {
                        ui.ctx().copy_text(code);
                    }
                });
            }
            Block::Table(rows) => {
                // Stack each record on narrow windows; wide capability matrices
                // otherwise force the entire help window beyond the display.
                if let Some(headers) = rows.first() {
                    for row in rows.iter().skip(1) {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.push_id(index, |ui| {
                                for (column, cell) in row.iter().enumerate() {
                                    let label =
                                        headers.get(column).map(String::as_str).unwrap_or("");
                                    render_text(ui, &format!("{label}: {cell}"), target);
                                }
                            });
                        });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_matches_all_words_and_prioritizes_topic_titles() {
        assert_eq!(
            matching_pages("KEYBOARD").first(),
            page_index("Keyboard-Shortcuts").as_ref()
        );
        assert!(matching_pages("unfindable-word-123").is_empty());
        assert!(!matching_pages("recovery ignored").is_empty());
        assert_eq!(matching_pages("  ").len(), PAGES.len());
    }

    #[test]
    fn embedded_pages_have_unique_names_and_resolvable_wiki_links() {
        let mut ids = std::collections::HashSet::new();
        for page in PAGES {
            assert!(ids.insert(page.id));
            assert!(page.markdown.starts_with("# "), "{} has no title", page.id);
            assert_eq!(
                page_index(&format!("{}.md#section", page.id)),
                page_index(page.id)
            );
            for line in page.markdown.lines() {
                for (_, link) in inline(line).1 {
                    if link.ends_with(".md") && !link.contains(['/', ':']) {
                        let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("docs/wiki")
                            .join(link);
                        assert!(file.is_file(), "{} contains a broken wiki link", page.id);
                    }
                }
            }
        }
    }

    #[test]
    fn navigation_keeps_history_and_unknown_pages_open_home() {
        let mut help = HelpBrowser::default();
        help.open_page("Recovery.md");
        assert!(help.open);
        assert_eq!(help.selected, page_index("Recovery").unwrap());
        assert_eq!(help.back, vec![0]);
        help.open_page("missing");
        assert_eq!(help.selected, 0);
        assert_eq!(help.back.len(), 2);
    }

    #[test]
    fn parser_handles_wrapped_steps_tables_code_and_unicode_links() {
        let parsed = blocks("# Title\n\n1. One\n   continued\n2. Two\n\n```sh\ngit status\n```\n\n| A | B |\n| --- | --- |\n| x | y |\n");
        assert_eq!(parsed[1], Block::Item("1.".into(), "One continued".into()));
        assert_eq!(parsed[3], Block::Code("git status".into()));
        assert_eq!(
            parsed[4],
            Block::Table(vec![
                vec!["A".into(), "B".into()],
                vec!["x".into(), "y".into()]
            ])
        );
        assert_eq!(
            inline("Read **help** with `git` and [café](Recovery.md)").0,
            "Read help with git and café"
        );
        assert_eq!(
            inline("[café](Recovery.md)").1,
            vec![("café".into(), "Recovery.md".into())]
        );
        assert!(external_url("javascript:alert(1)").is_none());
        assert!(external_url("file:///secret").is_none());
        assert!(external_url("//untrusted.example").is_none());
    }

    #[test]
    fn all_embedded_topics_render_at_compact_and_desktop_sizes() {
        for width in [460.0, 1100.0] {
            let ctx = egui::Context::default();
            let mut help = HelpBrowser::default();
            for page in PAGES {
                help.open_page(page.id);
                // The second frame exercises the settled window size/layout.
                for _ in 0..2 {
                    let mut output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, 760.0),
                            )),
                            ..Default::default()
                        },
                        |ui| help.show(ui.ctx()),
                    );
                    output.textures_delta.clear();
                    assert!(!output.shapes.is_empty(), "{} rendered no content", page.id);
                }
            }
        }
    }

    #[test]
    fn desktop_topics_stack_beside_a_readable_article() {
        let ctx = egui::Context::default();
        let mut help = HelpBrowser::default();
        help.open_page("User-Guide");
        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 760.0),
                    )),
                    ..Default::default()
                },
                |ui| help.show(ui.ctx()),
            );
            output.textures_delta.clear();
        }
        assert_eq!(help.topic_rects.len(), PAGES.len());
        for pair in help.topic_rects.windows(2) {
            assert!(
                pair[1].top() >= pair[0].bottom(),
                "topics must stack vertically"
            );
            assert!((pair[0].left() - pair[1].left()).abs() < 1.0);
        }
        let article = help.article_rect.expect("article area");
        assert!(
            article.width() >= 400.0,
            "article width: {}",
            article.width()
        );
        for topic in &help.topic_rects {
            assert!(topic.width() >= 80.0, "topic squeezed: {topic:?}");
            assert!(topic.height() <= 60.0, "topic wraps excessively: {topic:?}");
            assert!(topic.right() < article.left(), "article overlaps topics");
        }
    }
}
