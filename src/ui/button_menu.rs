use eframe::egui::{self, Context, Pos2, Ui};
use std::time::{Duration, Instant};

use crate::config::ButtonConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherMenuAction {
    TogglePin,
    ExpandToFull,
    Settings,
    Quit,
}

pub struct LauncherContextMenu {
    open: bool,
    open_pos: Option<Pos2>,
    opened_this_frame: bool,
    unfocus_grace_until: Option<Instant>,
}

impl Default for LauncherContextMenu {
    fn default() -> Self {
        Self {
            open: false,
            open_pos: None,
            opened_this_frame: false,
            unfocus_grace_until: None,
        }
    }
}

impl LauncherContextMenu {
    pub fn open(&mut self, pos: Pos2) {
        self.open = true;
        self.open_pos = Some(pos);
        self.opened_this_frame = true;
        self.unfocus_grace_until = Some(Instant::now() + Duration::from_millis(200));
    }

    pub fn close(&mut self) {
        self.open = false;
        self.open_pos = None;
        self.opened_this_frame = false;
        self.unfocus_grace_until = None;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn on_secondary_click(&mut self, response: &egui::Response) {
        let pos = response
            .interact_pointer_pos()
            .unwrap_or_else(|| response.rect.center());
        self.open(pos);
        response.ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    pub fn show(&mut self, ctx: &Context, pinned: bool) -> Option<LauncherMenuAction> {
        if !self.open {
            return None;
        }
        let pos = self.open_pos.unwrap_or(Pos2::ZERO);

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.close();
            return None;
        }

        let skip_unfocus_dismiss = self.opened_this_frame
            || self
                .unfocus_grace_until
                .is_some_and(|until| Instant::now() < until);
        if !skip_unfocus_dismiss && !ctx.input(|i| i.focused) {
            self.close();
            return None;
        }

        let mut action = None;
        let menu_rect = egui::Area::new(egui::Id::new("launcher_context_menu"))
            .fixed_pos(pos)
            .order(egui::Order::Foreground)
            .interactable(true)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(140.0);
                    draw_launcher_menu_items(ui, pinned, &mut action);
                })
            })
            .response
            .rect;

        let skip_dismiss = self.opened_this_frame;
        self.opened_this_frame = false;

        if !skip_dismiss && should_dismiss_menu(ctx, menu_rect) {
            self.close();
            return None;
        }

        if let Some(chosen) = action {
            self.close();
            return Some(chosen);
        }

        None
    }
}

