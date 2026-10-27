//! Native acceptance of the built-in editor, using only disposable GUI fixtures.
use super::*;
use std::{error::Error, path::Path};

type ReviewResult<T> = Result<T, Box<dyn Error>>;

fn tree(stream: &mut TcpStream) -> ReviewResult<egui::accesskit::TreeUpdate> {
    match request(stream, Request::GetTree)? {
        Response::Tree {
            accesskit: Some(tree),
            ..
        } => Ok(tree),
        _ => Err("Missing file editor accessibility tree".into()),
    }
}

pub(super) fn click_bounds(
    stream: &mut TcpStream,
    bounds: egui::accesskit::Rect,
    button: egui::PointerButton,
) -> ReviewResult<()> {
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
                    button,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod scroll_tests {
    use super::*;
    use egui::accesskit::{Node, NodeId, Rect, Role, TreeId, TreeUpdate};

    #[test]
    fn changes_row_behind_fixed_composer_is_rejected_until_scrolled_into_list() {
        // Native 125% capture: AccessKit reports the untracked row even though
        // the list ends above it, and no ancestor exposes clips_children.
        let mut row = Node::new(Role::Button);
        row.set_label("file.txt");
        row.set_bounds(Rect::new(872.0, 414.4, 1088.0, 446.4));
        let mut actions = Node::new(Role::Button);
        actions.set_label("Actions for file.txt");
        actions.set_bounds(Rect::new(1096.0, 414.4, 1138.0, 446.8));
        let mut composer = Node::new(Role::Label);
        composer.set_value("Create a commit");
        composer.set_bounds(Rect::new(820.8, 406.0, 929.5, 422.8));
        let mut tab = Node::new(Role::Button);
        tab.set_label("Changes");
        tab.set_bounds(Rect::new(821.0, 154.0, 901.8, 186.0));
        let mut toolbar = Node::new(Role::Button);
        toolbar.set_label("Changes");
        toolbar.set_bounds(Rect::new(587.7, 3.5, 660.0, 35.5));
        let mut tree = TreeUpdate {
            tree_id: TreeId::ROOT,
            nodes: vec![
                (NodeId(1), row),
                (NodeId(2), actions),
                (NodeId(3), composer),
                (NodeId(4), tab),
                (NodeId(5), toolbar),
            ],
            tree: None,
            focus: NodeId(1),
        };
        assert!(changes_row_bounds(&tree, "file.txt").unwrap().is_some());
        assert!(changes_file_bounds(&tree, "file.txt").unwrap().is_none());
        let interior = changes_list_viewport(&tree, "file.txt").unwrap().unwrap();
        assert!(interior.y0 > 186.0 && interior.y1 < 398.4);
        tree.nodes[0]
            .1
            .set_bounds(Rect::new(872.0, 320.0, 1088.0, 352.0));
        tree.nodes[1]
            .1
            .set_bounds(Rect::new(1096.0, 320.0, 1138.0, 352.4));
        assert!(changes_file_bounds(&tree, "file.txt").unwrap().is_some());
        tree.nodes[0]
            .1
            .set_bounds(Rect::new(872.0, 170.0, 1088.0, 202.0));
        tree.nodes[1]
            .1
            .set_bounds(Rect::new(1096.0, 170.0, 1138.0, 202.4));
        assert!(changes_file_bounds(&tree, "file.txt").unwrap().is_none());
    }
}

fn editor_input(
    tree: &egui::accesskit::TreeUpdate,
    file: &str,
) -> ReviewResult<(egui::accesskit::NodeId, egui::accesskit::Rect, String)> {
    let title = format!("Edit {file}");
    let dirty_title = format!("{title} •");
    let roots: Vec<_> = tree
        .nodes
        .iter()
        .filter(|(_, node)| {
            node.role() == egui::accesskit::Role::Window
                && matches!(node.label(), Some(label) if label == title || label == dirty_title)
        })
        .collect();
    if roots.len() != 1 {
        return Err(format!("Expected one editor window for fixture {file}").into());
    }
    let mut pending = roots[0].1.children().to_vec();
    let mut visited = HashSet::new();
    let mut inputs = Vec::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        let Some((_, node)) = tree.nodes.iter().find(|(candidate, _)| *candidate == id) else {
            continue;
        };
        pending.extend_from_slice(node.children());
        if node.role() == egui::accesskit::Role::MultilineTextInput {
            if let Some(bounds) = visible_bounds(tree, id, node, true) {
                inputs.push((id, bounds, node.value().unwrap_or_default().to_owned()));
            }
        }
    }
    if inputs.len() != 1 {
        return Err("Expected one visible multiline input inside the file editor".into());
    }
    Ok(inputs.remove(0))
}

