//! Native recovery acceptance for the isolated two-commit `--clean-tracked` fixture.
//! All mutations go through the same visible controls and confirmations as a user.
use super::*;
use std::{error::Error, path::Path};

const MAIN_REF: &str = "Reference refs/heads/main. Click to select, double-click to check out a branch, right-click for actions.";

fn tree(stream: &mut TcpStream) -> Result<egui::accesskit::TreeUpdate, Box<dyn Error>> {
    match request(stream, Request::GetTree)? {
        Response::Tree {
            accesskit: Some(tree),
            ..
        } => Ok(tree),
        _ => Err("Recovery acceptance needs a native accessibility tree".into()),
    }
}

fn commit_id(label: &str, summary: &str) -> Option<String> {
    let prefix = format!("Commit {summary} by Recorde Smoke, ");
    let oid = label
        .strip_prefix(&prefix)?
        .strip_suffix(". Drag onto HEAD to cherry-pick.")?;
    (oid.len() == 40 && oid.bytes().all(|byte| byte.is_ascii_hexdigit())).then(|| oid.to_owned())
}

fn row<'a>(
    tree: &'a egui::accesskit::TreeUpdate,
    oid: &str,
) -> Option<(egui::accesskit::NodeId, &'a egui::accesskit::Node)> {
    let suffix = format!(", {oid}. Drag onto HEAD to cherry-pick.");
    tree.nodes.iter().find_map(|(id, node)| {
        (node.role() == egui::accesskit::Role::ListBoxOption
            && node
                .label()
                .is_some_and(|label| label.starts_with("Commit ") && label.ends_with(&suffix)))
        .then_some((*id, node))
    })
}

fn main_points_to(tree: &egui::accesskit::TreeUpdate, oid: &str) -> bool {
    let Some((id, node)) = row(tree, oid) else {
        return false;
    };
    let Some(row_bounds) = visible_bounds(tree, id, node, false) else {
        return false;
    };
    let current_main = tree.nodes.iter().any(|(id, node)| {
        node.label() == Some("Local branch main, current branch")
            && visible_bounds(tree, *id, node, false).is_some()
    });
    current_main
        && tree.nodes.iter().any(|(id, node)| {
            node.label() == Some(MAIN_REF)
                && visible_bounds(tree, *id, node, false).is_some_and(|bounds| {
                    let center_y = (bounds.y0 + bounds.y1) / 2.0;
                    center_y >= row_bounds.y0 && center_y <= row_bounds.y1
                })
        })
}

fn wait_for_head(
    stream: &mut TcpStream,
    oid: &str,
    next_control: &str,
) -> Result<(), Box<dyn Error>> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let tree = tree(stream)?;
        // Both graph refresh and the next enabled recovery action must settle.
        // A merely closed confirmation does not establish operation completion.
        if main_points_to(&tree, oid) && clickable_bounds(&tree, next_control)?.is_some() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Recovery did not show main at {oid} with enabled {next_control:?}"
            )
            .into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn open_reset_menu(stream: &mut TcpStream, oid: &str) -> Result<(), Box<dyn Error>> {
    let tree = tree(stream)?;
    let (id, node) = row(&tree, oid).ok_or("Fixture root commit row is missing")?;
    let bounds =
        visible_bounds(&tree, id, node, true).ok_or("Fixture root commit row is not visible")?;
    // The row centre is in the commit text area, away from reference chips.
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
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        )?;
    }
    click_label(stream, "Reset current branch to here…")?;
    expect_label(stream, &format!("Reset main to {}", &oid[..7]), true)?;
    // Mixed is the app's default; fail if the actual confirmation is different.
    expect_label(stream, "Mixed Reset", true)?;
    Ok(())
}

fn restore(
    stream: &mut TcpStream,
    directory: &Path,
    redo: bool,
    oid: &str,
    name: &str,
) -> Result<(), Box<dyn Error>> {
    let (toolbar, confirmation, next) = if redo {
        (
            "Redo Mixed reset  (Ctrl+Shift+Z)",
            "Redo Mixed reset",
            "Undo Mixed reset  (Ctrl+Z)",
        )
    } else {
        (
            "Undo Mixed reset  (Ctrl+Z)",
            "Undo Mixed reset",
            "Redo Mixed reset  (Ctrl+Shift+Z)",
        )
    };
    click_label(stream, toolbar)?;
    expect_label(stream, &format!("{confirmation}?"), true)?;
    capture(stream, directory, &format!("{name}-confirmation"))?;
    click_label(stream, confirmation)?;
    wait_for_head(stream, oid, next)?;
    capture(stream, directory, name)?;
    Ok(())
}

