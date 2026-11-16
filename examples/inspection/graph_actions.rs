//! Native graph acceptance: selection and review only; never confirm a mutation.
use super::*;
use std::{error::Error, path::Path};

type Tree = egui::accesskit::TreeUpdate;
type Bounds = egui::accesskit::Rect;

#[derive(Clone)]
struct Row {
    label: String,
    oid: String,
    bounds: Bounds,
    selected: bool,
}

fn tree(stream: &mut TcpStream) -> Result<Tree, Box<dyn Error>> {
    match request(stream, Request::GetTree)? {
        Response::Tree {
            accesskit: Some(tree),
            ..
        } => Ok(tree),
        _ => Err("Graph acceptance requires an accessibility tree".into()),
    }
}

fn rows(tree: &Tree) -> Vec<Row> {
    let mut rows: Vec<_> = tree
        .nodes
        .iter()
        .filter_map(|(id, node)| {
            if node.role() != egui::accesskit::Role::ListBoxOption {
                return None;
            }
            let label = node.label()?;
            let commit = label.strip_prefix("Commit ")?;
            let commit = commit.strip_suffix(". Drag onto HEAD to cherry-pick.")?;
            let (_, oid) = commit.rsplit_once(", ")?;
            if oid.len() != 40 || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return None;
            }
            Some(Row {
                label: label.into(),
                oid: oid.into(),
                bounds: visible_bounds(tree, *id, node, false)?,
                selected: node.is_selected() == Some(true),
            })
        })
        .collect();
    rows.sort_by(|a, b| a.bounds.y0.total_cmp(&b.bounds.y0));
    rows
}

fn pointer(
    stream: &mut TcpStream,
    bounds: Bounds,
    button: egui::PointerButton,
    modifiers: egui::Modifiers,
) -> Result<(), Box<dyn Error>> {
    let pos = egui::pos2(
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    );
    for pressed in [true, false] {
        apply_events(
            stream,
            vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                },
            ],
        )?;
    }
    apply_events(
        stream,
        vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
    )
}

/// Polls like `expect_label`: after an action the app may still be
/// refreshing, with rows briefly disabled or a dialog not yet laid out.
fn eventually<T>(
    stream: &mut TcpStream,
    mut check: impl FnMut(&Tree) -> Result<T, Box<dyn Error>>,
) -> Result<T, Box<dyn Error>> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match check(&tree(stream)?) {
            Ok(value) => return Ok(value),
            Err(error) if Instant::now() >= deadline => return Err(error),
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

fn click_row(
    stream: &mut TcpStream,
    row: &Row,
    button: egui::PointerButton,
    modifiers: egui::Modifiers,
) -> Result<(), Box<dyn Error>> {
    let bounds = eventually(stream, |current| {
        clickable_bounds(current, &row.label)?
            .ok_or_else(|| "Selected fixture commit is no longer visible or enabled".into())
    })?;
    pointer(stream, bounds, button, modifiers)
}

fn selected(stream: &mut TcpStream, expected: &[&str]) -> Result<(), Box<dyn Error>> {
    let expected: HashSet<_> = expected.iter().copied().collect();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let selected: HashSet<String> = rows(&tree(stream)?)
            .into_iter()
            .filter(|row| row.selected)
            .map(|row| row.oid)
            .collect();
        if selected.iter().map(String::as_str).collect::<HashSet<_>>() == expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Graph selection differs: expected {expected:?}, got {selected:?}"
            )
            .into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn key(stream: &mut TcpStream, key: egui::Key) -> Result<(), Box<dyn Error>> {
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

fn context_action(stream: &mut TcpStream, row: &Row, label: &str) -> Result<(), Box<dyn Error>> {
    // The Details panel can expose the same action as the context menu. Only
    // click the newly visible menu control, never choose an ambiguous match.
    let before = tree(stream)?;
    let existing: HashSet<_> = before
        .nodes
        .iter()
        .filter(|(id, node)| {
            node.label() == Some(label) && visible_bounds(&before, *id, node, true).is_some()
        })
        .map(|(id, _)| *id)
        .collect();
    click_row(
        stream,
        row,
        egui::PointerButton::Secondary,
        egui::Modifiers::NONE,
    )?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let current = tree(stream)?;
        let candidates: Vec<_> = current
            .nodes
            .iter()
            .filter(|(id, node)| {
                !existing.contains(id)
                    && node.label() == Some(label)
                    && matches!(
                        node.role(),
                        egui::accesskit::Role::Button | egui::accesskit::Role::MenuItem
                    )
            })
            .filter_map(|(id, node)| visible_bounds(&current, *id, node, true))
            .collect();
        match candidates.as_slice() {
            [bounds] => {
                return pointer(
                    stream,
                    *bounds,
                    egui::PointerButton::Primary,
                    egui::Modifiers::NONE,
                )
            }
            [] if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                return Err(format!(
                    "Expected one new context action {label:?}, found {}",
                    candidates.len()
                )
                .into())
            }
        }
    }
}