fn expect_contents(stream: &mut TcpStream, file: &str, expected: &str) -> ReviewResult<()> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let current = tree(stream)?;
        if editor_input(&current, file)?.2 == expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(
                format!("Editor contents did not match expected fixture state for {file}").into(),
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn replace_contents(stream: &mut TcpStream, file: &str, value: &str) -> ReviewResult<()> {
    let current = tree(stream)?;
    let (id, bounds, _) = editor_input(&current, file)?;
    click_bounds(stream, bounds, egui::PointerButton::Primary)?;
    if tree(stream)?.focus != id {
        return Err("File editor text input did not receive accessible focus".into());
    }
    apply_events(
        stream,
        [true, false]
            .into_iter()
            .map(|pressed| egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..egui::Modifiers::NONE
                },
            })
            .collect(),
    )?;
    apply_events(stream, vec![egui::Event::Text(value.to_owned())])?;
    expect_contents(stream, file, value)
}

fn changes_row_bounds(
    tree: &egui::accesskit::TreeUpdate,
    file: &str,
) -> ReviewResult<Option<egui::accesskit::Rect>> {
    // Wide windows also expose the same filename in the diff navigator. Locate
    // the Changes row beside its uniquely named action menu, not that duplicate.
    let Some(actions) = clickable_bounds(tree, &format!("Actions for {file}"))? else {
        return Ok(None);
    };
    let rows: Vec<_> = tree
        .nodes
        .iter()
        .filter_map(|(id, node)| {
            if node.role() != egui::accesskit::Role::Button || node.label() != Some(file) {
                return None;
            }
            visible_bounds(tree, *id, node, true).filter(|bounds| {
                bounds.x1 <= actions.x0 && bounds.y0 < actions.y1 && bounds.y1 > actions.y0
            })
        })
        .collect();
    match rows.as_slice() {
        [] => Ok(None),
        [bounds] => Ok(Some(*bounds)),
        _ => Err(format!("Ambiguous Changes row beside Actions for {file}").into()),
    }
}

fn changes_list_viewport(
    tree: &egui::accesskit::TreeUpdate,
    file: &str,
) -> ReviewResult<Option<egui::accesskit::Rect>> {
    let Some(actions) = clickable_bounds(tree, &format!("Actions for {file}"))? else {
        return Ok(None);
    };
    let headers: Vec<_> = tree
        .nodes
        .iter()
        .filter_map(|(id, node)| {
            (node.role() == egui::accesskit::Role::Label
                && (node.value() == Some("Create a commit")
                    || node.label() == Some("Create a commit")))
            .then(|| visible_bounds(tree, *id, node, false))
            .flatten()
        })
        .collect();
    let [composer] = headers.as_slice() else {
        return Err("Expected one visible Create a commit header to bound the Changes list".into());
    };
    let tabs: Vec<_> = tree
        .nodes
        .iter()
        .filter_map(|(id, node)| {
            if node.role() != egui::accesskit::Role::Button || node.label() != Some("Changes") {
                return None;
            }
            visible_bounds(tree, *id, node, true).filter(|bounds| {
                bounds.x1 >= composer.x0 && bounds.x0 <= actions.x1 && bounds.y1 < composer.y0
            })
        })
        .collect();
    let [tab] = tabs.as_slice() else {
        return Err("Expected one Changes panel tab above the commit composer".into());
    };
    // This egui ScrollArea does not expose clips_children in AccessKit. Its
    // covered descendants still have bounds. Derive a conservative interior
    // from visible sibling headers, leaving one text-line above the composer.
    let text_height = composer.y1 - composer.y0;
    let viewport = egui::accesskit::Rect::new(
        composer.x0,
        tab.y1 + text_height / 2.0,
        actions.x1,
        composer.y0 - text_height,
    );
    if viewport.y1 <= viewport.y0 || viewport.x1 <= viewport.x0 {
        return Err("Changes file list has no observable safe scrolling interior".into());
    }
    Ok(Some(viewport))
}

