use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Context};

const HINT_DURATION: Duration = Duration::from_millis(1500);
const SWITCH_DURATION: Duration = Duration::from_millis(100);

/// Profile 切换过渡动画
pub struct ProfileTransition {
    started: Instant,
    midpoint_applied: bool,
    pending_panel: Option<crate::config::PanelConfig>,
    pending_profile: String,
}

impl ProfileTransition {
    pub fn new(panel: crate::config::PanelConfig, profile_name: String) -> Self {
        Self {
            started: Instant::now(),
            midpoint_applied: false,
            pending_panel: Some(panel),
            pending_profile: profile_name,
        }
    }

    pub fn progress(&self) -> f32 {
        (self.started.elapsed().as_secs_f32() / SWITCH_DURATION.as_secs_f32()).clamp(0.0, 1.0)
    }

    pub fn content_alpha(&self) -> f32 {
        let t = self.progress();
        if t < 0.5 {
            1.0 - t * 2.0
        } else {
            (t - 0.5) * 2.0
        }
    }

    pub fn should_apply_pending(&self) -> bool {
        !self.midpoint_applied && self.progress() >= 0.5
    }

    pub fn take_pending(&mut self) -> Option<(crate::config::PanelConfig, String)> {
        if !self.should_apply_pending() {
            return None;
        }
        let panel = self.pending_panel.take()?;
        Some((panel, self.pending_profile.clone()))
    }

    pub fn is_finished(&self) -> bool {
        self.started.elapsed() >= SWITCH_DURATION
    }
}

pub struct HintOverlay {
    text: String,
    until: Instant,
}

impl HintOverlay {
    pub fn show(text: String) -> Self {
        Self {
            text,
            until: Instant::now() + HINT_DURATION,
        }
    }

    pub fn is_active(&self) -> bool {
        Instant::now() < self.until
    }

    pub fn paint(&self, ctx: &Context) {
        if !self.is_active() {
            return;
        }

        let elapsed =
            HINT_DURATION.saturating_sub(self.until.saturating_duration_since(Instant::now()));
        let fade = 1.0 - (elapsed.as_secs_f32() / HINT_DURATION.as_secs_f32());
        let alpha = (fade * 220.0) as u8;

        egui::Area::new(egui::Id::new("profile_hint"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-8.0, 8.0))
            .interactable(false)
            .show(ctx, |ui| {
                let frame = egui::Frame::none()
                    .fill(Color32::from_rgba_unmultiplied(20, 20, 20, alpha))
                    .inner_margin(egui::Margin::symmetric(8.0, 4.0))
                    .rounding(4.0);
                frame.show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(&self.text)
                            .size(11.0)
                            .color(Color32::from_rgba_unmultiplied(230, 230, 230, alpha)),
                    );
                });
            });
    }
}
