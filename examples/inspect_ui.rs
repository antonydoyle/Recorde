//! Development-only AccessKit and keyboard smoke client for a native Recorde window.
//! The app must be built with `ui-inspection` and bind its inspection port to loopback.
//! `--review` runs commands and edits disposable fixture files; `--clean-tracked`
//! also exercises reset/recovery and Git Flow. Use only the isolated repositories
//! created by gui_smoke.py/windows_smoke.ps1, never a personal working repository.
use egui_inspection::{read_message, write_message, Request, Response};
use std::{
    collections::HashSet,
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    time::{Duration, Instant},
};

#[path = "inspection/file_editor.rs"]
mod file_editor;
#[path = "inspection/gitflow.rs"]
mod gitflow;
#[path = "inspection/graph_actions.rs"]
mod graph_actions;
#[path = "inspection/recovery.rs"]
mod recovery;
#[path = "inspection/showcase.rs"]
mod showcase;
#[path = "inspection/tour.rs"]
mod tour;

fn request(
    stream: &mut TcpStream,
    request: Request,
) -> Result<Response, Box<dyn std::error::Error>> {
    write_message(&mut *stream, &request)?;
    Ok(read_message(stream)?)
}

fn apply_events(
    stream: &mut TcpStream,
    events: Vec<egui::Event>,
) -> Result<(), Box<dyn std::error::Error>> {
    match request(stream, Request::ApplyEvents { events })? {
        Response::Done => Ok(()),
        response => Err(format!("Input was not applied: {response:?}").into()),
    }
}

fn visible_bounds(
    tree: &egui::accesskit::TreeUpdate,
    id: egui::accesskit::NodeId,
    node: &egui::accesskit::Node,
    require_enabled: bool,
) -> Option<egui::accesskit::Rect> {
    if node.is_hidden() || (require_enabled && node.is_disabled()) {
        return None;
    }
    let mut bounds = node.bounds()?;
    let mut cursor = id;
    let mut visited = HashSet::new();
    while visited.insert(cursor) {
        let Some((parent_id, parent)) = tree
            .nodes
            .iter()
            .find(|(_, parent)| parent.children().contains(&cursor))
        else {
            break;
        };
        if parent.is_hidden() || (require_enabled && parent.is_disabled()) {
            return None;
        }
        if parent.clips_children() {
            if let Some(clip) = parent.bounds() {
                bounds.x0 = bounds.x0.max(clip.x0);
                bounds.y0 = bounds.y0.max(clip.y0);
                bounds.x1 = bounds.x1.min(clip.x1);
                bounds.y1 = bounds.y1.min(clip.y1);
            }
        }
        cursor = *parent_id;
    }
    ([bounds.x0, bounds.y0, bounds.x1, bounds.y1]
        .iter()
        .all(|value| value.is_finite())
        && bounds.x1 > bounds.x0
        && bounds.y1 > bounds.y0)
        .then_some(bounds)
}

fn has_visible_text(tree: &egui::accesskit::TreeUpdate, text: &str) -> bool {
    tree.nodes.iter().any(|(id, node)| {
        (node.label() == Some(text) || node.value() == Some(text))
            && visible_bounds(tree, *id, node, false).is_some()
    })
}

fn clickable_bounds(
    tree: &egui::accesskit::TreeUpdate,
    label: &str,
) -> Result<Option<egui::accesskit::Rect>, String> {
    use egui::accesskit::Role;
    let mut candidates = Vec::new();
    for (id, node) in &tree.nodes {
        if node.label() != Some(label)
            || node.is_disabled()
            || node.is_hidden()
            || !matches!(
                node.role(),
                Role::Button | Role::MenuItem | Role::Tab | Role::ListBoxOption
            )
        {
            continue;
        }
        // Bounds are logical points in the inspection protocol. Exclude clipped
        // descendants and hidden/disabled containers, rather than clicking an
        // offscreen text link which happens to share a visible button's label.
        if let Some(bounds) = visible_bounds(tree, *id, node, true) {
            candidates.push(bounds);
        }
    }
    match candidates.as_slice() {
        [] => Ok(None),
        [bounds] => Ok(Some(*bounds)),
        _ => Err(format!(
            "Accessible control is ambiguous: {label} ({} visible interactive matches)",
            candidates.len()
        )),
    }
}