pub(super) fn changes_file_bounds(
    tree: &egui::accesskit::TreeUpdate,
    file: &str,
) -> ReviewResult<Option<egui::accesskit::Rect>> {
    let Some(row) = changes_row_bounds(tree, file)? else {
        return Ok(None);
    };
    let Some(viewport) = changes_list_viewport(tree, file)? else {
        return Ok(None);
    };
    // Require the whole row to be inside the observed list; a center hidden by
    // the fixed composer must never be treated as a valid context-menu target.
    Ok((row.y0 >= viewport.y0 && row.y1 <= viewport.y1).then_some(row))
}

fn scroll_file_into_view(
    stream: &mut TcpStream,
    file: &str,
) -> ReviewResult<egui::accesskit::Rect> {
    let deadline = Instant::now() + Duration::from_secs(5);
    for _ in 0..20 {
        let current = tree(stream)?;
        if let Some(row) = changes_file_bounds(&current, file)? {
            return Ok(row);
        }
        if Instant::now() >= deadline {
            break;
        }
        let (Some(row), Some(viewport)) = (
            changes_row_bounds(&current, file)?,
            changes_list_viewport(&current, file)?,
        ) else {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        };
        let actions = clickable_bounds(&current, &format!("Actions for {file}"))?
            .ok_or("Changes action menu disappeared while locating its scroll area")?;
        let pos = egui::pos2(
            ((actions.x0 + actions.x1) / 2.0) as f32,
            ((viewport.y0 + viewport.y1) / 2.0) as f32,
        );
        let amount = ((viewport.y1 - viewport.y0) / 2.0) as f32;
        apply_events(
            stream,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(
                        0.0,
                        if row.y0 < viewport.y0 {
                            amount
                        } else {
                            -amount
                        },
                    ),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        )?;
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!("Could not scroll the Changes list to a fully visible fixture row: {file}").into())
}

fn open_file(
    stream: &mut TcpStream,
    directory: &Path,
    file: &str,
    phase: &str,
) -> ReviewResult<()> {
    let result = (|| {
        scroll_file_into_view(stream, file)?;
        // Capture after scrolling, then fetch fresh row bounds after any smooth
        // scrolling/repaint frames instead of reusing pre-capture geometry.
        capture(
            stream,
            directory,
            &format!("file-editor-{phase}-before-open"),
        )?;
        open_file_from_changes(stream, directory, file, phase)
    })();
    if let Err(error) = result {
        let diagnostic = capture(
            stream,
            directory,
            &format!("file-editor-{phase}-open-failed"),
        );
        return Err(match diagnostic {
            Ok(()) => format!("File editor {phase} open failed for {file}: {error}"),
            Err(capture_error) => format!("File editor {phase} open failed for {file}: {error}; diagnostic capture also failed: {capture_error}"),
        }.into());
    }
    Ok(())
}

fn open_file_from_changes(
    stream: &mut TcpStream,
    directory: &Path,
    file: &str,
    phase: &str,
) -> ReviewResult<()> {
    let bounds = scroll_file_into_view(stream, file)?;
    std::fs::write(
        directory.join(format!("file-editor-{phase}-right-click-target.json")),
        serde_json::to_vec_pretty(&serde_json::json!({
            "file": file,
            "bounds_in_logical_points": {
                "left": bounds.x0, "top": bounds.y0,
                "right": bounds.x1, "bottom": bounds.y1,
            },
            "selection": "Visible file Button beside the unique Actions for file control",
        }))?,
    )?;
    click_bounds(stream, bounds, egui::PointerButton::Secondary)?;
    click_label(stream, "Edit in Recorde")?;
    expect_label(stream, &format!("Edit {file}"), true)?;
    editor_input(&tree(stream)?, file)?;
    Ok(())
}

