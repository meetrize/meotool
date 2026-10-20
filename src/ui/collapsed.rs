use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Ui, Vec2, ViewportCommand};

use crate::config::{ButtonConfig, PanelConfig};
use crate::ui::button_menu::ButtonContextMenu;
use crate::ui::icons::IconCache;
use crate::ui::modes::paint_button;
use crate::ui::theme::Theme;

const LAUNCHER_W: f32 = PanelConfig::COLLAPSED_LAUNCHER_WIDTH;
const LAUNCHER_H: f32 = PanelConfig::COLLAPSED_LAUNCHER_HEIGHT;
/// 布局占位（含 bleed），保证桌宠与描边不被裁切
const LAUNCHER_ALLOC: Vec2 = Vec2::new(
    PanelConfig::COLLAPSED_LAUNCHER_WIDTH + PanelConfig::COLLAPSED_LAUNCHER_BLEED * 2.0,
    PanelConfig::COLLAPSED_LAUNCHER_HEIGHT + PanelConfig::COLLAPSED_LAUNCHER_BLEED * 2.0,
);
const COLLAPSE_DELAY: Duration = Duration::from_millis(500);

fn pointer_hovered(ctx: &egui::Context, rect: egui::Rect) -> bool {
    ctx.input(|i| i.pointer.hover_pos().is_some_and(|pos| rect.contains(pos)))
}

/// 判断屏幕坐标下的指针是否在 egui 矩形内。
/// 窗口启用 MousePassthrough 时 egui 收不到 hover，需回退到全局鼠标位置。
fn pointer_over_rect(ctx: &egui::Context, rect: egui::Rect) -> bool {
    if pointer_hovered(ctx, rect) {
        return true;
    }
    let Some(inner) = ctx.input(|i| i.viewport().inner_rect) else {
        return false;
    };
    let Some((mx, my)) = crate::platform::global_mouse_pos() else {
        return false;
    };
    let screen = egui::Rect::from_min_max(
        inner.min + rect.min.to_vec2(),
        inner.min + rect.max.to_vec2(),
    );
    screen.contains(egui::pos2(mx, my))
}

pub struct CollapsedState {
    pub expanded: bool,
    pub collapse_after: Option<Instant>,
    /// 当前是否已向系统请求鼠标穿透（避免每帧重复发命令）
    mouse_passthrough: bool,
}

impl Default for CollapsedState {
    fn default() -> Self {
        Self {
            expanded: false,
            collapse_after: None,
            mouse_passthrough: false,
        }
    }
}

impl CollapsedState {
    fn set_expanded(&mut self, expanded: bool) {
        if self.expanded == expanded {
            return;
        }
        self.expanded = expanded;
        if expanded {
            self.collapse_after = None;
        }
    }

    fn set_mouse_passthrough(&mut self, ctx: &egui::Context, enabled: bool) {
        if self.mouse_passthrough == enabled {
            return;
        }
        self.mouse_passthrough = enabled;
        ctx.send_viewport_cmd(ViewportCommand::MousePassthrough(enabled));
    }
}

pub struct CollapsedResponse {
    pub triggered: Option<usize>,
    pub needs_repaint: bool,
    pub viewport_size: (f32, f32),
    pub native_drag_started: bool,
    pub expand_to_full: bool,
}

