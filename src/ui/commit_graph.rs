use egui::{Color32, Painter, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use crate::state::SharedState;

const ROW_HEIGHT: f32 = 28.0;
const LANE_WIDTH: f32 = 16.0;
const DOT_RADIUS: f32 = 5.0;
const GRAPH_LEFT_PAD: f32 = 8.0;

/// Colour palette for branch lanes.
const LANE_COLORS: &[Color32] = &[
    Color32::from_rgb(99, 148, 255),
    Color32::from_rgb(255, 138, 101),
    Color32::from_rgb(102, 217, 140),
    Color32::from_rgb(255, 213, 79),
    Color32::from_rgb(206, 147, 216),
    Color32::from_rgb(77, 208, 225),
    Color32::from_rgb(255, 112, 112),
    Color32::from_rgb(165, 214, 167),
];

fn lane_color(lane: usize) -> Color32 {
    LANE_COLORS[lane % LANE_COLORS.len()]
}

pub struct CommitGraphView {
    state: SharedState,
}

impl CommitGraphView {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        let state = self.state.lock().unwrap();

        let Some(ref graph) = state.commit_graph else {
            ui.centered_and_justified(|ui| {
                ui.label("Open a repository to view commits.");
            });
            return;
        };

        let nodes = graph.nodes.clone();
        drop(state);

        let max_lane = nodes.iter().map(|n| n.lane).max().unwrap_or(0);
        let graph_col_width = GRAPH_LEFT_PAD + (max_lane + 1) as f32 * LANE_WIDTH + 8.0;
        let total_height = nodes.len() as f32 * ROW_HEIGHT;

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let (response, painter) = ui.allocate_painter(
                    Vec2::new(ui.available_width(), total_height),
                    Sense::click(),
                );

                let clip = response.rect;

                // Only render visible rows.
                let scroll_top = clip.top();
                let scroll_bot = clip.bottom();
                let first_row = ((scroll_top - clip.top()) / ROW_HEIGHT).floor() as usize;
                let last_row =
                    (((scroll_bot - clip.top()) / ROW_HEIGHT).ceil() as usize + 1).min(nodes.len());

                for i in first_row..last_row {
                    let node = &nodes[i];
                    let y = clip.top() + i as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0;
                    let color = lane_color(node.lane);
                    let dot_x = clip.left() + GRAPH_LEFT_PAD + node.lane as f32 * LANE_WIDTH;

                    // Vertical line to next commit in the same lane.
                    if i + 1 < nodes.len() {
                        let next_same = nodes[i + 1..]
                            .iter()
                            .enumerate()
                            .find(|(_, n)| n.lane == node.lane);
                        if let Some((offset, _)) = next_same {
                            let y2 = clip.top()
                                + (i + 1 + offset) as f32 * ROW_HEIGHT
                                + ROW_HEIGHT / 2.0;
                            painter.line_segment(
                                [Pos2::new(dot_x, y), Pos2::new(dot_x, y2)],
                                Stroke::new(1.5, color),
                            );
                        }
                    }

                    // Draw commit dot.
                    painter.circle_filled(Pos2::new(dot_x, y), DOT_RADIUS, color);
                    painter.circle_stroke(
                        Pos2::new(dot_x, y),
                        DOT_RADIUS,
                        Stroke::new(1.5, Color32::WHITE),
                    );

                    // Ref labels.
                    let mut label_x = dot_x + DOT_RADIUS + 6.0;
                    for ref_name in &node.refs {
                        let label_rect = paint_ref_label(&painter, Pos2::new(label_x, y), ref_name);
                        label_x += label_rect.width() + 4.0;
                    }

                    // Commit summary text.
                    let text_x = label_x + 4.0;
                    painter.text(
                        Pos2::new(text_x, y),
                        egui::Align2::LEFT_CENTER,
                        &node.summary,
                        egui::FontId::proportional(13.0),
                        Color32::from_gray(220),
                    );
                }
            });
    }
}

fn paint_ref_label(painter: &Painter, center: Pos2, name: &str) -> Rect {
    let font = egui::FontId::proportional(11.0);
    let galley = painter.layout_no_wrap(name.to_string(), font, Color32::WHITE);
    let pad = Vec2::new(5.0, 2.0);
    let rect = Rect::from_center_size(
        Pos2::new(center.x + galley.size().x / 2.0 + pad.x, center.y),
        galley.size() + pad * 2.0,
    );
    painter.rect_filled(rect, 3.0, Color32::from_rgb(60, 100, 180));
    painter.galley(rect.min + pad, galley, Color32::WHITE);
    rect
}