fn expect_clean(stream: &mut TcpStream, file: &str) -> ReviewResult<()> {
    expect_label(stream, &format!("Edit {file}"), true)?;
    expect_label(stream, &format!("Edit {file} •"), false)?;
    let current = tree(stream)?;
    if !current.nodes.iter().any(|(id, node)| {
        node.role() == egui::accesskit::Role::Button
            && node.label() == Some("Save")
            && node.is_disabled()
            && visible_bounds(&current, *id, node, false).is_some()
    }) {
        return Err("Saved/reloaded editor must expose a disabled Save button".into());
    }
    Ok(())
}

fn review_draft_navigation(
    stream: &mut TcpStream,
    directory: &Path,
    file: &str,
    draft: &str,
) -> ReviewResult<()> {
    #[derive(serde::Deserialize)]
    struct Fixture {
        workspace_name: String,
        primary_repository: String,
        secondary_repository: String,
    }
    let fixture: Fixture =
        serde_json::from_slice(&std::fs::read(directory.join("navigation-fixture.json"))?)?;
    let suffix = directory
        .file_name()
        .ok_or("Navigation fixture directory has no name")?
        .to_string_lossy();
    if fixture.workspace_name != format!("Recorde native workspace {suffix}")
        || Path::new(&fixture.primary_repository)
            .file_name()
            .and_then(|name| name.to_str())
            != Some("repo")
        || Path::new(&fixture.secondary_repository)
            .file_name()
            .and_then(|name| name.to_str())
            != Some("workspace-repo")
    {
        return Err(
            "Draft navigation acceptance requires the isolated CleanTracked workspace fixture"
                .into(),
        );
    }
    click_label(stream, "Close repository tab repo")?;
    expect_label(stream, "Unsaved file edits", true)?;
    expect_label(
        stream,
        &format!("{file} — {}", fixture.primary_repository),
        true,
    )?;
    capture(stream, directory, "file-editor-tab-close-guard")?;
    // A lifecycle decision must also prevent the editor's raw Save shortcut.
    apply_events(
        stream,
        [true, false]
            .into_iter()
            .map(|pressed| egui::Event::Key {
                key: egui::Key::S,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..egui::Modifiers::NONE
                },
            })
            .collect(),
    )?;
    click_label(stream, "Keep editing")?;
    expect_label(stream, "Unsaved file edits", false)?;
    expect_contents(stream, file, draft)?;
    expect_label(stream, &format!("Edit {file} •"), true)?;
    expect_label(stream, "Close repository tab repo", true)?;

    // egui exposes the workspace picker as a ComboBox value, not a button label.
    let current = tree(stream)?;
    let pickers: Vec<_> = current
        .nodes
        .iter()
        .filter_map(|(id, node)| {
            (node.role() == egui::accesskit::Role::ComboBox && node.value() == Some("Workspaces"))
                .then(|| visible_bounds(&current, *id, node, true))
                .flatten()
        })
        .collect();
    let [bounds] = pickers.as_slice() else {
        return Err("Expected one visible Workspaces picker".into());
    };
    click_bounds(stream, *bounds, egui::PointerButton::Primary)?;
    click_label(stream, &fixture.workspace_name)?;
    expect_label(stream, "Unsaved file edits", true)?;
    expect_label(
        stream,
        &format!("{file} — {}", fixture.primary_repository),
        true,
    )?;
    capture(stream, directory, "file-editor-workspace-guard")?;
    click_label(stream, "Keep editing")?;
    expect_label(stream, "Unsaved file edits", false)?;
    expect_contents(stream, file, draft)?;
    expect_label(stream, &format!("Edit {file} •"), true)?;
    expect_label(stream, "Close repository tab repo", true)?;
    // Close a workspace picker retained behind the cancelled lifecycle modal.
    // This leaves the next editor action free of an unrelated popup.
    apply_events(
        stream,
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
    capture(stream, directory, "file-editor-navigation-cancelled")?;
    println!(
        "Native dirty tab-close and workspace navigation cancellation retained the original draft"
    );
    Ok(())
}