pub fn show_collapsed(
    ui: &mut Ui,
    theme: &Theme,
    panel: &PanelConfig,
    buttons: &[ButtonConfig],
    state: &mut CollapsedState,
    icons: &mut IconCache,
    flash_index: Option<(usize, Color32)>,
    selected_cell: Option<(u8, u8)>,
    suppress_collapse: bool,
    menu: &mut ButtonContextMenu,
    launcher_menu: &mut crate::ui::button_menu::LauncherContextMenu,
) -> CollapsedResponse {
    let mut triggered = None;
    let mut native_drag_started = false;
    let mut expand_to_full = false;

    let (full_w, full_h) = panel.panel_size();
    // 布局与窗口始终占满展开尺寸；展开只切换绘制，尺寸不变 → 图标零闪动
    let content_w = LAUNCHER_ALLOC.x.max(full_w);
    let content_h = LAUNCHER_ALLOC.y + panel.gap_y + full_h;

    let origin = ui.available_rect_before_wrap().min;
    let launcher_rect = egui::Rect::from_min_size(
        egui::pos2(origin.x + (content_w - LAUNCHER_ALLOC.x) * 0.5, origin.y),
        LAUNCHER_ALLOC,
    );

    // 在分配布局前先检测图标 hover，同一帧即可展开
    if pointer_over_rect(ui.ctx(), launcher_rect) {
        state.set_expanded(true);
    }

    let expanded = state.expanded;
    let total_size = Vec2::new(content_w, content_h);
    let (rect, _response) = ui.allocate_exact_size(total_size, egui::Sense::hover());

    // 图标与按钮面板共用水平中心：面板左右边缘相对图标中心等距
    let launcher_rect = egui::Rect::from_min_size(
        egui::pos2(
            rect.min.x + (content_w - LAUNCHER_ALLOC.x) * 0.5,
            rect.min.y,
        ),
        LAUNCHER_ALLOC,
    );
    let launcher_resp = ui.interact(
        launcher_rect,
        egui::Id::new("collapsed_launcher"),
        egui::Sense::click().union(egui::Sense::drag()),
    );

    if launcher_resp.is_pointer_button_down_on()
        && ui.ctx().input(|i| {
            let Some(origin) = i.pointer.press_origin() else {
                return false;
            };
            let Some(current) = i.pointer.interact_pos() else {
                return false;
            };
            origin.distance(current) > 4.0
        })
    {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        native_drag_started = true;
    }

    if launcher_resp.secondary_clicked() {
        launcher_menu.on_secondary_click(&launcher_resp);
    }

    if launcher_resp.double_clicked() {
        expand_to_full = true;
    }

    paint_launcher(ui, theme, icons, panel, launcher_rect);

    // 面板在图标正下方，左右边缘相对图标中心等距
    let grid_rect = egui::Rect::from_min_size(
        egui::pos2(
            rect.min.x + (content_w - full_w) * 0.5,
            rect.min.y + LAUNCHER_ALLOC.y + panel.gap_y,
        ),
        Vec2::new(full_w, full_h),
    );

    let hit_rect = egui::Rect::from_min_max(rect.min, rect.min + Vec2::new(content_w, content_h));
    let over_launcher = pointer_over_rect(ui.ctx(), launcher_rect);
    let over_content = if expanded {
        pointer_over_rect(ui.ctx(), hit_rect)
    } else {
        over_launcher
    };

    if over_launcher || (expanded && over_content) {
        state.set_expanded(true);
    }

    if state.expanded {
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(grid_rect), |ui| {
            if let Some(index) = paint_grid(
                ui,
                theme,
                icons,
                panel,
                buttons,
                flash_index,
                selected_cell,
                menu,
            ) {
                triggered = Some(index);
            }
        });
    }

    if suppress_collapse {
        state.collapse_after = None;
    } else if over_content {
        state.collapse_after = None;
    } else if state.expanded {
        if state.collapse_after.is_none() {
            state.collapse_after = Some(Instant::now() + COLLAPSE_DELAY);
        } else if Instant::now() >= state.collapse_after.unwrap() {
            state.set_expanded(false);
        }
    }

    // 收起时空白区域鼠标穿透，避免挡桌面；悬停图标或展开时关闭穿透
    let want_passthrough = !state.expanded
        && !over_launcher
        && !launcher_menu.is_open()
        && !menu.is_open()
        && !suppress_collapse;
    state.set_mouse_passthrough(ui.ctx(), want_passthrough);

    CollapsedResponse {
        triggered,
        // 折叠倒计时需要准时唤醒；穿透态悬停靠主循环低频定时轮询，避免 60fps 空转
        needs_repaint: state.collapse_after.is_some() || launcher_menu.is_open(),
        viewport_size: panel.collapsed_viewport_size(true),
        native_drag_started,
        expand_to_full,
    }
}

fn paint_launcher(
    ui: &mut Ui,
    theme: &Theme,
    icons: &mut IconCache,
    panel: &PanelConfig,
    rect: egui::Rect,
) {
    let center = rect.center();
    let radius = LAUNCHER_H.min(LAUNCHER_W) / 2.0;

    // 配置了折叠图标时：彩色原图直接浮在透明窗口上，像桌面宠物
    if let Some(icon_name) = panel
        .collapsed_icon
        .as_ref()
        .filter(|s| !s.trim().is_empty())
    {
        if let Some(texture) = icons.get_colored(ui.ctx(), icon_name) {
            let tex_size = texture.size_vec2();
            let avail = Vec2::new(LAUNCHER_W, LAUNCHER_H);
            let scale = (avail.x / tex_size.x).min(avail.y / tex_size.y);
            let display = tex_size * scale;
            let icon_rect = egui::Rect::from_center_size(center, display);

            ui.painter().image(
                texture.id(),
                icon_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );

            return;
        }
    }

    // 无图标时回退为圆形启动器
    ui.painter().circle_filled(center, radius, theme.button_bg);
    if theme.button_border_enabled && theme.button_border_width > 0.0 {
        let inset = theme.button_border_width * 0.5;
        ui.painter().circle_stroke(
            center,
            radius - inset,
            egui::Stroke::new(theme.button_border_width, theme.button_border_color),
        );
    }

    ui.painter().text(
        center,
        egui::Align2::CENTER_CENTER,
        "⚡",
        theme.letter_font_id(),
        theme.letter_color,
    );
}

fn paint_grid(
    ui: &mut Ui,
    theme: &Theme,
    icons: &mut IconCache,
    panel: &PanelConfig,
    buttons: &[ButtonConfig],
    flash_index: Option<(usize, Color32)>,
    selected_cell: Option<(u8, u8)>,
    menu: &mut ButtonContextMenu,
) -> Option<usize> {
    let mut triggered = None;
    let button_size = egui::vec2(panel.button_width, panel.button_height);
    let cols = panel.cols as usize;

    egui::Grid::new("collapsed_grid_inner")
        .spacing(egui::vec2(panel.gap_x, panel.gap_y))
        .show(ui, |ui| {
            for (index, button) in buttons.iter().enumerate() {
                if index > 0 && index % cols == 0 {
                    ui.end_row();
                }
                let row = (index / cols) as u8;
                let col = (index % cols) as u8;
                let selected = selected_cell == Some((row, col));
                let flash = flash_index.filter(|(i, _)| *i == index).map(|(_, c)| c);
                let result = paint_button(
                    ui,
                    theme,
                    icons,
                    button,
                    index,
                    button_size,
                    panel.opacity,
                    panel.display_mode,
                    flash,
                    selected,
                    menu,
                );
                if result.clicked {
                    triggered = Some(index);
                }
            }
            ui.end_row();
        });

    triggered
}
