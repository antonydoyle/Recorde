//! Motion: shared durations, easing and animated reveal, fade and hover helpers.
//!
//! Transitions are short (100–200 ms, ease-out) and only describe a change that
//! already happened: content appearing, a panel opening, a hover. Direct
//! manipulation (staging, selection, scrolling) stays immediate. When reduced
//! motion is in effect every helper returns its end state on the first frame,
//! so layouts are identical with and without animation.
use std::sync::atomic::{AtomicBool, Ordering};

use egui::{Color32, Context, Id, Response, Ui};

/// Hover and press feedback, small fades.
pub const FAST: f32 = 0.12;
/// Accordions, reveals and disclosure sections.
pub const NORMAL: f32 = 0.18;
/// Panels, banners and notifications entering the window.
pub const SLOW: f32 = 0.24;

static REDUCED: AtomicBool = AtomicBool::new(false);

/// User preference; `System` follows the operating system's animation setting.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionPreference {
    #[default]
    System,
    Reduced,
    Full,
}

impl MotionPreference {
    pub const ALL: [Self; 3] = [Self::System, Self::Full, Self::Reduced];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "Match system setting",
            Self::Full => "Always animate",
            Self::Reduced => "Reduce motion",
        }
    }
}

/// Resolves and stores the preference, then updates egui's own animation time
/// (collapsing headers, windows and `animate_bool_responsive` users).
pub fn apply(ctx: &Context, preference: MotionPreference) {
    let reduced = match preference {
        MotionPreference::System => system_prefers_reduced_motion(),
        MotionPreference::Reduced => true,
        MotionPreference::Full => false,
    };
    REDUCED.store(reduced, Ordering::Relaxed);
    ctx.global_style_mut(|style| style.animation_time = duration(NORMAL));
}

pub fn reduced() -> bool {
    REDUCED.load(Ordering::Relaxed)
}

/// `seconds`, or zero when motion is reduced.
pub fn duration(seconds: f32) -> f32 {
    if reduced() {
        0.0
    } else {
        seconds
    }
}

/// Cubic ease-out: quick start, gentle settle.
pub fn ease_out(t: f32) -> f32 {
    let inverse = 1.0 - t.clamp(0.0, 1.0);
    1.0 - inverse * inverse * inverse
}

/// Eased 0→1 progress towards `on`. Repaints are requested while it moves.
pub fn progress(ctx: &Context, id: Id, on: bool, seconds: f32) -> f32 {
    ctx.animate_bool_with_time_and_easing(id, on, duration(seconds), ease_out)
}

/// Accordion: animates the height (and opacity) of `add` as `open` changes.
/// Returns `None` once fully closed. The id must be stable for the content,
/// for example `ui.id().with(("batch_actions", repo))`.
pub fn reveal<R>(ui: &mut Ui, id: Id, open: bool, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    use egui::collapsing_header::CollapsingState;
    let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, open);
    state.set_open(open);
    let openness = state.openness(ui.ctx());
    state
        .show_body_unindented(ui, |ui| {
            if openness < 1.0 {
                ui.multiply_opacity(openness);
            }
            add(ui)
        })
        .map(|inner| inner.inner)
}

/// Like [`reveal`], but the caller keeps the open flag in egui memory under `id`
/// and only toggles it. Useful for "Show advanced" disclosures.
pub fn is_open(ctx: &Context, id: Id, default_open: bool) -> bool {
    egui::collapsing_header::CollapsingState::load_with_default_open(ctx, id, default_open)
        .is_open()
}

pub fn set_open(ctx: &Context, id: Id, open: bool) {
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ctx, id, open);
    state.set_open(open);
    state.store(ctx);
}

/// Openness 0..1 of a disclosure stored under `id` (for chevron rotation).
pub fn openness(ctx: &Context, id: Id, default_open: bool) -> f32 {
    egui::collapsing_header::CollapsingState::load_with_default_open(ctx, id, default_open)
        .openness(ctx)
}

/// Fades content in while `visible`; returns the opacity that was applied.
pub fn fade(ui: &mut Ui, id: Id, visible: bool) -> f32 {
    let t = progress(ui.ctx(), id, visible, FAST);
    if t < 1.0 {
        ui.multiply_opacity(t);
    }
    t
}

/// Hover progress for a response, for colour transitions on custom rows.
pub fn hover(ui: &Ui, response: &Response) -> f32 {
    progress(
        ui.ctx(),
        response.id.with("motion_hover"),
        response.hovered() || response.has_focus(),
        FAST,
    )
}

/// Blends two colours by `t` (gamma space, matching egui's own visuals).
pub fn mix(from: Color32, to: Color32, t: f32) -> Color32 {
    from.lerp_to_gamma(to, t.clamp(0.0, 1.0))
}

/// Vertical slide distance for an element entering from above (negative) or
/// below (positive): `distance * (1 - progress)`.
pub fn slide(progress: f32, distance: f32) -> f32 {
    distance * (1.0 - progress.clamp(0.0, 1.0))
}

#[cfg(windows)]
fn system_prefers_reduced_motion() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION,
    };
    let mut enabled: i32 = 1;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL to the provided pointer.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut enabled as *mut i32).cast(),
            0,
        )
    };
    ok != 0 && enabled == 0
}

#[cfg(not(windows))]
fn system_prefers_reduced_motion() -> bool {
    // GNOME/KDE and macOS expose this through desktop settings that need a
    // subprocess or platform bindings; keep startup free of probes and let the
    // explicit preference cover those platforms.
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_is_bounded_and_monotonic() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert_eq!(ease_out(-1.0), 0.0);
        assert_eq!(ease_out(2.0), 1.0);
        let mut previous = 0.0;
        for step in 1..=20 {
            let value = ease_out(step as f32 / 20.0);
            assert!(value >= previous);
            previous = value;
        }
        assert!(ease_out(0.5) > 0.5, "ease-out moves quickly at the start");
    }

    #[test]
    fn slide_reaches_rest_position() {
        assert_eq!(slide(0.0, -12.0), -12.0);
        assert_eq!(slide(1.0, -12.0), 0.0);
        assert_eq!(slide(0.5, 10.0), 5.0);
    }

    #[test]
    fn reduced_motion_makes_reveals_instant() {
        let ctx = Context::default();
        apply(&ctx, MotionPreference::Reduced);
        assert!(reduced());
        assert_eq!(duration(NORMAL), 0.0);
        assert_eq!(ctx.global_style().animation_time, 0.0);
        let id = Id::new("reveal_test");
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(300.0, 200.0),
            )),
            ..Default::default()
        });
        let mut ui = Ui::new(
            ctx.clone(),
            Id::new("reveal_test_ui"),
            egui::UiBuilder::new().max_rect(ctx.content_rect()),
        );
        let shown = reveal(&mut ui, id, true, |ui| ui.label("content")).is_some();
        ctx.end_pass().textures_delta.clear();
        assert!(shown, "an open reveal renders its content immediately");
        apply(&ctx, MotionPreference::Full);
        assert!(!reduced());
        assert_eq!(ctx.global_style().animation_time, NORMAL);
    }
}