fn click_label(stream: &mut TcpStream, label: &str) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(3);
    let bounds = loop {
        let Response::Tree {
            accesskit: Some(tree),
            ..
        } = request(stream, Request::GetTree)?
        else {
            return Err("Missing accessibility tree".into());
        };
        if let Some(bounds) = clickable_bounds(&tree, label)? {
            break bounds;
        }
        if Instant::now() >= deadline {
            return Err(format!("Visible interactive control is missing: {label}").into());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let pos = egui::pos2(
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    );
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

fn expect_label(
    stream: &mut TcpStream,
    label: &str,
    present: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let Response::Tree {
            accesskit: Some(tree),
            ..
        } = request(stream, Request::GetTree)?
        else {
            return Err("Missing accessibility tree".into());
        };
        if has_visible_text(&tree, label) == present {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Expected visible accessible text {label:?} to be {}",
                if present { "present" } else { "absent" }
            )
            .into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn parse_flags(args: impl Iterator<Item = String>) -> Result<(bool, bool, bool), String> {
    let mut graph = false;
    let mut review = false;
    let mut clean_tracked = false;
    for arg in args {
        match arg.as_str() {
            "--graph" => graph = true,
            "--review" => review = true,
            "--clean-tracked" => clean_tracked = true,
            _ => return Err(format!("Unknown inspection option: {arg}")),
        }
    }
    if clean_tracked && !review {
        return Err("--clean-tracked requires --review".into());
    }
    Ok((graph, review, clean_tracked))
}

fn submit_command(stream: &mut TcpStream, command: &str) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let Response::Tree {
            accesskit: Some(tree),
            ..
        } = request(stream, Request::GetTree)?
        else {
            return Err("Missing command input tree".into());
        };
        if tree.nodes.iter().any(|(id, node)| {
            *id == tree.focus
                && node.role() == egui::accesskit::Role::TextInput
                && node.placeholder() == Some("Press Enter to run (for example, git status)")
                && visible_bounds(&tree, *id, node, true).is_some()
        }) {
            break;
        }
        if Instant::now() >= deadline {
            return Err("Command input did not receive accessible keyboard focus".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    apply_events(stream, vec![egui::Event::Text(command.to_owned())])?;
    apply_events(
        stream,
        [true, false]
            .into_iter()
            .map(|pressed| egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            })
            .collect(),
    )
}

fn review_command_tabs(
    stream: &mut TcpStream,
    directory: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    submit_command(stream, "echo recorde-native-tab-one")?;
    expect_label(stream, "recorde-native-tab-one", true)?;
    expect_label(stream, "Stop", false)?;
    click_label(stream, "New tab")?;
    expect_label(stream, "recorde-native-tab-one", false)?;
    submit_command(stream, "echo recorde-native-tab-two")?;
    expect_label(stream, "recorde-native-tab-two", true)?;
    expect_label(stream, "Stop", false)?;
    click_label(stream, "Command 1")?;
    expect_label(stream, "recorde-native-tab-one", true)?;
    expect_label(stream, "recorde-native-tab-two", false)?;
    click_label(stream, "Hide")?;
    expect_label(stream, "Terminal commands", false)?;
    click_label(stream, "Terminal")?;
    expect_label(stream, "recorde-native-tab-one", true)?;

    click_label(stream, "New tab")?;
    let waiting = if cfg!(windows) {
        "ping -n 30 127.0.0.1 > nul"
    } else {
        "sleep 30"
    };
    submit_command(stream, waiting)?;
    expect_label(stream, "Stop", true)?;
    click_label(stream, "File")?;
    click_label(stream, "Quit")?;
    expect_label(stream, "Waiting to quit", true)?;
    capture(stream, directory, "quit-with-running-command")?;
    click_label(stream, "Keep working")?;
    expect_label(stream, "Waiting to quit", false)?;
    expect_label(stream, "Stop", true)?;
    click_label(stream, "Close tab")?;
    expect_label(stream, "Stop and close tab", true)?;
    click_label(stream, "Keep running")?;
    expect_label(stream, "Stop and close tab", false)?;
    expect_label(stream, "Stop", true)?;
    click_label(stream, "Stop")?;
    expect_label(stream, "Stopped", true)?;
    expect_label(stream, "Stop", false)?;
    capture(stream, directory, "terminal-stop")?;
    // Selecting the tab restores input focus before starting the second command.
    click_label(stream, "Command 3")?;
    submit_command(stream, waiting)?;
    expect_label(stream, "Stop", true)?;
    click_label(stream, "Close tab")?;
    click_label(stream, "Stop and close tab")?;
    expect_label(stream, "Command 3", false)?;
    expect_label(stream, "recorde-native-tab-two", true)?;
    capture(stream, directory, "terminal-tab-isolation")?;
    click_label(stream, "Close tab")?;
    expect_label(stream, "recorde-native-tab-one", true)?;
    click_label(stream, "Close tab")?;
    expect_label(stream, "Terminal commands", false)?;
    click_label(stream, "Terminal")?;
    expect_label(stream, "Command 4", true)?;
    expect_label(stream, "recorde-native-tab-one", false)?;
    Ok(())
}

fn capture(
    stream: &mut TcpStream,
    directory: &std::path::Path,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Event acknowledgement precedes the end of egui's opening animations.
    // Let those repaint frames settle, then request a fresh tree before the
    // screenshot so popup backgrounds are captured at their final opacity.
    std::thread::sleep(Duration::from_millis(300));
    let tree = request(stream, Request::GetTree)?;
    let Response::Tree {
        accesskit: Some(accesskit),
        ..
    } = &tree
    else {
        return Err("Cannot capture a view without its accessibility tree".into());
    };
    if !accesskit
        .nodes
        .iter()
        .any(|(id, node)| visible_bounds(accesskit, *id, node, false).is_some())
    {
        return Err("Cannot capture a view with no visible accessible bounds".into());
    }
    std::fs::write(
        directory.join(format!("{name}.json")),
        serde_json::to_vec_pretty(&tree)?,
    )?;
    match request(
        stream,
        Request::GetScreenshot {
            pixels_per_point: None,
        },
    )? {
        Response::Screenshot(image) => {
            std::fs::write(directory.join(format!("{name}.png")), image.bytes)?
        }
        response => return Err(format!("Screenshot failed: {response:?}").into()),
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let address: SocketAddr = args.next().ok_or("inspection address required")?.parse()?;
    if !address.ip().is_loopback() {
        return Err("Use a loopback inspection address".into());
    }
    let directory = PathBuf::from(args.next().ok_or("artifact directory required")?);
    let (expect_graph, review, clean_tracked) = parse_flags(args)?;
    std::fs::create_dir_all(&directory)?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5))?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    if egui_inspection::protocol::read_handshake(&mut stream)? != egui_inspection::PROTOCOL_VERSION
    {
        return Err("Incompatible inspection protocol".into());
    }
    // Design review: RECORDE_UI_TOUR=1 captures screens instead of acceptance checks.
    if std::env::var_os("RECORDE_UI_TOUR").is_some() {
        return tour::run(&mut stream, &directory);
    }
    // Marketing captures of a generated busy repository: RECORDE_UI_SHOWCASE=1.
    if std::env::var_os("RECORDE_UI_SHOWCASE").is_some() {
        return showcase::run(&mut stream, &directory);
    }
    let mut focused = HashSet::new();
    for step in 0..7 {
        let response = request(&mut stream, Request::GetTree)?;
        std::fs::write(
            directory.join(format!("accesskit-{step}.json")),
            serde_json::to_vec_pretty(&response)?,
        )?;
        let Response::Tree {
            accesskit: Some(tree),
            pixels_per_point,
            ..
        } = response
        else {
            return Err("Native app did not produce an AccessKit tree".into());
        };
        if tree.nodes.len() < 30 {
            return Err("AccessKit tree is unexpectedly empty".into());
        }
        if !tree
            .nodes
            .iter()
            .any(|(_, node)| node.role() == egui::accesskit::Role::Button)
        {
            return Err("Native controls are missing from the AccessKit tree".into());
        }
        if step > 0 {
            if !tree.nodes.iter().any(|(id, _)| *id == tree.focus) {
                return Err("Keyboard focus does not identify an accessible node".into());
            }
            focused.insert(tree.focus);
        }
        if step == 0 {
            if expect_graph {
                let labels: Vec<_> = tree
                    .nodes
                    .iter()
                    .filter_map(|(_, node)| node.label())
                    .collect();
                if !labels
                    .iter()
                    .any(|label| label.starts_with("Commit ") && label.contains(" by "))
                    || !labels.iter().any(|label| {
                        matches!(
                            *label,
                            "Local branch main" | "Local branch main, current branch"
                        )
                    })
                    || !labels.iter().any(|label| label.contains("Fetch"))
                {
                    return Err("Commit author/text, branch names or action labels are missing from native accessibility".into());
                }
            }
            std::fs::write(
                directory.join("display.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"pixels_per_point":pixels_per_point,"accessible_nodes":tree.nodes.len()}),
                )?,
            )?;
            match request(
                &mut stream,
                Request::GetScreenshot {
                    pixels_per_point: None,
                },
            )? {
                Response::Screenshot(image) => {
                    std::fs::write(directory.join("native-frame.png"), image.bytes)?
                }
                response => return Err(format!("Screenshot failed: {response:?}").into()),
            }
        }
        let events = [true, false]
            .into_iter()
            .map(|pressed| egui::Event::Key {
                key: egui::Key::Tab,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            })
            .collect();
        apply_events(&mut stream, events)?;
    }
    if focused.len() < 3 {
        return Err("Tab did not move accessible focus among controls".into());
    }
    if expect_graph {
        let Response::Tree {
            accesskit: Some(tree),
            ..
        } = request(&mut stream, Request::GetTree)?
        else {
            return Err("Missing commit tree".into());
        };
        let (first_id, first) = tree
            .nodes
            .iter()
            .filter(|(_, node)| {
                node.role() == egui::accesskit::Role::ListBoxOption
                    && node
                        .label()
                        .is_some_and(|label| label.starts_with("Commit ") && label.contains(" by "))
            })
            .min_by(|(_, a), (_, b)| {
                a.bounds()
                    .map(|bounds| bounds.y0)
                    .unwrap_or(f64::INFINITY)
                    .total_cmp(&b.bounds().map(|bounds| bounds.y0).unwrap_or(f64::INFINITY))
            })
            .ok_or("No accessible commit row")?;
        let bounds = first
            .bounds()
            .ok_or("Commit row has no accessible bounds")?;
        let pos = egui::pos2(
            ((bounds.x0 + bounds.x1) / 2.0) as f32,
            ((bounds.y0 + bounds.y1) / 2.0) as f32,
        );
        for pressed in [true, false] {
            apply_events(
                &mut stream,
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
        let Response::Tree {
            accesskit: Some(selected),
            ..
        } = request(&mut stream, Request::GetTree)?
        else {
            return Err("Missing selected commit tree".into());
        };
        if !selected
            .nodes
            .iter()
            .any(|(id, node)| id == first_id && node.is_selected() == Some(true))
        {
            return Err("Commit selection is not exposed through native accessibility".into());
        }
        apply_events(
            &mut stream,
            [true, false]
                .into_iter()
                .map(|pressed| egui::Event::Key {
                    key: egui::Key::ArrowDown,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                })
                .collect(),
        )?;
        let response = request(&mut stream, Request::GetTree)?;
        std::fs::write(
            directory.join("accesskit-graph-navigation.json"),
            serde_json::to_vec_pretty(&response)?,
        )?;
        let Response::Tree {
            accesskit: Some(navigated),
            ..
        } = response
        else {
            return Err("Missing navigated commit tree".into());
        };
        if !navigated.nodes.iter().any(|(id, node)| {
            id != first_id
                && *id == navigated.focus
                && node.is_selected() == Some(true)
                && node
                    .label()
                    .is_some_and(|label| label.starts_with("Commit "))
        }) {
            return Err("Arrow key did not move both commit selection and accessible focus".into());
        }
    }
    if review {
        click_label(&mut stream, "Help")?;
        click_label(&mut stream, "User Guide")?;
        capture(&mut stream, &directory, "help-guide")?;
        expect_label(&mut stream, "Your first commit", true)?;
        click_label(&mut stream, "Themes and accessibility")?;
        capture(&mut stream, &directory, "help-themes")?;
        expect_label(&mut stream, "Choose a colour theme", true)?;
        click_label(&mut stream, "Close")?;
        expect_label(&mut stream, "Recorde Help", false)?;
        click_label(&mut stream, "Terminal")?;
        capture(&mut stream, &directory, "terminal-panel")?;
        expect_label(&mut stream, "Terminal commands", true)?;
        expect_label(&mut stream, "Columns", true)?;
        review_command_tabs(&mut stream, &directory)?;
        click_label(&mut stream, "Terminal")?;
        expect_label(&mut stream, "Terminal commands", false)?;
        graph_actions::review(&mut stream, &directory, clean_tracked)?;
        file_editor::review(&mut stream, &directory, clean_tracked)?;
        if !clean_tracked {
            let Response::Tree {
                accesskit: Some(tree),
                ..
            } = request(&mut stream, Request::GetTree)?
            else {
                return Err("Missing file actions tree".into());
            };
            let action = ["Actions for hello.txt", "Actions for workspace.rs"]
                .into_iter()
                .find(|label| has_visible_text(&tree, label))
                .ok_or("Missing fixture file actions")?;
            let file = action.strip_prefix("Actions for ").unwrap();
            let bounds = file_editor::changes_file_bounds(&tree, file)?
                .ok_or("Missing Changes selection row")?;
            file_editor::click_bounds(&mut stream, bounds, egui::PointerButton::Primary)?;
            click_label(&mut stream, action)?;
            expect_label(&mut stream, "Stage", true)?;
            expect_label(&mut stream, "Discard", true)?;
            capture(&mut stream, &directory, "file-actions-menu")?;
            apply_events(
                &mut stream,
                [true, false]
                    .into_iter()
                    .map(|pressed| egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    })
                    .collect(),
            )?;
            expect_label(&mut stream, "Stage", false)?;
            capture(&mut stream, &directory, "file-list")?;
        }
        if clean_tracked {
            recovery::review(&mut stream, &directory)?;
            gitflow::review(&mut stream, &directory)?;
        }
        click_label(&mut stream, "Columns")?;
        expect_label(
            &mut stream,
            "Visible columns (Commit is always shown)",
            true,
        )?;
        capture(&mut stream, &directory, "graph-columns")?;
        apply_events(
            &mut stream,
            vec![
                egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        )?;
        expect_label(
            &mut stream,
            "Visible columns (Commit is always shown)",
            false,
        )?;
        click_label(&mut stream, "File")?;
        click_label(&mut stream, "Clone Repository…")?;
        click_label(&mut stream, "Browse connected accounts…")?;
        expect_label(&mut stream, "Browse provider repositories", true)?;
        capture(&mut stream, &directory, "repository-browser")?;
        click_label(&mut stream, "Close window")?;
        expect_label(&mut stream, "Browse provider repositories", false)?;
    }
    println!("Native AccessKit labels, selection and keyboard focus passed ({} distinct tab-focused controls)", focused.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::accesskit::{Node, NodeId, Rect, Role, TreeUpdate};

    #[test]
    fn review_flag_is_independent_of_graph_order_and_unknown_flags_fail() {
        let flags = |values: &[&str]| parse_flags(values.iter().map(|value| (*value).to_string()));
        assert_eq!(flags(&["--review"]), Ok((false, true, false)));
        assert_eq!(flags(&["--review", "--graph"]), Ok((true, true, false)));
        assert_eq!(flags(&["--graph", "--review"]), Ok((true, true, false)));
        assert_eq!(
            flags(&["--review", "--clean-tracked"]),
            Ok((false, true, true))
        );
        assert!(flags(&["--clean-tracked"]).is_err());
        assert!(flags(&["--typo"]).is_err());
    }

    #[test]
    fn label_selector_rejects_duplicates_and_noninteractive_hidden_or_clipped_nodes() {
        let mut button = Node::new(Role::Button);
        button.set_label("Open");
        button.set_bounds(Rect::new(0.0, 0.0, 20.0, 20.0));
        let mut text = Node::new(Role::Label);
        text.set_label("Open");
        text.set_bounds(Rect::new(0.0, 0.0, 20.0, 20.0));
        let mut tree = TreeUpdate {
            tree_id: egui::accesskit::TreeId::ROOT,
            nodes: vec![(NodeId(1), button.clone()), (NodeId(2), text)],
            tree: None,
            focus: NodeId(1),
        };
        assert!(clickable_bounds(&tree, "Open").unwrap().is_some());
        tree.nodes.push((NodeId(3), button.clone()));
        assert!(clickable_bounds(&tree, "Open").is_err());
        tree.nodes[2].1.set_hidden();
        assert!(clickable_bounds(&tree, "Open").unwrap().is_some());
        tree.nodes[0].1.set_disabled();
        assert!(clickable_bounds(&tree, "Open").unwrap().is_none());
        tree.nodes[0].1.clear_disabled();
        let mut parent = Node::new(Role::GenericContainer);
        parent.set_children(vec![NodeId(1)]);
        parent.set_clips_children();
        parent.set_bounds(Rect::new(30.0, 30.0, 50.0, 50.0));
        tree.nodes.push((NodeId(4), parent));
        assert!(clickable_bounds(&tree, "Open").unwrap().is_none());
    }

    #[test]
    fn text_expectations_accept_values_and_disabled_controls_but_reject_clipping() {
        let mut heading = Node::new(Role::Label);
        heading.set_value("Heading");
        heading.set_bounds(Rect::new(0.0, 0.0, 20.0, 20.0));
        let mut button = Node::new(Role::Button);
        button.set_label("Run");
        button.set_disabled();
        button.set_bounds(Rect::new(0.0, 20.0, 20.0, 40.0));
        let mut tree = TreeUpdate {
            tree_id: egui::accesskit::TreeId::ROOT,
            nodes: vec![(NodeId(1), heading), (NodeId(2), button)],
            tree: None,
            focus: NodeId(1),
        };
        assert!(has_visible_text(&tree, "Heading"));
        assert!(has_visible_text(&tree, "Run"));
        assert!(clickable_bounds(&tree, "Run").unwrap().is_none());
        let mut parent = Node::new(Role::GenericContainer);
        parent.set_children(vec![NodeId(1)]);
        parent.set_clips_children();
        parent.set_bounds(Rect::new(30.0, 30.0, 50.0, 50.0));
        tree.nodes.push((NodeId(3), parent));
        assert!(!has_visible_text(&tree, "Heading"));
        tree.nodes[1].1.set_hidden();
        assert!(!has_visible_text(&tree, "Run"));
    }
}
