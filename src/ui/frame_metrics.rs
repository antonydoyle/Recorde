//! Opt-in native CPU-frame measurements for disposable acceptance fixtures.
pub(crate) struct FrameMetrics {
    enabled: bool,
    frames: usize,
    samples: Vec<f32>,
}

impl Default for FrameMetrics {
    fn default() -> Self {
        Self {
            enabled: std::env::var("RECORDE_PROFILE_FRAMES").as_deref() == Ok("1"),
            frames: 0,
            samples: Vec::new(),
        }
    }
}

impl FrameMetrics {
    pub fn observe(&mut self, ctx: &egui::Context, seconds: Option<f32>, ready: bool) {
        if !self.enabled {
            return;
        }
        ctx.request_repaint();
        if !ready {
            return;
        }
        if let Some(seconds) = seconds.filter(|value| value.is_finite() && *value >= 0.0) {
            self.frames += 1;
            if self.frames > 20 {
                self.samples.push(seconds);
            }
            if self.samples.len() == 220 {
                self.samples.sort_by(f32::total_cmp);
                eprintln!("RECORDE_FRAME_METRICS samples=220 warmup=20 p95_ms={:.3} max_ms={:.3} scope=app_update_and_render_cpu_excluding_vsync", self.samples[209] * 1000.0, self.samples[219] * 1000.0);
                self.enabled = false;
            }
        }
    }
}