fn unique_text(tree: &Tree, prefix: &str) -> Result<String, Box<dyn Error>> {
    let values: HashSet<_> = tree
        .nodes
        .iter()
        .filter(|(id, node)| visible_bounds(tree, *id, node, false).is_some())
        .flat_map(|(id, node)| {
            [node.label(), node.value()]
                .into_iter()
                .flatten()
                .map(move |text| (*id, text))
        })
        .filter(|(_, text)| text.starts_with(prefix))
        .filter(|(id, text)| {
            // Windows exposes multiline labels both as a complete accessible
            // value and nested text runs. Ignore a prefix run only when its
            // own visible ancestor supplies that longer text. Independent
            // labels remain ambiguous even when one is a prefix of another.
            let mut cursor = *id;
            let mut visited = HashSet::new();
            while visited.insert(cursor) {
                let Some((parent_id, parent)) = tree
                    .nodes
                    .iter()
                    .find(|(_, parent)| parent.children().contains(&cursor))
                else {
                    break;
                };
                if visible_bounds(tree, *parent_id, parent, false).is_some()
                    && [parent.label(), parent.value()]
                        .into_iter()
                        .flatten()
                        .any(|whole| whole.len() > text.len() && whole.starts_with(*text))
                {
                    return false;
                }
                cursor = *parent_id;
            }
            true
        })
        .map(|(_, text)| text)
        .collect();
    if values.len() != 1 {
        return Err(
            format!("Expected one visible text beginning {prefix:?}, found {values:?}").into(),
        );
    }
    Ok((*values.iter().next().unwrap()).into())
}

