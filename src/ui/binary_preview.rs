use crate::git::binary_preview::{BinaryPreview, BinarySide};
use crate::state::DiffContext;

pub fn show(
    ui: &mut egui::Ui,
    preview: &BinaryPreview,
    textures: &mut [Option<egui::TextureHandle>; 2],
    context: DiffContext,
) {
    let labels = match context {
        DiffContext::StagedFile => ["HEAD", "Index (staged)"],
        DiffContext::UnstagedFile => ["Index", "Working file"],
        DiffContext::CommitDiff => ["Parent", "Commit"],
    };
    let id = ui.id().with("image_comparison_options");
    let (mut zoom, mut fit, mut overlay, mut opacity) = ui
        .ctx()
        .data_mut(|data| data.get_temp::<(f32, bool, bool, f32)>(id))
        .unwrap_or((1.0, true, false, 0.5));
    let images = [&preview.before, &preview.after].map(|side| match side {
        BinarySide::Content { image, .. } => image.as_ref(),
        _ => None,
    });
    for (index, image) in images.iter().enumerate() {
        if let Some(image) = image {
            textures[index].get_or_insert_with(|| {
                ui.ctx().load_texture(
                    format!("binary-preview-{index}"),
                    egui::ColorImage::from_rgba_unmultiplied(image.size, &image.rgba),
                    egui::TextureOptions::LINEAR,
                )
            });
        }
    }
    let can_overlay = images[0]
        .zip(images[1])
        .is_some_and(|(a, b)| a.original_size == b.original_size);
    if images.iter().any(Option::is_some) {
        ui.horizontal(|ui| {
            ui.checkbox(&mut fit, "Fit");
            crate::ui::kit::slider(ui, egui::Slider::new(&mut zoom, 0.25..=4.0).text("Zoom"));
            ui.add_enabled(can_overlay, egui::Checkbox::new(&mut overlay, "Overlay"))
                .on_hover_text("Overlay requires images with matching pixel dimensions.");
        });
        ui.small("Previews are limited to 1024 pixels per side; GIF displays its first frame.");
    }
    if overlay && can_overlay {
        crate::ui::kit::slider(
            ui,
            egui::Slider::new(&mut opacity, 0.0..=1.0).text(format!("{} opacity", labels[1])),
        );
        let a = textures[0].as_ref().unwrap();
        let b = textures[1].as_ref().unwrap();
        let factor = if fit {
            (ui.available_width() / a.size_vec2().x).min(1.0)
        } else {
            1.0
        } * zoom;
        egui::ScrollArea::both()
            .id_salt("image_overlay")
            .show(ui, |ui| {
                let (rect, _) =
                    ui.allocate_exact_size(a.size_vec2() * factor, egui::Sense::hover());
                let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
                ui.painter().image(a.id(), rect, uv, egui::Color32::WHITE);
                ui.painter().image(
                    b.id(),
                    rect,
                    uv,
                    egui::Color32::WHITE.gamma_multiply(opacity),
                );
            });
        ui.ctx()
            .data_mut(|data| data.insert_temp(id, (zoom, fit, overlay, opacity)));
        return;
    }
    ui.ctx()
        .data_mut(|data| data.insert_temp(id, (zoom, fit, overlay, opacity)));
    ui.columns(2, |columns| {
        for (index, side) in [&preview.before, &preview.after].into_iter().enumerate() {
            let ui = &mut columns[index];
            ui.strong(labels[index]);
            match side {
                BinarySide::Missing => {
                    ui.label("File absent");
                }
                BinarySide::Unavailable(error) => {
                    ui.label(error);
                }
                BinarySide::Content {
                    size,
                    hex,
                    image,
                    note,
                } => {
                    ui.label(format!("{size} bytes"));
                    if let Some(image) = image {
                        let texture = textures[index].as_ref().unwrap();
                        ui.label(format!(
                            "{} x {} pixels",
                            image.original_size[0], image.original_size[1]
                        ));
                        let factor = if fit {
                            (ui.available_width() / texture.size_vec2().x).min(1.0)
                        } else {
                            1.0
                        } * zoom;
                        egui::ScrollArea::both()
                            .id_salt(("image_zoom", index))
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Image::new(texture)
                                        .fit_to_exact_size(texture.size_vec2() * factor),
                                );
                            });
                    } else {
                        if let Some(note) = note {
                            ui.label(note);
                        }
                        if !hex.is_empty() {
                            egui::ScrollArea::horizontal()
                                .id_salt(("binary_hex", index))
                                .show(ui, |ui| {
                                    ui.monospace(hex);
                                });
                        }
                    }
                }
            }
        }
    });
}
