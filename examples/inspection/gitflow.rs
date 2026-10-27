//! Native Git Flow acceptance for the disposable Windows CleanTracked fixture.
use super::*;
use std::{error::Error, path::Path};

const FEATURE: &str = "native-ui-feature";

fn tree(stream: &mut TcpStream) -> Result<egui::accesskit::TreeUpdate, Box<dyn Error>> {
    match request(stream, Request::GetTree)? {
        Response::Tree {
            accesskit: Some(tree),
            ..
        } => Ok(tree),
        _ => Err("Git Flow acceptance needs a native accessibility tree".into()),
    }
}

fn current_branch(tree: &egui::accesskit::TreeUpdate, name: &str) -> bool {
    has_visible_text(tree, &format!("Local branch {name}, current branch"))
}

fn local_branch_exists(tree: &egui::accesskit::TreeUpdate, name: &str) -> bool {
    let plain = format!("Local branch {name}");
    let current = format!("{plain}, current branch");
    tree.nodes.iter().any(|(_, node)| {
        node.label()
            .is_some_and(|label| label == plain || label == current)
    })
}

fn branch_points_to(tree: &egui::accesskit::TreeUpdate, branch: &str, oid: &str) -> bool {
    let suffix = format!(", {oid}. Drag onto HEAD to cherry-pick.");
    let row = tree.nodes.iter().find_map(|(id, node)| {
        (node.role() == egui::accesskit::Role::ListBoxOption
            && node.label().is_some_and(|label| label.ends_with(&suffix)))
        .then(|| visible_bounds(tree, *id, node, false))
        .flatten()
    });
    let Some(row) = row else { return false };
    let reference = format!("Reference refs/heads/{branch}. Click to select, double-click to check out a branch, right-click for actions.");
    tree.nodes.iter().any(|(id, node)| {
        node.label() == Some(reference.as_str())
            && visible_bounds(tree, *id, node, false).is_some_and(|bounds| {
                let y = (bounds.y0 + bounds.y1) / 2.0;
                y >= row.y0 && y <= row.y1
            })
    })
}

fn open(stream: &mut TcpStream) -> Result<(), Box<dyn Error>> {
    click_label(stream, "Branch")?;
    click_label(stream, "Git Flow…")?;
    Ok(())
}

fn wait_for_branches(
    stream: &mut TcpStream,
    current: &str,
    feature_present: bool,
) -> Result<(), Box<dyn Error>> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let tree = tree(stream)?;
        if current_branch(&tree, current)
            && local_branch_exists(&tree, "main")
            && local_branch_exists(&tree, "develop")
            && local_branch_exists(&tree, &format!("feature/{FEATURE}")) == feature_present
            && !has_visible_text(&tree, "Initialize Git Flow")
            && !has_visible_text(&tree, "Start")
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Git Flow did not settle on {current:?} with feature presence {feature_present}"
            )
            .into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn enter_feature_name(stream: &mut TcpStream) -> Result<(), Box<dyn Error>> {
    let tree = tree(stream)?;
    let inputs: Vec<_> = tree
        .nodes
        .iter()
        .filter_map(|(id, node)| {
            (node.role() == egui::accesskit::Role::TextInput && node.placeholder() == Some("name"))
                .then(|| visible_bounds(&tree, *id, node, true).map(|bounds| (*id, node, bounds)))
                .flatten()
        })
        .collect();
    let [(id, node, bounds)] = inputs.as_slice() else {
        return Err("Git Flow requires one visible Feature name input".into());
    };
    if node.value().is_some_and(|value| !value.is_empty()) {
        return Err("Git Flow acceptance refuses to replace an existing feature draft".into());
    }
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
    let focused = match request(stream, Request::GetTree)? {
        Response::Tree {
            accesskit: Some(tree),
            ..
        } => tree.focus == *id,
        _ => false,
    };
    if !focused {
        return Err("Git Flow feature name did not receive keyboard focus".into());
    }
    apply_events(stream, vec![egui::Event::Text(FEATURE.into())])?;
    expect_label(stream, FEATURE, true)?;
    Ok(())
}

fn fixture_tip(tree: &egui::accesskit::TreeUpdate) -> Result<String, Box<dyn Error>> {
    let commits: Vec<_> = tree
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
        return Err("Git Flow acceptance requires exactly two fixture commit rows".into());
    }
    let parse = |summary: &str| -> Option<String> {
        let prefix = format!("Commit {summary} by Recorde Smoke, ");
        let matches: Vec<_> = commits
            .iter()
            .filter_map(|label| {
                let oid = label
                    .strip_prefix(&prefix)?
                    .strip_suffix(". Drag onto HEAD to cherry-pick.")?;
                (oid.len() == 40 && oid.bytes().all(|byte| byte.is_ascii_hexdigit()))
                    .then(|| oid.to_owned())
            })
            .collect();
        (matches.len() == 1).then(|| matches[0].clone())
    };
    parse("fixture").ok_or("Git Flow root fixture commit is missing")?;
    parse("second fixture").ok_or_else(|| "Git Flow tip fixture commit is missing".into())
}