/// Call only for the disposable Windows CleanTracked fixture. Leaves main at
/// its original tip after Reset -> Undo -> Redo -> Undo; the redo entry remains.
pub(super) fn review(stream: &mut TcpStream, directory: &Path) -> Result<(), Box<dyn Error>> {
    let initial = tree(stream)?;
    let commits: Vec<_> = initial
        .nodes
        .iter()
        .filter_map(|(_, node)| {
            (node.role() == egui::accesskit::Role::ListBoxOption)
                .then(|| node.label())
                .flatten()
                .filter(|label| label.starts_with("Commit "))
        })
        .collect();
    if commits.len() != 2 {
        return Err(format!(
            "Recovery acceptance requires exactly two fixture commit rows; found {}",
            commits.len()
        )
        .into());
    }
    let roots: Vec<_> = commits
        .iter()
        .filter_map(|label| commit_id(label, "fixture"))
        .collect();
    let tips: Vec<_> = commits
        .iter()
        .filter_map(|label| commit_id(label, "second fixture"))
        .collect();
    let ([base], [original]) = (roots.as_slice(), tips.as_slice()) else {
        return Err(
            "Recovery acceptance requires the isolated Recorde Smoke fixture commits".into(),
        );
    };
    if !main_points_to(&initial, original) {
        return Err("Recovery acceptance requires current branch main at the original second fixture commit".into());
    }
    std::fs::write(
        directory.join("recovery-targets.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "branch": "main", "original_head": original, "reset_target": base,
            "sequence": ["Mixed reset", "Undo", "Redo", "Undo"],
            "fixture": "Windows CleanTracked; isolated two-commit repository"
        }))?,
    )?;
    capture(stream, directory, "recovery-before")?;
    open_reset_menu(stream, base)?;
    capture(stream, directory, "recovery-reset-confirmation")?;
    click_label(stream, "Mixed Reset")?;
    wait_for_head(stream, base, "Undo Mixed reset  (Ctrl+Z)")?;
    capture(stream, directory, "recovery-reset")?;
    restore(stream, directory, false, original, "recovery-undo")?;
    restore(stream, directory, true, base, "recovery-redo")?;
    restore(stream, directory, false, original, "recovery-restored")?;
    println!("Native Mixed Reset / Undo / Redo / Undo passed; main restored to {original}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_commit_labels_require_exact_author_summary_and_full_sha() {
        let oid = "0123456789abcdef0123456789abcdef01234567";
        let label =
            format!("Commit fixture by Recorde Smoke, {oid}. Drag onto HEAD to cherry-pick.");
        assert_eq!(commit_id(&label, "fixture").as_deref(), Some(oid));
        assert!(commit_id(&label, "second fixture").is_none());
        assert!(commit_id(&label.replace("Recorde Smoke", "Someone else"), "fixture").is_none());
        assert!(commit_id(&label.replace(oid, "0123456"), "fixture").is_none());
    }

    #[test]
    fn head_assertion_requires_current_main_ref_on_the_exact_commit_row() {
        use egui::accesskit::{Node, NodeId, Rect, Role, TreeId, TreeUpdate};
        let oid = "0123456789abcdef0123456789abcdef01234567";
        let mut commit = Node::new(Role::ListBoxOption);
        commit.set_label(format!(
            "Commit fixture by Recorde Smoke, {oid}. Drag onto HEAD to cherry-pick."
        ));
        commit.set_bounds(Rect::new(200.0, 100.0, 900.0, 140.0));
        let mut reference = Node::new(Role::Button);
        reference.set_label(MAIN_REF);
        reference.set_bounds(Rect::new(205.0, 105.0, 260.0, 135.0));
        let mut branch = Node::new(Role::Button);
        branch.set_label("Local branch main, current branch");
        branch.set_bounds(Rect::new(0.0, 200.0, 180.0, 240.0));
        let mut tree = TreeUpdate {
            tree_id: TreeId::ROOT,
            nodes: vec![
                (NodeId(1), commit),
                (NodeId(2), reference),
                (NodeId(3), branch),
            ],
            tree: None,
            focus: NodeId(1),
        };
        assert!(main_points_to(&tree, oid));
        assert!(!main_points_to(
            &tree,
            "abcdef0123456789abcdef0123456789abcdef01"
        ));
        tree.nodes[1]
            .1
            .set_bounds(Rect::new(205.0, 145.0, 260.0, 175.0));
        assert!(
            !main_points_to(&tree, oid),
            "main chip on another commit must fail"
        );
        tree.nodes[1]
            .1
            .set_bounds(Rect::new(205.0, 105.0, 260.0, 135.0));
        tree.nodes[2].1.set_label("Local branch main");
        assert!(
            !main_points_to(&tree, oid),
            "detached or other current branch must fail"
        );
    }
}
