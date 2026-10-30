//! Visual tour for design review: captures the main screens, menus and every
//! Settings tab of the isolated smoke fixture. Steps are best effort, so one
//! renamed control does not hide the remaining captures; failures are logged.
use super::*;
use std::{error::Error, path::Path};

fn tree(stream: &mut TcpStream) -> Result<egui::accesskit::TreeUpdate, Box<dyn Error>> {
    match request(stream, Request::GetTree)? {
        Response::Tree {
            accesskit: Some(tree),
            ..
        } => Ok(tree),
        _ => Err("Tour needs a native accessibility tree".into()),
    }
}

fn press(stream: &mut TcpStream, key: egui::Key) -> Result<(), Box<dyn Error>> {
    apply_events(
        stream,
        [true, false]
            .into_iter()
            .map(|pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            })
            .collect(),
    )
}

fn click_at(stream: &mut TcpStream, pos: egui::Pos2) -> Result<(), Box<dyn Error>> {
    for pressed in [true, false] {
        apply_events(
            stream,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        )?;
    }
    Ok(())
}

/// Clicks the topmost visible node whose label starts with `prefix`, any role.
fn click_prefix(stream: &mut TcpStream, prefix: &str) -> Result<(), Box<dyn Error>> {
    let tree = tree(stream)?;
    let bounds = tree
        .nodes
        .iter()
        .filter(|(_, node)| node.label().is_some_and(|label| label.starts_with(prefix)))
        .filter_map(|(id, node)| visible_bounds(&tree, *id, node, true))
        .min_by(|a, b| a.y0.total_cmp(&b.y0))
        .ok_or_else(|| format!("No visible node labelled {prefix:?}"))?;
    click_at(
        stream,
        egui::pos2(
            ((bounds.x0 + bounds.x1) / 2.0) as f32,
            ((bounds.y0 + bounds.y1) / 2.0) as f32,
        ),
    )
}

/// Clicks the lowest visible node labelled exactly `label`. Settings navigation
/// sits below the menu bar, so this picks "Repository" the page over the menu.
fn click_lowest(stream: &mut TcpStream, label: &str) -> Result<(), Box<dyn Error>> {
    let tree = tree(stream)?;
    let bounds = tree
        .nodes
        .iter()
        .filter(|(_, node)| node.label().as_deref() == Some(label))
        .filter_map(|(id, node)| visible_bounds(&tree, *id, node, true))
        .max_by(|a, b| a.y0.total_cmp(&b.y0))
        .ok_or_else(|| format!("No visible node labelled {label:?}"))?;
    click_at(
        stream,
        egui::pos2(
            ((bounds.x0 + bounds.x1) / 2.0) as f32,
            ((bounds.y0 + bounds.y1) / 2.0) as f32,
        ),
    )
}

fn scroll_at(stream: &mut TcpStream, pos: egui::Pos2, delta: f32) -> Result<(), Box<dyn Error>> {
    apply_events(
        stream,
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, delta),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    )
}

fn step(log: &mut Vec<String>, name: &str, result: Result<(), Box<dyn Error>>) {
    match result {
        Ok(()) => log.push(format!("ok   {name}")),
        Err(error) => log.push(format!("FAIL {name}: {error}")),
    }
}

pub fn run(stream: &mut TcpStream, directory: &Path) -> Result<(), Box<dyn Error>> {
    let mut log = Vec::new();
    step(&mut log, "main", capture(stream, directory, "tour-01-main"));
    step(
        &mut log,
        "wip",
        (|| {
            click_prefix(stream, "Working tree: uncommitted changes")?;
            capture(stream, directory, "tour-02-wip-selected")
        })(),
    );
    step(
        &mut log,
        "file",
        (|| {
            click_prefix(stream, "hello.txt")?;
            capture(stream, directory, "tour-03-file-selected")
        })(),
    );
    step(
        &mut log,
        "batch",
        (|| {
            click_prefix(stream, "Select hello.txt")?;
            capture(stream, directory, "tour-03b-batch-selected")?;
            click_prefix(stream, "Select hello.txt")
        })(),
    );
    step(
        &mut log,
        "commit",
        (|| {
            click_prefix(stream, "Commit second fixture")?;
            capture(stream, directory, "tour-04-commit-selected")
        })(),
    );
    for (index, menu) in ["File", "Edit", "View", "Repository", "Branch", "Help"]
        .into_iter()
        .enumerate()
    {
        step(
            &mut log,
            menu,
            (|| {
                click_label(stream, menu)?;
                capture(stream, directory, &format!("tour-1{index}-menu-{menu}"))?;
                press(stream, egui::Key::Escape)
            })(),
        );
    }
    step(
        &mut log,
        "open settings",
        (|| {
            click_label(stream, "File")?;
            click_label(stream, "Settings")?;
            expect_label(stream, "Accounts & SSH", true)
        })(),
    );
    for (index, tab) in [
        "General",
        "Appearance",
        "Accounts & SSH",
        "Git",
        "Tools",
        "Keyboard",
        "Repository",
        "Updates",
        "About",
    ]
    .into_iter()
    .enumerate()
    {
        let slug: String = tab.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        step(
            &mut log,
            tab,
            (|| {
                click_lowest(stream, tab)?;
                capture(
                    stream,
                    directory,
                    &format!("tour-2{index:02}-settings-{slug}"),
                )?;
                // Capture further down long tabs. Scroll over the page column,
                // found from the navigation: node bounds include scrolled-out
                // content, so they cannot locate the window.
                let navigation = tree(stream)?
                    .nodes
                    .iter()
                    .filter(|(_, node)| node.label().as_deref() == Some("General"))
                    .filter_map(|(_, node)| node.bounds())
                    .max_by(|a, b| a.y0.total_cmp(&b.y0))
                    .ok_or("No settings navigation")?;
                let centre = egui::pos2(
                    (navigation.x1 + 360.0) as f32,
                    (navigation.y0 + 220.0) as f32,
                );
                for page in 1..=5 {
                    scroll_at(stream, centre, -400.0)?;
                    capture(
                        stream,
                        directory,
                        &format!("tour-2{index:02}-settings-{slug}-p{page}"),
                    )?;
                }
                Ok(())
            })(),
        );
    }
    step(
        &mut log,
        "Add account",
        (|| {
            click_lowest(stream, "Accounts & SSH")?;
            // Earlier captures scrolled this page; start from the top.
            let navigation = tree(stream)?
                .nodes
                .iter()
                .filter(|(_, node)| node.label().as_deref() == Some("General"))
                .filter_map(|(_, node)| node.bounds())
                .max_by(|a, b| a.y0.total_cmp(&b.y0))
                .ok_or("No settings navigation")?;
            let page = egui::pos2(
                (navigation.x1 + 360.0) as f32,
                (navigation.y0 + 220.0) as f32,
            );
            scroll_at(stream, page, 4000.0)?;
            click_prefix(stream, "Add account")?;
            capture(stream, directory, "tour-300-add-account")?;
            scroll_at(stream, page, -400.0)?;
            capture(stream, directory, "tour-301-add-account-sign-in")
        })(),
    );
    std::fs::write(directory.join("tour.log"), log.join("\n"))?;
    Ok(())
}
