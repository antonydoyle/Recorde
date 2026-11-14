//! Marketing captures of a large generated repository
//! (`scripts/generate_showcase_repos.py`). Also records how long the graph takes
//! to appear and to respond while scrolling deep into busy history.
use super::*;
use std::{error::Error, path::Path};

type Bounds = egui::accesskit::Rect;

fn tree(stream: &mut TcpStream) -> Result<egui::accesskit::TreeUpdate, Box<dyn Error>> {
    match request(stream, Request::GetTree)? {
        Response::Tree {
            accesskit: Some(tree),
            ..
        } => Ok(tree),
        _ => Err("Showcase needs a native accessibility tree".into()),
    }
}

/// Visible commit rows, top to bottom.
fn commit_rows(tree: &egui::accesskit::TreeUpdate) -> Vec<(String, Bounds)> {
    let mut rows: Vec<_> = tree
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == egui::accesskit::Role::ListBoxOption)
        .filter_map(|(id, node)| {
            let label = node.label()?;
            label.starts_with("Commit ").then_some(())?;
            Some((label.to_string(), visible_bounds(tree, *id, node, false)?))
        })
        .filter(|(_, bounds)| bounds.y1 - bounds.y0 > 4.0)
        .collect();
    rows.sort_by(|a, b| a.1.y0.total_cmp(&b.1.y0));
    rows
}

fn loaded_label(tree: &egui::accesskit::TreeUpdate) -> Option<String> {
    tree.nodes
        .iter()
        .filter_map(|(_, node)| node.label())
        .find(|label| label.contains("commits loaded") || label.contains("commits shown"))
        .map(|label| label.to_string())
}

fn selected_label(tree: &egui::accesskit::TreeUpdate) -> Option<String> {
    tree.nodes
        .iter()
        .filter(|(_, node)| node.role() == egui::accesskit::Role::ListBoxOption)
        .filter(|(_, node)| node.is_selected() == Some(true))
        .filter_map(|(_, node)| node.label())
        .find(|label| label.starts_with("Commit ") || label.starts_with("Uncommitted"))
        .map(|label| label.to_string())
}

fn centre(bounds: &Bounds) -> egui::Pos2 {
    egui::pos2(
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    )
}

