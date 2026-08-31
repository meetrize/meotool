use egui::{Color32, FontId, Ui, Vec2};

use crate::config::color::parse_color;
use crate::config::{ButtonConfig, DisplayMode};
use crate::ui::button_menu::ButtonContextMenu;
use crate::ui::icons::{resolve_icon_tint, IconCache};
use crate::ui::theme::Theme;

pub struct ButtonPaintResult {
    pub clicked: bool,
}

/// 按显示模式绘制单个按钮
pub fn paint_button(
    ui: &mut Ui,
    theme: &Theme,
    icons: &mut IconCache,
    button: &ButtonConfig,
    index: usize,
    size: Vec2,
    opacity: f32,
    mode: DisplayMode,
    flash: Option<Color32>,
    selected: bool,
    menu: &mut ButtonContextMenu,
) -> ButtonPaintResult {
    let mut frame = build_button_frame(theme, button, opacity);
    if selected {
        frame = frame.stroke(egui::Stroke::new(2.0, theme.selection));
    }
    if let Some(color) = flash {
        frame = frame.fill(color);
    }

    let mut clicked = false;
    let enabled = button.enabled;

    frame.show(ui, |ui| {
        let response = match mode {
            DisplayMode::Text => paint_text(ui, theme, button, size, enabled),
            DisplayMode::Icon => paint_icon(
                ui, theme, icons, button, size, false, index, opacity, enabled,
            ),
            DisplayMode::IndexIcon => paint_icon(
                ui, theme, icons, button, size, true, index, opacity, enabled,
            ),
        };

        if response.secondary_clicked() {
            menu.on_secondary_click(index, &response);
        }

        if response.clicked() && enabled {
            clicked = true;
        }
        if response.hovered() && enabled && mode != DisplayMode::Text {
            response.on_hover_text(&button.label);
        }
    });

    ButtonPaintResult { clicked }
}

fn build_button_frame(theme: &Theme, button: &ButtonConfig, opacity: f32) -> egui::Frame {
    if button.background_color.is_none() && button.border_color.is_none() {
        return theme.button_frame(opacity);
    }

    let bg = button
        .background_color
        .as_deref()
        .and_then(parse_color)
        .map(crate::ui::egui_color::to_egui)
        .unwrap_or(theme.button_bg);
    let bg = Theme::scaled_alpha(bg, opacity);

    let mut frame = egui::Frame::none().fill(bg).rounding(theme.corner_radius);

    let border_color = button
        .border_color
        .as_deref()
        .and_then(parse_color)
        .map(crate::ui::egui_color::to_egui)
        .unwrap_or(theme.button_border_color);
    let border_override = button.border_color.is_some();
    let border_enabled = theme.button_border_enabled || border_override;
    let border_width = if border_override && theme.button_border_width <= 0.0 {
        1.0
    } else {
        theme.button_border_width
    };

    if border_enabled && border_width > 0.0 {
        let stroke_color = Theme::scaled_alpha(border_color, opacity);
        frame = frame.stroke(egui::Stroke::new(border_width, stroke_color));
    }

    frame
}

fn inactive_color(color: Color32) -> Color32 {
    color.gamma_multiply(0.5)
}

fn paint_text(
    ui: &mut Ui,
    theme: &Theme,
    button: &ButtonConfig,
    size: Vec2,
    active: bool,
) -> egui::Response {
    let label = if button.label.is_empty() {
        " "
    } else {
        &button.label
    };
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());

    if ui.is_rect_visible(rect) {
        let color = if active {
            theme.letter_color
        } else {
            inactive_color(theme.letter_color)
        };
        let text_pos = rect.center() + egui::vec2(0.0, theme.letter_size * 0.18);
        ui.painter().text(
            text_pos,
            egui::Align2::CENTER_CENTER,
            label,
            theme.letter_font_id(),
            color,
        );
    }

    response
}

fn paint_icon(
    ui: &mut Ui,
    theme: &Theme,
    icons: &mut IconCache,
    button: &ButtonConfig,
    size: Vec2,
    show_index: bool,
    index: usize,
    opacity: f32,
    active: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let icon_tint = resolve_icon_tint(button.icon_color.as_deref(), theme.letter_color, active);

    if ui.is_rect_visible(rect) {
        if show_index && button.enabled {
            let badge = format!("{}", index + 1);
            let badge_size = theme.letter_size.max(10.0);
            let badge_rect = egui::Rect::from_min_size(
                rect.min + egui::vec2(4.0, 4.0),
                egui::vec2(badge_size + 6.0, badge_size + 2.0),
            );
            let badge_bg = Theme::scaled_alpha(theme.letter_bg, opacity);
            ui.painter().rect_filled(badge_rect, 4.0, badge_bg);
            ui.painter().text(
                badge_rect.center(),
                egui::Align2::CENTER_CENTER,
                badge,
                theme.letter_font_id(),
                theme.letter_color,
            );
        }

        let icon_size = theme.icon_size;
        let icon_rect =
            egui::Rect::from_center_size(rect.center(), egui::vec2(icon_size, icon_size));

        if let Some(icon_name) = &button.icon {
            if let Some(texture) = icons.get(ui.ctx(), icon_name) {
                ui.painter().image(
                    texture.id(),
                    icon_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    icon_tint,
                );
            } else {
                paint_fallback_char(ui, theme, button, icon_rect, active, icon_tint);
            }
        } else if !button.label.is_empty() {
            paint_fallback_char(ui, theme, button, icon_rect, active, icon_tint);
        }
    }

    response
}

fn paint_fallback_char(
    ui: &mut Ui,
    theme: &Theme,
    button: &ButtonConfig,
    rect: egui::Rect,
    active: bool,
    icon_tint: Color32,
) {
    let ch = button.label.chars().next().unwrap_or('?');
    let char_size = theme.letter_size.max(theme.icon_size * 0.6);
    let font_id = if theme.letter_font.is_some() {
        FontId::new(char_size, theme.letter_font_id().family)
    } else {
        egui::FontId::proportional(char_size)
    };
    let color = if button.icon_color.is_some() {
        icon_tint
    } else if active {
        theme.letter_color
    } else {
        inactive_color(theme.letter_color)
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        ch.to_string(),
        font_id,
        color,
    );
}
