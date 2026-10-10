//! Development-only AccessKit and keyboard smoke client for a native Recorde window.
//! The app must be built with `ui-inspection` and bind its inspection port to loopback.
use egui_inspection::{read_message, write_message, Request, Response};
use std::{
    collections::HashSet,
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    time::Duration,
};

fn request(
    stream: &mut TcpStream,
    request: Request,
) -> Result<Response, Box<dyn std::error::Error>> {
    write_message(&mut *stream, &request)?;
    Ok(read_message(stream)?)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let address: SocketAddr = args.next().ok_or("inspection address required")?.parse()?;
    if !address.ip().is_loopback() {
        return Err("Use a loopback inspection address".into());
    }
    let directory = PathBuf::from(args.next().ok_or("artifact directory required")?);
    let expect_graph = args.next().as_deref() == Some("--graph");
    std::fs::create_dir_all(&directory)?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5))?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    if egui_inspection::protocol::read_handshake(&mut stream)? != egui_inspection::PROTOCOL_VERSION
    {
        return Err("Incompatible inspection protocol".into());
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
                    || !labels
                        .iter()
                        .any(|label| label.starts_with("Branch ") && label.contains("main"))
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
        if !matches!(
            request(&mut stream, Request::ApplyEvents { events })?,
            Response::Done
        ) {
            return Err("Keyboard input was not acknowledged".into());
        }
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
            request(
                &mut stream,
                Request::ApplyEvents {
                    events: vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                },
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
        request(
            &mut stream,
            Request::ApplyEvents {
                events: [true, false]
                    .into_iter()
                    .map(|pressed| egui::Event::Key {
                        key: egui::Key::ArrowDown,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    })
                    .collect(),
            },
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
    println!("Native AccessKit labels, selection and keyboard focus passed ({} distinct tab-focused controls)", focused.len());
    Ok(())
}