pub fn run(stream: &mut TcpStream, directory: &Path) -> Result<(), Box<dyn Error>> {
    let started = Instant::now();
    let mut log = Vec::new();
    let deadline = started + Duration::from_secs(60);
    let rows = loop {
        let rows = commit_rows(&tree(stream)?);
        if rows.len() >= 10 {
            break rows;
        }
        if Instant::now() > deadline {
            return Err("The graph did not show commit rows within 60 seconds".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    log.push(format!(
        "graph rows visible {:.2}s after the inspector connected ({} rows on screen)",
        started.elapsed().as_secs_f32(),
        rows.len()
    ));
    // Progressive history keeps loading pages after the first rows appear.
    let mut loaded = String::new();
    let mut stable_since = Instant::now();
    while stable_since.elapsed() < Duration::from_secs(2) && Instant::now() < deadline {
        let now = loaded_label(&tree(stream)?).unwrap_or_default();
        if now != loaded {
            log.push(format!("{:.2}s: {now}", started.elapsed().as_secs_f32()));
            loaded = now;
            stable_since = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    capture(stream, directory, "showcase-01-overview")?;

    let rows = commit_rows(&tree(stream)?);
    let target = rows
        .iter()
        .find(|(label, _)| label.starts_with("Commit Add fuzzy matching"))
        .or_else(|| rows.get(1))
        .ok_or("No commit row to select")?
        .clone();
    tour::click_at(stream, centre(&target.1))?;
    let clicked = Instant::now();
    let mut seen = Vec::new();
    while clicked.elapsed() < Duration::from_secs(2) {
        let state = selected_label(&tree(stream)?).unwrap_or_else(|| "nothing".into());
        if seen.last() != Some(&state) {
            log.push(format!(
                "{:.0} ms after click, selected: {}",
                clicked.elapsed().as_secs_f32() * 1000.0,
                state.chars().take(70).collect::<String>()
            ));
            seen.push(state);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // Open the commit's file so the diff pane shows code, not an empty state.
    let current = tree(stream)?;
    let panel_left = target.1.x1;
    let file = current
        .nodes
        .iter()
        .filter(|(_, node)| {
            node.label()
                .is_some_and(|label| label.contains("command-palette.tsx"))
        })
        .filter_map(|(id, node)| visible_bounds(&current, *id, node, true))
        .filter(|bounds| bounds.x0 >= panel_left)
        // The file row is wider than its actions button.
        .max_by(|a, b| (a.x1 - a.x0).total_cmp(&(b.x1 - b.x0)));
    match file {
        Some(bounds) => {
            tour::click_at(stream, centre(&bounds))?;
            std::thread::sleep(Duration::from_millis(600));
        }
        None => log.push("changed file not found in the details panel".into()),
    }
    capture(stream, directory, "showcase-02-commit")?;

    // RECORDE_UI_SHOWCASE=features continues with one capture per feature.
    if std::env::var("RECORDE_UI_SHOWCASE").as_deref() == Ok("features") {
        features(stream, directory, &mut log);
        std::fs::write(directory.join("showcase.log"), log.join("\n"))?;
        return Ok(());
    }

    // Scroll deep into history. Each request waits for the app to process the
    // wheel event and report a new tree, so its latency approximates frame cost.
    let graph = centre(&rows[rows.len() / 2].1);
    let mut slowest = 0.0f32;
    let mut first_label = rows[0].0.clone();
    let mut moved = 0;
    for page in 1..=40 {
        let step = Instant::now();
        tour::scroll_at(stream, graph, -2400.0)?;
        let rows = commit_rows(&tree(stream)?);
        let elapsed = step.elapsed().as_secs_f32();
        slowest = slowest.max(elapsed);
        if let Some((label, _)) = rows.first() {
            if *label != first_label {
                moved += 1;
                first_label = label.clone();
            }
        }
        if page % 10 == 0 {
            log.push(format!(
                "scroll page {page}: {:.0} ms, {} rows on screen",
                elapsed * 1000.0,
                rows.len()
            ));
        }
    }
    log.push(format!(
        "40 scroll steps moved the view {moved} times; slowest step {:.0} ms",
        slowest * 1000.0
    ));
    capture(stream, directory, "showcase-03-deep-history")?;
    std::fs::write(directory.join("showcase.log"), log.join("\n"))?;
    Ok(())
}

/// The visible node matching `wanted` with the widest bounds (a row rather
/// than a button inside it), lowest on screen first when `lowest` is set.
fn find(
    stream: &mut TcpStream,
    wanted: impl Fn(&egui::accesskit::Node) -> bool,
    lowest: bool,
) -> Result<Bounds, Box<dyn Error>> {
    let tree = tree(stream)?;
    let mut found: Vec<Bounds> = tree
        .nodes
        .iter()
        .filter(|(_, node)| wanted(node))
        .filter_map(|(id, node)| visible_bounds(&tree, *id, node, true))
        .filter(|bounds| bounds.x1 - bounds.x0 > 2.0 && bounds.y1 - bounds.y0 > 2.0)
        .collect();
    if lowest {
        found.sort_by(|a, b| b.y0.total_cmp(&a.y0));
        return found
            .into_iter()
            .next()
            .ok_or_else(|| "no matching node".into());
    }
    found
        .into_iter()
        .max_by(|a, b| (a.x1 - a.x0).total_cmp(&(b.x1 - b.x0)))
        .ok_or_else(|| "no matching node".into())
}

fn label_is(text: &'static str) -> impl Fn(&egui::accesskit::Node) -> bool {
    move |node| node.label().as_deref() == Some(text)
}

fn label_starts(text: &'static str) -> impl Fn(&egui::accesskit::Node) -> bool {
    move |node| node.label().is_some_and(|label| label.starts_with(text))
}

fn right_click(stream: &mut TcpStream, pos: egui::Pos2) -> Result<(), Box<dyn Error>> {
    for pressed in [true, false] {
        apply_events(
            stream,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        )?;
    }
    Ok(())
}

fn key(
    stream: &mut TcpStream,
    key: egui::Key,
    modifiers: egui::Modifiers,
) -> Result<(), Box<dyn Error>> {
    apply_events(
        stream,
        [true, false]
            .into_iter()
            .map(|pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            })
            .collect(),
    )
}

fn type_text(stream: &mut TcpStream, text: &str) -> Result<(), Box<dyn Error>> {
    apply_events(stream, vec![egui::Event::Text(text.to_owned())])
}

fn settle() {
    std::thread::sleep(Duration::from_millis(700));
}

fn hover_on(
    stream: &mut TcpStream,
    wanted: impl Fn(&egui::accesskit::Node) -> bool,
) -> Result<(), Box<dyn Error>> {
    let bounds = find(stream, wanted, false)?;
    apply_events(stream, vec![egui::Event::PointerMoved(centre(&bounds))])?;
    settle();
    Ok(())
}

fn click_on(
    stream: &mut TcpStream,
    wanted: impl Fn(&egui::accesskit::Node) -> bool,
    lowest: bool,
) -> Result<(), Box<dyn Error>> {
    let bounds = find(stream, wanted, lowest)?;
    tour::click_at(stream, centre(&bounds))?;
    settle();
    Ok(())
}

fn right_click_on(
    stream: &mut TcpStream,
    wanted: impl Fn(&egui::accesskit::Node) -> bool,
) -> Result<(), Box<dyn Error>> {
    let bounds = find(stream, wanted, false)?;
    right_click(stream, centre(&bounds))?;
    settle();
    Ok(())
}

/// Product-page captures, one per feature. Each step is best effort and logged,
/// so one unreachable control does not hide the remaining captures. Steps only
/// stage, type and open editors in the disposable fixture copies; nothing is
/// committed, rebased or resolved.
fn features(stream: &mut TcpStream, directory: &Path, log: &mut Vec<String>) {
    let step = |log: &mut Vec<String>, name: &str, result: Result<(), Box<dyn Error>>| match result
    {
        Ok(()) => log.push(format!("ok   {name}")),
        Err(error) => log.push(format!("FAIL {name}: {error}")),
    };
    step(
        log,
        "commit composer",
        (|| {
            click_on(stream, label_starts("Uncommitted changes"), false)?;
            // Row actions appear on hover, so point at the button before clicking it.
            hover_on(stream, label_is("Stage CHANGELOG.md"))?;
            click_on(stream, label_is("Stage CHANGELOG.md"), false)?;
            click_on(
                stream,
                label_is("src/features/command-palette/command-palette.tsx"),
                false,
            )?;
            click_on(
                stream,
                |node| {
                    node.placeholder()
                        .is_some_and(|text| text.starts_with("Short summary"))
                },
                false,
            )?;
            type_text(stream, "Score command results with fuzzy matching")?;
            settle();
            capture(stream, directory, "feature-02-commit")?;
            key(stream, egui::Key::Escape, egui::Modifiers::NONE)
        })(),
    );
    step(
        log,
        "command palette",
        (|| {
            click_on(stream, label_starts("Commit Add fuzzy matching"), false)?;
            key(stream, egui::Key::K, egui::Modifiers::COMMAND)?;
            settle();
            type_text(stream, "branch")?;
            settle();
            capture(stream, directory, "feature-03-command-palette")?;
            key(stream, egui::Key::Escape, egui::Modifiers::NONE)?;
            settle();
            Ok(())
        })(),
    );
    step(
        log,
        "interactive rebase",
        (|| {
            right_click_on(stream, label_starts("Commit Add keyboard navigation"))?;
            click_on(stream, label_is("Interactive Rebase from here…"), false)?;
            capture(stream, directory, "feature-04-interactive-rebase")?;
            click_on(stream, label_is("Cancel"), true)
        })(),
    );
    step(
        log,
        "themes",
        (|| {
            click_on(stream, label_is("Open settings"), false)?;
            click_on(stream, label_is("Appearance"), true)?;
            click_on(
                stream,
                |node| {
                    node.label().as_deref() == Some("Colour palette")
                        && node.value().as_deref() == Some("Tokyo Night")
                },
                false,
            )?;
            capture(stream, directory, "feature-06-themes")?;
            key(stream, egui::Key::Escape, egui::Modifiers::NONE)?;
            settle();
            click_on(stream, label_is("Close"), true)
        })(),
    );
    step(
        log,
        "conflict editor",
        (|| {
            click_on(stream, label_is("design-system"), false)?;
            std::thread::sleep(Duration::from_secs(3));
            right_click_on(stream, |node| {
                node.label().is_some_and(|label| label.contains("color.ts"))
            })?;
            click_on(stream, label_is("Resolve in Recorde…"), false)?;
            std::thread::sleep(Duration::from_secs(1));
            capture(stream, directory, "feature-05-conflicts")?;
            click_on(stream, label_is("Close window"), false)?;
            settle();
            Ok(())
        })(),
    );
}
