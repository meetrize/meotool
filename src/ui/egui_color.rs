//! 配置色 ↔ egui::Color32（仅 ui-egui 路径使用）。

use egui::Color32;

use crate::config::color::RgbColor;

pub fn to_egui(c: RgbColor) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

pub fn from_egui(c: Color32) -> RgbColor {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    RgbColor::rgba(r, g, b, a)
}
