use egui::{Color32, FontFamily, FontId};

use crate::config::model::UiStyle;
use crate::ui::egui_color::to_egui;

pub const CUSTOM_FONT_KEY: &str = "custom_letter";

/// 运行时界面主题（由配置解析而来）
#[derive(Clone)]
pub struct Theme {
    pub background: Color32,
    pub button_bg: Color32,
    pub letter_bg: Color32,
    pub letter_color: Color32,
    pub letter_size: f32,
    pub letter_font: Option<String>,
    pub window_border_enabled: bool,
    pub window_border_width: f32,
    pub window_border_color: Color32,
    pub button_border_enabled: bool,
    pub button_border_width: f32,
    pub button_border_color: Color32,
    pub success_flash: Color32,
    pub error_flash: Color32,
    pub selection: Color32,
    pub corner_radius: f32,
    pub icon_size: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Self::from_ui_style(&UiStyle::default())
    }
}

impl Theme {
    pub fn from_ui_style(style: &UiStyle) -> Self {
        Self {
            background: to_egui(style.background),
            button_bg: to_egui(style.button_bg),
            letter_bg: to_egui(style.letter_bg),
            letter_color: to_egui(style.letter_color),
            letter_size: style.letter_size,
            letter_font: style.letter_font.clone(),
            window_border_enabled: style.window_border_enabled,
            window_border_width: style.window_border_width,
            window_border_color: to_egui(style.window_border_color),
            button_border_enabled: style.button_border_enabled,
            button_border_width: style.button_border_width,
            button_border_color: to_egui(style.button_border_color),
            success_flash: Color32::from_rgba_unmultiplied(50, 180, 80, 200),
            error_flash: Color32::from_rgba_unmultiplied(200, 60, 60, 200),
            selection: Color32::from_rgba_unmultiplied(0, 122, 255, 153),
            corner_radius: 8.0,
            icon_size: 32.0,
        }
    }

    pub fn letter_font_id(&self) -> FontId {
        if self.letter_font.is_some() {
            FontId::new(self.letter_size, FontFamily::Name(CUSTOM_FONT_KEY.into()))
        } else {
            FontId::proportional(self.letter_size)
        }
    }

    pub fn scaled_alpha(color: Color32, opacity: f32) -> Color32 {
        Color32::from_rgba_unmultiplied(
            color.r(),
            color.g(),
            color.b(),
            (color.a() as f32 * opacity).clamp(0.0, 255.0) as u8,
        )
    }

    pub fn button_frame(&self, opacity: f32) -> egui::Frame {
        let bg = Self::scaled_alpha(self.button_bg, opacity);
        let mut frame = egui::Frame::none().fill(bg).rounding(self.corner_radius);
        if self.button_border_enabled && self.button_border_width > 0.0 {
            let stroke_color = Self::scaled_alpha(self.button_border_color, opacity);
            frame = frame.stroke(egui::Stroke::new(self.button_border_width, stroke_color));
        }
        frame
    }

    pub fn panel_frame(&self, opacity: f32, content_alpha: f32) -> egui::Frame {
        let combined = opacity * content_alpha;
        let bg = Self::scaled_alpha(self.background, combined);
        let mut frame = egui::Frame::none().fill(bg);
        if self.window_border_enabled && self.window_border_width > 0.0 {
            let stroke_color = Self::scaled_alpha(self.window_border_color, combined);
            frame = frame.stroke(egui::Stroke::new(self.window_border_width, stroke_color));
        }
        frame
    }
}
