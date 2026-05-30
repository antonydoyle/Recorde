use egui::{Color32, Context, Stroke, Visuals};

use crate::state::{new_shared_state, SharedState};
use crate::theme;
use crate::ui::{
    commit_graph::CommitGraphView,
    detail_panel::DetailPanel,
    sidebar::Sidebar,
    status_bar::StatusBar,
    toolbar::Toolbar,
};

pub struct RecordeApp {
    state: SharedState,
    toolbar: Toolbar,
    sidebar: Sidebar,
    commit_graph_view: CommitGraphView,
    detail_panel: DetailPanel,
    status_bar: StatusBar,
}

impl RecordeApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_repo: Option<std::path::PathBuf>) -> Self {
        apply_theme(&cc.egui_ctx);

        let state = new_shared_state();

        if let Some(path) = initial_repo {
            crate::git::operations::load_repo(path, state.clone());
        }

        Self {
            toolbar: Toolbar::new(state.clone()),
            sidebar: Sidebar::new(state.clone()),
            commit_graph_view: CommitGraphView::new(state.clone()),
            detail_panel: DetailPanel::new(state.clone()),
            status_bar: StatusBar::new(state.clone()),
            state,
        }
    }
}

impl eframe::App for RecordeApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.status_bar.show(ctx);
        self.toolbar.show(ctx);

        egui::SidePanel::left("sidebar")
            .resizable(true)
            .default_width(220.0)
            .min_width(150.0)
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                self.sidebar.show(ui);
            });

        egui::SidePanel::right("detail_panel")
            .resizable(true)
            .default_width(320.0)
            .min_width(200.0)
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                self.detail_panel.show(ui);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::BG_BASE))
            .show(ctx, |ui| {
                self.commit_graph_view.show(ui);
            });
    }
}

fn apply_theme(ctx: &Context) {
    let mut v = Visuals::dark();

    v.panel_fill = theme::BG_BASE;
    v.window_fill = theme::BG_BASE;
    v.faint_bg_color = theme::BG_SURFACE;
    v.extreme_bg_color = Color32::from_rgb(0x12, 0x14, 0x18);
    v.override_text_color = Some(theme::TEXT_PRIMARY);

    v.widgets.noninteractive.bg_fill = theme::BG_SURFACE;
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, theme::TEXT_SECONDARY);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, theme::BORDER);

    v.widgets.inactive.bg_fill = theme::BG_ELEVATED;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, theme::TEXT_SECONDARY);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, theme::BORDER);

    v.widgets.hovered.bg_fill = Color32::from_rgb(0x35, 0x3A, 0x4A);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, theme::TEXT_PRIMARY);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, theme::ACCENT_BLUE);

    v.widgets.active.bg_fill = theme::ACCENT_BLUE;
    v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);

    v.selection.bg_fill = Color32::from_rgba_premultiplied(0x4D, 0x88, 0xFF, 50);
    v.selection.stroke = Stroke::new(1.0, theme::ACCENT_BLUE);

    v.window_stroke = Stroke::new(1.0, theme::BORDER);
    v.popup_shadow = egui::Shadow::NONE;
    v.window_shadow = egui::Shadow::NONE;

    ctx.set_visuals(v);

    // Slightly larger default font
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default();
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::Vec2::new(6.0, 3.0);
    style.spacing.window_margin = egui::Margin::same(0.0);
    style.spacing.button_padding = egui::Vec2::new(8.0, 4.0);
    ctx.set_style(style);
}
