use eframe::egui::{self, Margin, ViewportBuilder, ViewportCommand, WindowLevel};

use crate::config::PanelConfig;
use crate::ui::icons::IconCache;
use crate::ui::theme::Theme;

const PIN_ICON: &str = "pin.png";
const PINNED_ICON: &str = "pinned.png";
const COLLAPSE_FALLBACK_ICON: &str = "layout-bottombar-collapse.png";
/// 图标绘制尺寸
const HEADER_ICON_SIZE: f32 = 14.0;
/// 折叠桌宠图标在顶栏中的显示区域（横版略宽）
const HEADER_COLLAPSE_ICON_SIZE: egui::Vec2 = egui::vec2(20.0, 14.0);
/// 顶栏每个操作按钮的透明点击区域宽度
const HEADER_ACTION_HIT_WIDTH: f32 = 30.0;
const HEADER_ACTIONS_WIDTH: f32 = HEADER_ACTION_HIT_WIDTH * 2.0;
const HEADER_APP_NAME_LEFT_INSET: f32 = 28.0;
const HEADER_APP_NAME_RIGHT_INSET: f32 = 8.0;
/// 顶栏应用名最大字符数（按 Unicode 标量），避免长中文标题撑爆字体图集
const HEADER_APP_NAME_MAX_CHARS: usize = 18;
/// tooltip 最大字符数
const HEADER_TOOLTIP_MAX_CHARS: usize = 80;

/// 构建透明置顶无边框窗口选项
pub fn native_options(
    panel_width: f32,
    panel_height: f32,
    x: f32,
    y: f32,
) -> eframe::NativeOptions {
    let mut options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("MeoTool")
            .with_inner_size([panel_width, panel_height])
            .with_position([x, y])
            .with_transparent(true)
            .with_decorations(false)
            .with_resizable(false)
            .with_always_on_top()
            .with_window_level(WindowLevel::AlwaysOnTop),
        vsync: true,
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        ..Default::default()
    };
    #[cfg(feature = "renderer-wgpu")]
    {
        options.renderer = eframe::Renderer::Wgpu;
    }
    #[cfg(all(feature = "renderer-glow", not(feature = "renderer-wgpu")))]
    {
        options.renderer = eframe::Renderer::Glow;
    }
    let _ = &mut options;
    options
}

pub struct PanelHeaderResponse {
    /// 本帧在标题栏发起了系统原生拖拽
    pub native_drag_started: bool,
    /// 本帧点击了钉住按钮
    pub pin_clicked: bool,
    /// 本帧点击了折叠模式按钮
    pub collapse_clicked: bool,
}

/// 顶栏：整行可拖拽移动窗口，居中显示当前激活应用名，右上角折叠 / 钉住按钮
pub fn show_panel_header(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    icons: &mut IconCache,
    pinned: bool,
    collapse_icon: &str,
    active_app_name: Option<&str>,
    active_app_tooltip: Option<&str>,
    header_tooltip: Option<&str>,
) -> PanelHeaderResponse {
    let mut native_drag_started = false;
    let mut pin_clicked = false;
    let mut collapse_clicked = false;

    let top = ui.available_rect_before_wrap();
    let header_rect = egui::Rect::from_min_size(
        top.min,
        egui::vec2(top.width(), PanelConfig::PANEL_HEADER_HEIGHT),
    );
    ui.advance_cursor_after_rect(header_rect);

    let collapse_hit_rect = egui::Rect::from_min_size(
        egui::pos2(header_rect.max.x - HEADER_ACTIONS_WIDTH, header_rect.min.y),
        egui::vec2(HEADER_ACTION_HIT_WIDTH, header_rect.height()),
    );
    let collapse_icon_rect =
        egui::Rect::from_center_size(collapse_hit_rect.center(), HEADER_COLLAPSE_ICON_SIZE);
    let collapse_response = ui.interact(
        collapse_hit_rect,
        ui.id().with("panel_collapse"),
        egui::Sense::click(),
    );
    if collapse_response.clicked() {
        collapse_clicked = true;
    }
    collapse_response.on_hover_text("切换为折叠模式");
    paint_header_collapse_icon(ui, ctx, icons, collapse_icon_rect, collapse_icon);

    let pin_hit_rect = egui::Rect::from_min_size(
        egui::pos2(
            header_rect.max.x - HEADER_ACTION_HIT_WIDTH,
            header_rect.min.y,
        ),
        egui::vec2(HEADER_ACTION_HIT_WIDTH, header_rect.height()),
    );
    let pin_icon_rect = egui::Rect::from_center_size(
        pin_hit_rect.center(),
        egui::vec2(HEADER_ICON_SIZE, HEADER_ICON_SIZE),
    );
    let pin_response = ui.interact(
        pin_hit_rect,
        ui.id().with("panel_pin"),
        egui::Sense::click(),
    );
    if pin_response.clicked() {
        pin_clicked = true;
    }
    pin_response.on_hover_text(if pinned {
        "取消钉住（恢复自动隐藏）"
    } else {
        "钉住面板（失焦不自动隐藏）"
    });
    paint_header_icon(
        ui,
        ctx,
        icons,
        pin_icon_rect,
        if pinned { PINNED_ICON } else { PIN_ICON },
    );

    let drag_rect = egui::Rect::from_min_max(
        header_rect.min,
        egui::pos2(header_rect.max.x - HEADER_ACTIONS_WIDTH, header_rect.max.y),
    );
    let app_name_rect = header_app_name_rect(drag_rect);
    let drag = ui.interact(
        drag_rect,
        ui.id().with("panel_header_drag"),
        egui::Sense::drag(),
    );

    if drag.is_pointer_button_down_on()
        && ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary))
    {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        native_drag_started = true;
    }
    if let Some(tooltip) = header_tooltip.filter(|text| !text.is_empty()) {
        drag.on_hover_text(truncate_chars(tooltip, HEADER_TOOLTIP_MAX_CHARS));
    }

    paint_drag_grip(
        ui.painter(),
        header_rect.left_center() + egui::vec2(12.0, 0.0),
        ui.visuals().weak_text_color(),
    );

    if let Some(name) = active_app_name.filter(|name| !name.is_empty()) {
        let display = truncate_chars(name, HEADER_APP_NAME_MAX_CHARS);
        if let Some(tooltip) = active_app_tooltip.filter(|text| !text.is_empty()) {
            let label = ui.interact(
                app_name_rect,
                ui.id().with("panel_header_app_name"),
                egui::Sense::click(),
            );
            label.on_hover_text(truncate_chars(tooltip, HEADER_TOOLTIP_MAX_CHARS));
        }
        paint_header_app_name(ui, drag_rect, &display);
    }

    PanelHeaderResponse {
        native_drag_started,
        pin_clicked,
        collapse_clicked,
    }
}