/// Run after recovery acceptance restores main. Uses an empty feature branch,
/// leaves develop checked out at the original tip, keeps Git Flow initialized,
/// and deletes the feature branch through Finish. No commits or network actions.
pub(super) fn review(stream: &mut TcpStream, directory: &Path) -> Result<(), Box<dyn Error>> {
    let initial = tree(stream)?;
    let original_tip = fixture_tip(&initial)?;
    if !current_branch(&initial, "main")
        || !branch_points_to(&initial, "main", &original_tip)
        || local_branch_exists(&initial, "develop")
    {
        return Err(
            "Git Flow acceptance requires current main and no pre-existing develop branch".into(),
        );
    }
    if local_branch_exists(&initial, &format!("feature/{FEATURE}")) {
        return Err("Git Flow acceptance feature branch already exists".into());
    }
    capture(stream, directory, "gitflow-before")?;
    open(stream)?;
    expect_label(stream, "Initialize Git Flow", true)?;
    let initialization = tree(stream)?;
    for value in ["main", "develop", "feature/", "release/", "hotfix/"] {
        if !initialization.nodes.iter().any(|(id, node)| {
            node.role() == egui::accesskit::Role::TextInput
                && node.value() == Some(value)
                && visible_bounds(&initialization, *id, node, true).is_some()
        }) {
            return Err(format!(
                "Git Flow initialization is not using expected fixture default {value:?}"
            )
            .into());
        }
    }
    capture(stream, directory, "gitflow-initialize")?;
    click_label(stream, "Initialize Git Flow")?;
    wait_for_branches(stream, "main", false)?;
    capture(stream, directory, "gitflow-initialized")?;
    open(stream)?;
    expect_label(stream, "Production: main   Development: develop", true)?;
    expect_label(stream, "Starts from develop.", true)?;
    expect_label(stream, "No feature, release or hotfix branches.", true)?;
    // The fresh dialog defaults to Feature. Its prefix and name placeholder
    // distinguish it from Release/Hotfix without relying on screen coordinates.
    expect_label(stream, "feature/", true)?;
    enter_feature_name(stream)?;
    capture(stream, directory, "gitflow-start-review")?;
    click_label(stream, "Start")?;
    wait_for_branches(stream, &format!("feature/{FEATURE}"), true)?;
    capture(stream, directory, "gitflow-feature-started")?;
    open(stream)?;
    expect_label(stream, &format!("Feature {FEATURE}"), true)?;
    expect_label(stream, "Production: main   Development: develop", true)?;
    capture(stream, directory, "gitflow-finish-review")?;
    click_label(stream, "Finish")?;
    expect_label(stream, "Confirm Git Flow Finish", true)?;
    capture(stream, directory, "gitflow-finish-confirmation")?;
    click_label(stream, "Cancel")?;
    expect_label(stream, "Confirm Finish", false)?;
    expect_label(stream, &format!("Feature {FEATURE}"), true)?;
    click_label(stream, "Finish")?;
    click_label(stream, "Confirm Finish")?;
    wait_for_branches(stream, "develop", false)?;
    let finished = tree(stream)?;
    if fixture_tip(&finished)? != original_tip
        || !branch_points_to(&finished, "main", &original_tip)
        || !branch_points_to(&finished, "develop", &original_tip)
    {
        return Err("Finishing an empty feature changed the fixture commit history".into());
    }
    capture(stream, directory, "gitflow-feature-finished")?;
    open(stream)?;
    expect_label(stream, "No feature, release or hotfix branches.", true)?;
    expect_label(stream, "Initialize Git Flow", false)?;
    capture(stream, directory, "gitflow-finished-dialog")?;
    click_label(stream, "Close")?;
    expect_label(stream, "No feature, release or hotfix branches.", false)?;
    std::fs::write(
        directory.join("gitflow-result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "fixture": "Windows CleanTracked; isolated two-commit repository",
            "original_tip": original_tip, "current_branch": "develop",
            "production_branch": "main", "feature": format!("feature/{FEATURE}"),
            "feature_deleted": true, "git_flow_initialized": true,
            "feature_had_commits": false
        }))?,
    )?;
    println!("Native Git Flow initialize / start / finish passed; empty feature removed and develop checked out");
    Ok(())
}
