use eframe::egui::{self, Context, ViewportBuilder, ViewportId, WindowLevel};

use crate::platform::{
    check_permissions, executable_path, open_accessibility_settings,
    open_input_monitoring_settings, relaunch_app, PermissionStatus,
};

const PERMISSION_VIEWPORT: &str = "meotool_permission_dialog";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingOpen {
    Accessibility,
    InputMonitoring,
}

/// 启动时权限引导：独立普通窗口（非 AlwaysOnTop），避免黑边与抢系统密码框焦点。
pub struct PermissionDialog {
    status: PermissionStatus,
    dismissed: bool,
    centered: bool,
    /// 用户已点「打开设置」，暂时让出前台给系统偏好设置 / 密码框
    yielded_for_system_settings: bool,
    pending_open: Option<PendingOpen>,
}

impl PermissionDialog {
    pub fn new() -> Self {
        Self {
            status: check_permissions(),
            dismissed: false,
            centered: false,
            yielded_for_system_settings: false,
            pending_open: None,
        }
    }

    pub fn should_show(&self) -> bool {
        !self.dismissed && !self.status.all_granted()
    }

    /// 重新检测：先探测，若仍缺失则重启应用以刷新 macOS TCC 缓存。
    pub fn refresh(&mut self) {
        self.status = check_permissions();
        if self.status.all_granted() {
            self.dismissed = true;
            return;
        }
        relaunch_app();
    }

    pub fn show(&mut self, ctx: &Context) -> bool {
        if !self.should_show() {
            return false;
        }

        // 处理上一帧按钮请求：先让主窗退让，再打开系统设置
        if let Some(pending) = self.pending_open.take() {
            self.yielded_for_system_settings = true;
            demote_and_hide_root(ctx);
            match pending {
                PendingOpen::Accessibility => open_accessibility_settings(),
                PendingOpen::InputMonitoring => open_input_monitoring_settings(),
            }
        }

        let viewport_id = ViewportId::from_hash_of(PERMISSION_VIEWPORT);
        // 独立系统窗口 + 普通层级 + 有边框 + 不透明：CentralPanel 铺满，无两侧黑边
        let builder = ViewportBuilder::default()
            .with_title("MeoTool 需要系统权限")
            .with_inner_size([520.0, 380.0])
            .with_min_inner_size([480.0, 340.0])
            .with_resizable(false)
            .with_decorations(true)
            .with_transparent(false)
            .with_window_level(WindowLevel::Normal);

        ctx.show_viewport_immediate(viewport_id, builder, |ctx, _class| {
            if !self.centered {
                if let Some(cmd) = egui::ViewportCommand::center_on_screen(ctx) {
                    ctx.send_viewport_cmd(cmd);
                }
                // 仅第一次显示时聚焦一次；yield 给系统设置后不再抢焦点
                if !self.yielded_for_system_settings {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                self.centered = true;
            }

            egui::CentralPanel::default().show(ctx, |ui| {
                self.draw_body(ui);
            });

            if ctx.input(|i| i.viewport().close_requested()) {
                self.dismissed = true;
            }
        });

        !self.status.all_granted() && !self.dismissed
    }

    fn draw_body(&mut self, ui: &mut egui::Ui) {
        ui.heading("需要以下系统权限");
        ui.add_space(8.0);
        ui.label("用于窗口感知、模拟快捷键和全局热键：");
        ui.add_space(8.0);

        if !self.status.accessibility {
            ui.horizontal(|ui| {
                ui.label("• 辅助功能 — 检测前台窗口、模拟按键");
                if ui.button("打开设置").clicked() {
                    self.pending_open = Some(PendingOpen::Accessibility);
                }
            });
        } else {
            ui.label("✓ 辅助功能已授权");
        }

        if !self.status.input_monitoring {
            ui.horizontal(|ui| {
                ui.label("• 输入监控 — 注册全局热键");
                if ui.button("打开设置").clicked() {
                    self.pending_open = Some(PendingOpen::InputMonitoring);
                }
            });
        } else {
            ui.label("✓ 输入监控已授权");
        }

        ui.add_space(8.0);
        if let Some(path) = executable_path() {
            ui.label("请在系统设置中授权以下程序：");
            ui.monospace(path.display().to_string());
            ui.label("若使用 cargo run 开发，也可授权终端（Terminal / iTerm / Cursor）。");
        }

        if self.yielded_for_system_settings {
            ui.add_space(8.0);
            ui.colored_label(
                egui::Color32::from_rgb(220, 180, 80),
                "已打开系统设置。请在密码框中输入密码完成授权；授权后点「重新检测」。",
            );
        }

        ui.add_space(8.0);
        ui.label(
            "macOS 授权后需重启应用才能生效。\n\
             点击「重新检测」将自动重启并再次检查。",
        );

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("重新检测").clicked() {
                self.refresh();
            }
            if ui.button("稍后").clicked() {
                self.dismissed = true;
            }
        });
    }
}

fn demote_and_hide_root(ctx: &Context) {
    // ROOT 主面板默认 AlwaysOnTop，会挡住系统密码框；打开设置时降级并隐藏
    ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(WindowLevel::Normal));
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
}

impl Default for PermissionDialog {
    fn default() -> Self {
        Self::new()
    }
}
