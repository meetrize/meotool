//! 独立对话框 viewport 构建（设置 / 编辑 / 图标选择）。
//! 不使用主面板同窗浮层，避免黑边、抢焦点与尺寸抖动。

use eframe::egui::{self, ViewportBuilder, WindowLevel};

pub fn settings_viewport() -> ViewportBuilder {
    ViewportBuilder::default()
        .with_title("MeoTool 设置")
        .with_inner_size([720.0, 560.0])
        .with_min_inner_size([600.0, 420.0])
        .with_resizable(true)
        .with_decorations(true)
        .with_transparent(false)
        .with_window_level(WindowLevel::Normal)
}

pub fn script_editor_viewport(title: &str) -> ViewportBuilder {
    ViewportBuilder::default()
        .with_title(title)
        .with_inner_size([960.0, 680.0])
        .with_min_inner_size([720.0, 480.0])
        .with_resizable(true)
        .with_decorations(true)
        .with_transparent(false)
        .with_window_level(WindowLevel::Normal)
}

pub fn appearance_editor_viewport(title: &str) -> ViewportBuilder {
    ViewportBuilder::default()
        .with_title(title)
        .with_inner_size([480.0, 520.0])
        .with_min_inner_size([440.0, 420.0])
        .with_resizable(true)
        .with_decorations(true)
        .with_transparent(false)
        .with_window_level(WindowLevel::Normal)
}

/// 兼容旧合一窗口尺寸（egui 已拆分为 script / appearance viewport）。
pub fn button_editor_viewport(title: &str) -> ViewportBuilder {
    script_editor_viewport(title)
}

pub fn icon_picker_viewport() -> ViewportBuilder {
    ViewportBuilder::default()
        .with_title("选择图标")
        .with_inner_size([520.0, 480.0])
        .with_min_inner_size([400.0, 320.0])
        .with_resizable(true)
        .with_decorations(true)
        .with_transparent(false)
        .with_window_level(WindowLevel::Normal)
}

/// 对话框打开时：主面板降级并隐藏，避免 AlwaysOnTop 挡操作 / 抢焦点
pub fn demote_main_panel(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(WindowLevel::Normal));
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
}

pub fn restore_main_panel_level(ctx: &egui::Context, panel_visible: bool) {
    if panel_visible {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(WindowLevel::AlwaysOnTop));
    }
}