/// Run after terminal acceptance has hidden its panel and before opening another
/// popup. All file changes go through the native editor; original fixture text
/// is restored after proving that Save persists across close/reopen.
pub(super) fn review(
    stream: &mut TcpStream,
    directory: &Path,
    clean_tracked: bool,
) -> ReviewResult<()> {
    let current = tree(stream)?;
    let working: Vec<_> = current
        .nodes
        .iter()
        .filter_map(|(_, node)| node.label())
        .filter(|label| label.starts_with("Working changes ("))
        .filter(|label| clickable_bounds(&current, label).ok().flatten().is_some())
        .collect();
    if working.len() != 1 {
        return Err("Expected one visible Working changes control".into());
    }
    click_label(stream, working[0])?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let file = loop {
        let current = tree(stream)?;
        // Restrict mutation to the explicitly provisioned smoke fixtures.
        let mut found = None;
        for candidate in ["hello.txt", "workspace.rs", "src/workspace.rs", "file.txt"] {
            if changes_row_bounds(&current, candidate)?.is_some() {
                found = Some(candidate);
                break;
            }
        }
        if let Some(file) = found {
            break file;
        }
        if Instant::now() >= deadline {
            return Err("No visible disposable editor fixture row".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    open_file(stream, directory, file, "initial")?;
    let original = editor_input(&tree(stream)?, file)?.2;
    expect_clean(stream, file)?;
    let draft = format!("{original}\nrecorde-native-unsaved-draft\n");
    replace_contents(stream, file, &draft)?;
    click_label(stream, "Close")?;
    expect_label(stream, "Discard unsaved changes?", true)?;
    capture(stream, directory, "file-editor-dirty-close")?;
    click_label(stream, "Keep editing")?;
    expect_label(stream, "Discard unsaved changes?", false)?;
    expect_contents(stream, file, &draft)?;

    if clean_tracked {
        review_draft_navigation(stream, directory, file, &draft)?;
    }

    click_label(stream, "Reload")?;
    expect_label(
        stream,
        "Discard unsaved changes and reload from disk?",
        true,
    )?;
    expect_label(stream, "Discard unsaved changes?", false)?;
    capture(stream, directory, "file-editor-reload-confirmation")?;
    click_label(stream, "Keep editing")?;
    expect_label(stream, "Discard and reload", false)?;
    expect_contents(stream, file, &draft)?;
    click_label(stream, "Reload")?;
    click_label(stream, "Discard and reload")?;
    expect_contents(stream, file, &original)?;
    expect_label(stream, "Discard and reload", false)?;
    expect_clean(stream, file)?;
    // Clean Reload reads directly without displaying a discard decision.
    click_label(stream, "Reload")?;
    expect_label(stream, "Discard and reload", false)?;
    expect_contents(stream, file, &original)?;

    let saved = format!("{original}\nrecorde-native-saved-round-trip\n");
    replace_contents(stream, file, &saved)?;
    click_label(stream, "Save")?;
    expect_clean(stream, file)?;
    click_label(stream, "Close")?;
    expect_label(stream, &format!("Edit {file}"), false)?;
    open_file(stream, directory, file, "saved-reopen")?;
    expect_contents(stream, file, &saved)?;
    expect_clean(stream, file)?;
    capture(stream, directory, "file-editor-saved-reopened")?;
    replace_contents(stream, file, &original)?;
    click_label(stream, "Save")?;
    expect_clean(stream, file)?;
    click_label(stream, "Close")?;
    open_file(stream, directory, file, "restored-reopen")?;
    expect_contents(stream, file, &original)?;
    capture(stream, directory, "file-editor-restored")?;
    click_label(stream, "Close")?;
    expect_label(stream, &format!("Edit {file}"), false)?;
    println!("Native file editor: dirty close/reload confirmations, cancellation, save/reopen and fixture restoration passed ({file})");
    Ok(())
}