fn draw_launcher_menu_items(ui: &mut Ui, pinned: bool, action: &mut Option<LauncherMenuAction>) {
    if pinned {
        if ui.button("取消钉住").clicked() {
            *action = Some(LauncherMenuAction::TogglePin);
        }
    } else if ui.button("钉住面板").clicked() {
        *action = Some(LauncherMenuAction::TogglePin);
    }

    if ui.button("恢复完整模式").clicked() {
        *action = Some(LauncherMenuAction::ExpandToFull);
    }

    ui.separator();

    if ui.button("设置").clicked() {
        *action = Some(LauncherMenuAction::Settings);
    }

    ui.separator();

    if ui.button("退出").clicked() {
        *action = Some(LauncherMenuAction::Quit);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonMenuAction {
    Edit,
    EditAppearance,
    Enable,
    Disable,
    ClearAction,
    Delete,
    Settings,
    Quit,
}

pub struct ButtonContextMenu {
    open_for: Option<usize>,
    open_pos: Option<Pos2>,
    opened_this_frame: bool,
    /// 打开菜单后短暂忽略失焦关闭，等待 Focus 收回面板焦点。
    unfocus_grace_until: Option<Instant>,
}

impl Default for ButtonContextMenu {
    fn default() -> Self {
        Self {
            open_for: None,
            open_pos: None,
            opened_this_frame: false,
            unfocus_grace_until: None,
        }
    }
}

impl ButtonContextMenu {
    pub fn open(&mut self, index: usize, pos: Pos2) {
        self.open_for = Some(index);
        self.open_pos = Some(pos);
        self.opened_this_frame = true;
        self.unfocus_grace_until = Some(Instant::now() + Duration::from_millis(200));
    }

    pub fn close(&mut self) {
        self.open_for = None;
        self.open_pos = None;
        self.opened_this_frame = false;
        self.unfocus_grace_until = None;
    }

    pub fn is_open(&self) -> bool {
        self.open_for.is_some()
    }

    pub fn on_secondary_click(&mut self, index: usize, response: &egui::Response) {
        let pos = response
            .interact_pointer_pos()
            .unwrap_or_else(|| response.rect.center());
        self.open(index, pos);
        response.ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    pub fn show(
        &mut self,
        ctx: &Context,
        buttons: &[ButtonConfig],
    ) -> Option<(usize, ButtonMenuAction)> {
        let index = self.open_for?;
        let button = buttons.get(index)?;
        let pos = self.open_pos.unwrap_or(Pos2::ZERO);

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.close();
            return None;
        }

        let skip_unfocus_dismiss = self.opened_this_frame
            || self
                .unfocus_grace_until
                .is_some_and(|until| Instant::now() < until);
        if !skip_unfocus_dismiss && !ctx.input(|i| i.focused) {
            self.close();
            return None;
        }

        let mut action = None;
        let menu_rect = egui::Area::new(egui::Id::new("button_context_menu"))
            .fixed_pos(pos)
            .order(egui::Order::Foreground)
            .interactable(true)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(128.0);
                    draw_menu_items(ui, button, &mut action);
                })
            })
            .response
            .rect;

        let skip_dismiss = self.opened_this_frame;
        self.opened_this_frame = false;

        if !skip_dismiss && should_dismiss_menu(ctx, menu_rect) {
            self.close();
            return None;
        }

        if let Some(chosen) = action {
            self.close();
            return Some((index, chosen));
        }

        None
    }
}

fn should_dismiss_menu(ctx: &Context, menu_rect: egui::Rect) -> bool {
    ctx.input(|input| {
        let pointer_pos = input.pointer.interact_pos();
        for button in [egui::PointerButton::Primary, egui::PointerButton::Secondary] {
            if input.pointer.button_pressed(button) {
                return pointer_pos.is_none_or(|pos| !menu_rect.contains(pos));
            }
        }
        false
    })
}

pub fn is_button_configured(button: &ButtonConfig) -> bool {
    !button.label.is_empty() || button.icon.is_some() || button.action.is_configured()
}

fn draw_menu_items(ui: &mut Ui, button: &ButtonConfig, action: &mut Option<ButtonMenuAction>) {
    let configured = is_button_configured(button);
    let has_action = button.action.is_configured();

    if ui.button("编辑脚本…").clicked() {
        *action = Some(ButtonMenuAction::Edit);
    }
    if ui.button("编辑外观…").clicked() {
        *action = Some(ButtonMenuAction::EditAppearance);
    }

    if configured {
        ui.separator();

        if button.enabled {
            if ui.button("禁用").clicked() {
                *action = Some(ButtonMenuAction::Disable);
            }
        } else if has_action {
            if ui.button("启用").clicked() {
                *action = Some(ButtonMenuAction::Enable);
            }
        }

        if has_action {
            if ui.button("清空动作").clicked() {
                *action = Some(ButtonMenuAction::ClearAction);
            }
        }

        ui.separator();

        if ui.button("删除").clicked() {
            *action = Some(ButtonMenuAction::Delete);
        }
    }

    ui.separator();

    if ui.button("设置").clicked() {
        *action = Some(ButtonMenuAction::Settings);
    }

    ui.separator();

    if ui.button("退出").clicked() {
        *action = Some(ButtonMenuAction::Quit);
    }
}

pub fn empty_button_config() -> ButtonConfig {
    crate::config::empty_slot_button()
}