pub(super) fn review(
    stream: &mut TcpStream,
    directory: &Path,
    require_clean: bool,
) -> Result<(), Box<dyn Error>> {
    let initial = tree(stream)?;
    let initial_rows = rows(&initial);
    if initial_rows.len() < 2 {
        return Err("Graph acceptance requires at least two visible fixture commits".into());
    }
    let first = &initial_rows[0];
    let second = &initial_rows[1];
    let current_branches: Vec<_> = initial
        .nodes
        .iter()
        .filter_map(|(id, node)| {
            visible_bounds(&initial, *id, node, false)?;
            node.label()?
                .strip_prefix("Local branch ")?
                .strip_suffix(", current branch")
        })
        .collect();
    let [branch] = current_branches.as_slice() else {
        return Err("Graph acceptance requires one visible checked-out local branch".into());
    };
    let head_ref = format!("refs/heads/{branch}");

    click_row(
        stream,
        first,
        egui::PointerButton::Primary,
        egui::Modifiers::NONE,
    )?;
    selected(stream, &[&first.oid])?;
    let command = egui::Modifiers {
        command: true,
        ctrl: !cfg!(target_os = "macos"),
        mac_cmd: cfg!(target_os = "macos"),
        ..egui::Modifiers::NONE
    };
    click_row(stream, second, egui::PointerButton::Primary, command)?;
    selected(stream, &[&first.oid, &second.oid])?;
    click_row(stream, second, egui::PointerButton::Primary, command)?;
    selected(stream, &[&first.oid])?;
    // Re-establish the anchor explicitly after the toggle.
    click_row(
        stream,
        first,
        egui::PointerButton::Primary,
        egui::Modifiers::NONE,
    )?;
    click_row(
        stream,
        second,
        egui::PointerButton::Primary,
        egui::Modifiers::SHIFT,
    )?;
    selected(stream, &[&first.oid, &second.oid])?;
    capture(stream, directory, "graph-multi-selection")?;

    let mut bulk_details: Vec<String> = Vec::new();
    let dirty_refusal =
        "Commit or stash staged and tracked working-tree changes before applying multiple commits.";
    for (label, name) in [("Cherry-pick", "cherry-pick"), ("Revert", "revert")] {
        context_action(stream, second, &format!("{label} 2 commits"))?;
        let question = format!("{label} 2 commits on {branch}?");
        if require_clean {
            expect_label(stream, &question, true)?;
            expect_label(stream, "Uncommitted changes are in the way", false)?;
            let detail = eventually(stream, |current| unique_text(current, "Repository: "))?;
            let lines: Vec<_> = detail.lines().collect();
            if lines.len() != 3 || lines[0] == "Repository: " {
                return Err(
                    format!("Bulk confirmation lacks its frozen repository: {detail:?}").into(),
                );
            }
            let head_oid = lines[1]
                .strip_prefix("Destination HEAD: ")
                .ok_or("Bulk confirmation lacks destination HEAD")?;
            if head_oid.len() != 40 || !initial_rows.iter().any(|row| row.oid == head_oid) {
                return Err("Bulk confirmation HEAD is not a full fixture commit ID".into());
            }
            let order = if label == "Revert" {
                "Reverts run newest first."
            } else {
                "Cherry-picks run oldest first."
            };
            let expected = format!("Selected commits (oldest first): {}, {}. {order} A recovery snapshot is saved first. The operation stops if the destination branch or HEAD changes before it starts.", &second.oid[..7], &first.oid[..7]);
            if lines[2] != expected {
                return Err(format!(
                    "Bulk confirmation selected IDs/order differ: {:?}",
                    lines[2]
                )
                .into());
            }
            if let Some(previous) = bulk_details.first() {
                if previous.lines().take(2).collect::<Vec<_>>() != lines[..2] {
                    return Err("Bulk previews changed their repository or destination HEAD".into());
                }
            }
            key(stream, egui::Key::ArrowDown)?;
            expect_label(stream, &detail, true)?;
            expect_label(stream, &question, true)?;
            capture(stream, directory, &format!("graph-bulk-{name}-preview"))?;
            click_label(stream, "Cancel")?;
            expect_label(stream, &question, false)?;
            bulk_details.push(detail);
        } else {
            expect_label(stream, dirty_refusal, true)?;
            expect_label(stream, "Uncommitted changes are in the way", true)?;
            expect_label(stream, &question, false)?;
            capture(
                stream,
                directory,
                &format!("graph-bulk-{name}-dirty-refusal"),
            )?;
            click_label(stream, "Dismiss")?;
            expect_label(stream, dirty_refusal, false)?;
        }
        selected(stream, &[&first.oid, &second.oid])?;
    }

    // A plain click is idempotent, so retry it if the app dropped it while
    // still settling after the dismissed notice.
    for attempt in 0..3 {
        click_row(
            stream,
            second,
            egui::PointerButton::Primary,
            egui::Modifiers::NONE,
        )?;
        match selected(stream, &[&second.oid]) {
            Ok(()) => break,
            Err(error) if attempt == 2 => return Err(error),
            Err(_) => {}
        }
    }
    let mut single_details = Vec::new();
    for (action, label, name) in [
        ("Cherry-pick", "Cherry-pick", "cherry-pick"),
        ("Revert Commit", "Revert", "revert"),
    ] {
        // Opening a preview never applies it, so a dropped menu click can be
        // retried safely.
        for attempt in 0..3 {
            context_action(stream, second, action)?;
            match expect_label(stream, "Confirm graph operation", true) {
                Ok(()) => break,
                Err(error) if attempt == 2 => return Err(error),
                Err(_) => {}
            }
        }
        expect_label(stream, &format!("Confirm {label}"), true)?;
        expect_label(stream, &format!("From: {}", &second.oid[..12]), true)?;
        expect_label(stream, &format!("Destination: {head_ref}"), true)?;
        let repository = eventually(stream, |current| unique_text(current, "Repository: "))?;
        let current = tree(stream)?;
        let head = unique_text(&current, "Expected HEAD: ")?;
        if !initial_rows
            .iter()
            .any(|row| head == format!("Expected HEAD: {head_ref} ({})", &row.oid[..7]))
        {
            return Err(
                "Single confirmation HEAD does not match the fixture branch and commits".into(),
            );
        }
        if let Some(detail) = bulk_details.first() {
            let mut lines = detail.lines();
            if lines.next() != Some(repository.as_str()) {
                return Err("Single and bulk previews disagree about the repository".into());
            }
            let full_head = lines
                .next()
                .unwrap()
                .strip_prefix("Destination HEAD: ")
                .unwrap();
            if head != format!("Expected HEAD: {head_ref} ({})", &full_head[..7]) {
                return Err("Single and bulk previews disagree about destination HEAD".into());
            }
        }
        // The dialog focuses Cancel on its next frame; arrow keys must reach
        // it rather than the action choices, which they would change.
        eventually(stream, |current| {
            current
                .nodes
                .iter()
                .find(|(id, _)| *id == current.focus)
                .and_then(|(_, node)| node.label())
                .filter(|label| *label == "Cancel")
                .map(|_| ())
                .ok_or_else(|| "Cancel does not have keyboard focus".into())
        })?;
        key(stream, egui::Key::ArrowDown)?;
        for text in [&repository, &head, &format!("From: {}", &second.oid[..12])] {
            expect_label(stream, text, true)?;
        }
        capture(stream, directory, &format!("graph-single-{name}-preview"))?;
        // Exercise both dismissal paths without ever activating confirmation.
        if label == "Revert" {
            key(stream, egui::Key::Escape)?;
        } else {
            click_label(stream, "Cancel")?;
        }
        expect_label(stream, "Confirm graph operation", false)?;
        selected(stream, &[&second.oid])?;
        single_details.push((repository, head));
    }
    let final_rows = rows(&tree(stream)?);
    if initial_rows.iter().map(|row| &row.oid).collect::<Vec<_>>()
        != final_rows.iter().map(|row| &row.oid).collect::<Vec<_>>()
    {
        return Err("Visible history changed during preview-only graph acceptance".into());
    }
    std::fs::write(
        directory.join("graph-actions-result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "require_clean": require_clean,
            "selected_oids_oldest_first": [&second.oid, &first.oid],
            "selection": "plain, Ctrl/Cmd toggle, Shift range",
            "bulk": if require_clean { "frozen previews cancelled" } else { dirty_refusal },
            "bulk_previews": bulk_details,
            "single_previews": single_details,
            "history_unchanged": true,
            "mutations_confirmed": 0
        }))?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::accesskit::{Node, NodeId, Rect, Role, TreeId};

    fn label(text: &str) -> Node {
        let mut node = Node::new(Role::Label);
        node.set_label(text);
        node.set_bounds(Rect::new(0.0, 0.0, 400.0, 100.0));
        node
    }

    #[test]
    fn complete_preview_text_supersedes_its_nested_prefix_runs() {
        let whole = "Repository: /fixture\nDestination HEAD: abc123\nSelected commits: def456";
        let mut parent = label(whole);
        parent.set_children(vec![NodeId(2)]);
        let mut middle = Node::new(Role::GenericContainer);
        middle.set_children(vec![NodeId(3), NodeId(4)]);
        let mut value = label("unrelated label");
        value.set_value("Repository: /fixture");
        let tree = Tree {
            tree_id: TreeId::ROOT,
            nodes: vec![
                (NodeId(1), parent),
                (NodeId(2), middle),
                (NodeId(3), label("Repository: ")),
                (NodeId(4), value),
            ],
            tree: None,
            focus: NodeId(1),
        };
        assert_eq!(unique_text(&tree, "Repository: ").unwrap(), whole);
    }

    #[test]
    fn independent_complete_labels_remain_ambiguous_even_when_prefixes_match() {
        let mut tree = Tree {
            tree_id: TreeId::ROOT,
            nodes: vec![
                (NodeId(1), label("Repository: /fixture")),
                (NodeId(2), label("Repository: /fixture/other")),
            ],
            tree: None,
            focus: NodeId(1),
        };
        assert!(unique_text(&tree, "Repository: ").is_err());
        tree.nodes[1].1.set_label("Repository: /different");
        assert!(unique_text(&tree, "Repository: ").is_err());
        tree.nodes[1].1.set_hidden();
        assert_eq!(
            unique_text(&tree, "Repository: ").unwrap(),
            "Repository: /fixture"
        );
    }
}