/// 透明背景 CentralPanel
pub fn show_panel<F: FnOnce(&mut egui::Ui)>(
    ctx: &egui::Context,
    theme: &Theme,
    opacity: f32,
    content_alpha: f32,
    pet_mode: bool,
    content: F,
) -> egui::Response {
    // 折叠桌宠模式：完全透明窗体，只显示图标本身
    let panel_frame = if pet_mode {
        egui::Frame::none().inner_margin(Margin::same(PanelConfig::PANEL_PADDING))
    } else {
        theme
            .panel_frame(opacity, content_alpha)
            .inner_margin(Margin::same(PanelConfig::PANEL_PADDING))
    };

    let response = egui::CentralPanel::default()
        .frame(panel_frame)
        .show(ctx, |ui| {
            ui.set_opacity(content_alpha);
            content(ui);
        })
        .response;

    response
}

pub fn move_viewport(ctx: &egui::Context, pos: egui::Pos2) {
    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
}

pub fn resize_viewport(ctx: &egui::Context, width: f32, height: f32) {
    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(width, height)));
}

pub fn set_viewport_visible(ctx: &egui::Context, visible: bool) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(visible));
}

/// 面板隐藏时缩到最小，降低透明窗口后备缓冲占用
pub fn shrink_viewport_when_hidden(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1.0, 1.0)));
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// 2×3 圆点拖拽手柄（不依赖 Unicode/字体，避免 CJK 字体缺字显示为方框）
fn paint_drag_grip(painter: &egui::Painter, center: egui::Pos2, color: egui::Color32) {
    const COLS: usize = 2;
    const ROWS: usize = 3;
    let dot_radius = 1.4;
    let spacing = 3.8;
    let origin = center - egui::vec2(spacing / 2.0, (ROWS as f32 - 1.0) * spacing / 2.0);

    for row in 0..ROWS {
        for col in 0..COLS {
            let pos = origin + egui::vec2(col as f32 * spacing, row as f32 * spacing);
            painter.circle_filled(pos, dot_radius, color);
        }
    }
}

fn header_app_name_rect(drag_rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(
            drag_rect.min.x + HEADER_APP_NAME_LEFT_INSET,
            drag_rect.min.y,
        ),
        egui::pos2(
            drag_rect.max.x - HEADER_APP_NAME_RIGHT_INSET,
            drag_rect.max.y,
        ),
    )
}

fn paint_header_app_name(ui: &egui::Ui, drag_rect: egui::Rect, name: &str) {
    let text_rect = header_app_name_rect(drag_rect);
    if text_rect.width() <= 0.0 {
        return;
    }

    let font_id = egui::TextStyle::Small.resolve(ui.style());
    let color = ui.visuals().weak_text_color();
    let galley = ui
        .painter()
        .layout(name.to_owned(), font_id, color, text_rect.width());
    let pos = egui::pos2(
        text_rect.center().x - galley.size().x / 2.0,
        text_rect.center().y - galley.size().y / 2.0,
    );
    ui.painter()
        .with_clip_rect(text_rect)
        .galley(pos, galley, color);
}

fn paint_header_icon(
    ui: &egui::Ui,
    ctx: &egui::Context,
    icons: &mut IconCache,
    rect: egui::Rect,
    icon_name: &str,
) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let Some(texture) = icons.get(ctx, icon_name) else {
        return;
    };
    let icon_rect = rect.shrink(1.0);
    ui.painter().image(
        texture.id(),
        icon_rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        ui.visuals().text_color(),
    );
}

/// 顶栏折叠按钮：使用折叠桌宠彩色原图（与菜单栏 / 折叠模式一致）
fn paint_header_collapse_icon(
    ui: &egui::Ui,
    ctx: &egui::Context,
    icons: &mut IconCache,
    rect: egui::Rect,
    icon_name: &str,
) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let name = icon_name.trim();
    let name = if name.is_empty() {
        PanelConfig::DEFAULT_COLLAPSED_ICON
    } else {
        name
    };
    if let Some(texture) = icons.get_colored(ctx, name) {
        let tex_size = texture.size_vec2();
        if tex_size.x > 0.0 && tex_size.y > 0.0 {
            let avail = rect.shrink(1.0).size();
            let scale = (avail.x / tex_size.x).min(avail.y / tex_size.y);
            let display = tex_size * scale;
            let icon_rect = egui::Rect::from_center_size(rect.center(), display);
            ui.painter().image(
                texture.id(),
                icon_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            return;
        }
    }
    paint_header_icon(ui, ctx, icons, rect, COLLAPSE_FALLBACK_ICON);
}
