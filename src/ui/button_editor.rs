use std::path::PathBuf;

use eframe::egui::{self, Context, ViewportId};

use crate::config::button_presets::{
    appearance_hint_text, apply_preset, preset_index_for, BUTTON_COLOR_PRESETS,
};
use crate::config::color::{color_to_string, parse_color};
use crate::config::model::{Action, ButtonConfig, ButtonScope, MatchRule, WindowInfo};

use crate::platform::RecordMode;

use super::action_editor::{show_action_editor, ActionEditorState, MacroEditorRequest};

use super::hotkey_recorder::{hotkey_field, HotkeyRecorder, RecordingSlot};
use super::icon_picker::{IconPickTarget, IconPicker};
use super::icons::{resolve_icon_tint, IconCache};
use super::overlay::{appearance_editor_viewport, script_editor_viewport};
use super::theme::Theme;

const BUTTON_SCRIPT_EDITOR_VIEWPORT: &str = "meotool_button_script_editor";
const BUTTON_APPEARANCE_EDITOR_VIEWPORT: &str = "meotool_button_appearance_editor";
const PREVIEW_SIZE: f32 = 64.0;
const PRESET_CELL: f32 = 52.0;
const PRESET_COLS: usize = 3;
const BUTTON_CORNER_RADIUS: f32 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppPickRequest {
    Start,
    /// 点选应用后写入动作脚本「激活应用」行
    StartForScriptActivate,
    /// 点选应用后写入动作脚本「打开应用」行
    StartForScriptOpenApp,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonEditorSurface {
    #[default]
    None,
    Script,
    Appearance,
}

#[derive(Debug)]
pub enum ButtonEditorAction {
    None,
    Saved { index: usize, button: ButtonConfig },
    Cancelled,
}

pub struct ButtonContextEditor {
    pub surface: ButtonEditorSurface,
    button_index: usize,
    profile_index: usize,
    profile_name: String,
    draft: ButtonConfig,
    pub action_editor: ActionEditorState,
    recorder: HotkeyRecorder,
    icon_picker: IconPicker,
    script_centered: bool,
    appearance_centered: bool,
    pub macro_request: Option<MacroEditorRequest>,
    /// 宏录制进行中时禁止 Esc 关闭编辑器
    pub macro_recording: bool,
    record_mode: RecordMode,
    pub app_pick_request: Option<AppPickRequest>,
    pub app_picking: bool,
    /// 点选结果写入动作脚本而非可见范围
    pub script_app_picking: bool,
    /// 点选结果写入「打开应用」脚本行
    pub script_open_app_picking: bool,
}

impl Default for ButtonContextEditor {
    fn default() -> Self {
        Self {
            surface: ButtonEditorSurface::None,
            button_index: 0,
            profile_index: 0,
            profile_name: String::new(),
            draft: ButtonConfig {
                label: String::new(),
                icon: None,
                action: Action::None,
                enabled: false,
                background_color: None,
                border_color: None,
                icon_color: None,
                scope: ButtonScope::Always,
                hotkey: None,
                show_in_tray_menu: false,
            },
            action_editor: ActionEditorState::default(),
            recorder: HotkeyRecorder::default(),
            icon_picker: IconPicker::default(),
            script_centered: false,
            appearance_centered: false,
            macro_request: None,
            macro_recording: false,
            record_mode: RecordMode::Idle,
            app_pick_request: None,
            app_picking: false,
            script_app_picking: false,
            script_open_app_picking: false,
        }
    }
}

impl ButtonContextEditor {
    pub fn is_open(&self) -> bool {
        self.surface != ButtonEditorSurface::None
    }

    pub fn open_script(
        &mut self,
        button_index: usize,
        profile_index: usize,
        profile_name: &str,
        button: &ButtonConfig,
    ) {
        self.ensure_session(button_index, profile_index, profile_name, button);
        self.surface = ButtonEditorSurface::Script;
        self.recorder.cancel();
        self.app_pick_request = None;
        self.app_picking = false;
        self.script_app_picking = false;
        self.script_open_app_picking = false;
    }

    pub fn open_appearance(
        &mut self,
        button_index: usize,
        profile_index: usize,
        profile_name: &str,
        button: &ButtonConfig,
    ) {
        self.ensure_session(button_index, profile_index, profile_name, button);
        self.surface = ButtonEditorSurface::Appearance;
        self.recorder.cancel();
        self.app_pick_request = None;
        self.app_picking = false;
        self.script_app_picking = false;
        self.script_open_app_picking = false;
    }

    fn ensure_session(
        &mut self,
        button_index: usize,
        profile_index: usize,
        profile_name: &str,
        button: &ButtonConfig,
    ) {
        let same_slot =
            self.button_index == button_index && self.profile_index == profile_index;
        self.button_index = button_index;
        self.profile_index = profile_index;
        self.profile_name = profile_name.to_string();
        if !same_slot {
            self.draft = button.clone();
            self.action_editor = ActionEditorState::from_action(&button.action);
            self.script_centered = false;
            self.appearance_centered = false;
        }
    }

    fn close(&mut self) {
        self.surface = ButtonEditorSurface::None;
        self.recorder.cancel();
        self.script_centered = false;
        self.appearance_centered = false;
    }

    /// 兼容旧调用：打开脚本编辑器。
    pub fn open_for(
        &mut self,
        button_index: usize,
        profile_index: usize,
        profile_name: &str,
        button: &ButtonConfig,
    ) {
        self.open_script(button_index, profile_index, profile_name, button);
    }

    pub fn needs_continuous_repaint(&self) -> bool {
        self.icon_picker.open
            || self.macro_recording
            || self.record_mode != RecordMode::Idle
            || self.app_picking
    }

    /// 编辑器打开时的降频刷新
    pub fn idle_repaint_interval(&self) -> Option<std::time::Duration> {
        if self.is_open() && !self.needs_continuous_repaint() {
            Some(std::time::Duration::from_millis(120))
        } else {
            None
        }
    }

    pub fn draft_enable_for_macro(&mut self) {
        self.draft.enabled = true;
    }

    pub fn apply_app_pick(&mut self, bundle_id: Option<String>, app_name: String) {
        if self.script_open_app_picking {
            self.script_open_app_picking = false;
            self.app_picking = false;
            let replace_all = self
                .action_editor
                .take_pending_open_app_pick()
                .unwrap_or(false);
            self.action_editor
                .insert_script_open_app(bundle_id, &app_name, replace_all);
            return;
        }
        if self.script_app_picking {
            self.script_app_picking = false;
            self.app_picking = false;
            match bundle_id.filter(|s| !s.trim().is_empty()) {
                Some(id) => self.action_editor.insert_script_activate(&id),
                None => {
                    self.action_editor.script_status =
                        "状态：未能识别 Bundle ID，请重试".into();
                }
            }
            let _ = app_name;
            return;
        }
        if !matches!(self.draft.scope, ButtonScope::WhenActive(_)) {
            self.draft.scope = ButtonScope::WhenActive(empty_match_rule());
        }
        if let ButtonScope::WhenActive(rule) = &mut self.draft.scope {
            rule.bundle_id = bundle_id.filter(|s| !s.is_empty());
            rule.app_name = if app_name.trim().is_empty() {
                None
            } else {
                Some(app_name)
            };
        }
        self.app_picking = false;
    }

    pub fn show(
        &mut self,
        ctx: &Context,
        icon_dirs: &[PathBuf],
        icons: &mut IconCache,
        theme: &Theme,
        record_mode: RecordMode,
        active_window: Option<&WindowInfo>,
    ) -> ButtonEditorAction {
        self.record_mode = record_mode;
        self.macro_recording = record_mode != RecordMode::Idle;
        self.macro_request = None;
        if self.surface == ButtonEditorSurface::Appearance {
            self.app_pick_request = None;
        }

        match self.surface {
            ButtonEditorSurface::None => ButtonEditorAction::None,
            ButtonEditorSurface::Script => {
                self.show_script(ctx, icon_dirs, icons, theme, record_mode, active_window)
            }
            ButtonEditorSurface::Appearance => {
                self.show_appearance(ctx, icon_dirs, icons, theme, active_window)
            }
        }
    }

    fn show_script(
        &mut self,
        ctx: &Context,
        icon_dirs: &[PathBuf],
        icons: &mut IconCache,
        theme: &Theme,
        record_mode: RecordMode,
        active_window: Option<&WindowInfo>,
    ) -> ButtonEditorAction {
        let mut action = ButtonEditorAction::None;
        let mut should_close = false;

        let title = format!(
            "编辑脚本 P{}#{}（{}）",
            self.profile_index,
            self.button_index,
            self.profile_name
        );

        ctx.show_viewport_immediate(
            ViewportId::from_hash_of(BUTTON_SCRIPT_EDITOR_VIEWPORT),
            script_editor_viewport(&title),
            |ctx, _class| {
                if !self.script_centered {
                    if let Some(cmd) = egui::ViewportCommand::center_on_screen(ctx) {
                        ctx.send_viewport_cmd(cmd);
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    self.script_centered = true;
                }

                egui::CentralPanel::default().show(ctx, |ui| {
                    self.draw_script_body(
                        ui,
                        ctx,
                        record_mode,
                        &mut action,
                        &mut should_close,
                    );
                });

                if ctx.input(|i| i.viewport().close_requested()) {
                    should_close = true;
                    action = ButtonEditorAction::Cancelled;
                }
            },
        );

        self.finish_show(ctx, icon_dirs, icons, &mut action, should_close)
    }

    fn show_appearance(
        &mut self,
        ctx: &Context,
        icon_dirs: &[PathBuf],
        icons: &mut IconCache,
        theme: &Theme,
        active_window: Option<&WindowInfo>,
    ) -> ButtonEditorAction {
        let mut action = ButtonEditorAction::None;
        let mut should_close = false;

        let title = format!(
            "按钮设置 P{}#{}（{}）",
            self.profile_index,
            self.button_index,
            self.profile_name
        );

        ctx.show_viewport_immediate(
            ViewportId::from_hash_of(BUTTON_APPEARANCE_EDITOR_VIEWPORT),
            appearance_editor_viewport(&title),
            |ctx, _class| {
                if !self.appearance_centered {
                    if let Some(cmd) = egui::ViewportCommand::center_on_screen(ctx) {
                        ctx.send_viewport_cmd(cmd);
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    self.appearance_centered = true;
                }

                egui::CentralPanel::default().show(ctx, |ui| {
                    self.draw_appearance_body(
                        ui,
                        ctx,
                        icons,
                        theme,
                        active_window,
                        &mut action,
                        &mut should_close,
                    );
                });

                if ctx.input(|i| i.viewport().close_requested()) {
                    should_close = true;
                    action = ButtonEditorAction::Cancelled;
                }
            },
        );

        self.finish_show(ctx, icon_dirs, icons, &mut action, should_close)
    }

    fn finish_show(
        &mut self,
        ctx: &Context,
        icon_dirs: &[PathBuf],
        icons: &mut IconCache,
        action: &mut ButtonEditorAction,
        should_close: bool,
    ) -> ButtonEditorAction {
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && self.is_open() && !self.macro_recording
        {
            if self.app_picking {
                self.app_pick_request = Some(AppPickRequest::Cancel);
            } else {
                *action = ButtonEditorAction::Cancelled;
            }
        }

        if should_close || matches!(*action, ButtonEditorAction::Cancelled) {
            self.close();
            self.icon_picker.close(icons);
            if self.app_picking {
                self.app_pick_request = Some(AppPickRequest::Cancel);
                self.app_picking = false;
            }
        }

        if let Some((target, icon_name)) = self.icon_picker.show(ctx, icon_dirs, icons) {
            if let IconPickTarget::ProfileButton { button, .. } = target {
                if button == self.button_index {
                    *self.draft.icon.get_or_insert_with(String::new) = icon_name;
                }
            }
        }

        std::mem::replace(action, ButtonEditorAction::None)
    }

    fn draw_script_body(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &Context,
        record_mode: RecordMode,
        action: &mut ButtonEditorAction,
        should_close: &mut bool,
    ) {
        let slot = RecordingSlot::Button {
            profile: self.profile_index,
            button: self.button_index,
        };
        let mut macro_request = None;
        show_action_editor(
            ui,
            ctx,
            &format!("panel_btn_{}_action", self.button_index),
            &mut self.action_editor,
            &mut self.recorder,
            slot,
            record_mode,
            &mut macro_request,
        );
        if macro_request.is_some() {
            self.macro_request = macro_request;
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("完成").clicked() {
                if let Err(err) = self.action_editor.validate_for_save() {
                    self.action_editor.script_status = format!("状态：{err}");
                    self.action_editor.refresh_script_status();
                } else {
                    self.draft.action =
                        self.action_editor.apply_to_button(&mut self.draft.enabled);
                    *action = ButtonEditorAction::Saved {
                        index: self.button_index,
                        button: self.draft.clone(),
                    };
                    *should_close = true;
                }
            }
            if ui.button("取消").clicked() {
                *action = ButtonEditorAction::Cancelled;
                *should_close = true;
            }
        });
    }

    fn draw_appearance_body(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &Context,
        icons: &mut IconCache,
        theme: &Theme,
        active_window: Option<&WindowInfo>,
        action: &mut ButtonEditorAction,
        should_close: &mut bool,
    ) {
        ui.checkbox(&mut self.draft.enabled, "启用");

        ui.horizontal(|ui| {
            ui.label("名称");
            ui.text_edit_singleline(&mut self.draft.label);
        });

        ui.horizontal(|ui| {
            ui.label("图标");
            let icon = self.draft.icon.get_or_insert_with(String::new);
            if icon_field(
                ui,
                ctx,
                icons,
                &format!("panel_btn_{}_icon", self.button_index),
                icon,
                self.draft.icon_color.as_deref(),
                theme.letter_color,
                self.draft.enabled,
            ) {
                self.icon_picker.open_for(IconPickTarget::ProfileButton {
                    profile: self.profile_index,
                    button: self.button_index,
                });
            }
        });

        {
            let mut hotkey = self.draft.hotkey.clone().unwrap_or_default();
            let slot = RecordingSlot::Button {
                profile: self.profile_index,
                button: self.button_index,
            };
            hotkey_field(
                ui,
                ctx,
                "快捷键",
                &mut hotkey,
                &mut self.recorder,
                slot,
            );
            if ui.small_button("清除快捷键").clicked() {
                hotkey.clear();
            }
            self.draft.hotkey = {
                let t = hotkey.trim();
                if t.is_empty() {
                    None
                } else {
                    Some(t.to_string())
                }
            };
        }

        show_appearance_section(
            ui,
            ctx,
            icons,
            self.button_index,
            &mut self.draft,
            theme,
        );

        show_scope_editor(
            ui,
            &mut self.draft.scope,
            active_window,
            self.app_picking,
            self.macro_recording,
            &mut self.app_pick_request,
        );

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("完成").clicked() {
                *action = ButtonEditorAction::Saved {
                    index: self.button_index,
                    button: self.draft.clone(),
                };
                *should_close = true;
            }
            if ui.button("取消").clicked() {
                *action = ButtonEditorAction::Cancelled;
                *should_close = true;
            }
        });
    }
}

/// 设置列表等处显示的 scope 短标签
pub fn scope_label(scope: &ButtonScope) -> String {
    match scope {
        ButtonScope::Always => "始终".into(),
        ButtonScope::WhenActive(rule) => {
            if let Some(name) = rule.app_name.as_deref().filter(|s| !s.is_empty()) {
                name.to_string()
            } else if let Some(id) = rule.bundle_id.as_deref().filter(|s| !s.is_empty()) {
                id.to_string()
            } else {
                "关联应用".into()
            }
        }
    }
}

fn empty_match_rule() -> MatchRule {
    MatchRule {
        bundle_id: None,
        app_name: None,
        title_regex: None,
        title_regex_text: None,
    }
}

fn show_appearance_section(
    ui: &mut egui::Ui,
    ctx: &Context,
    icons: &mut IconCache,
    button_index: usize,
    draft: &mut ButtonConfig,
    theme: &Theme,
) {
    ui.add_space(4.0);
    ui.heading("外观");

    ui.horizontal(|ui| {
        ui.label("预览");
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(PREVIEW_SIZE, PREVIEW_SIZE),
            egui::Sense::hover(),
        );
        let bg = draft
            .background_color
            .as_deref()
            .and_then(parse_color)
            .map(crate::ui::egui_color::to_egui)
            .unwrap_or(theme.button_bg);
        ui.painter()
            .rect_filled(rect, BUTTON_CORNER_RADIUS, bg);
        let icon_tint = resolve_icon_tint(
            draft.icon_color.as_deref(),
            theme.letter_color,
            draft.enabled,
        );
        if let Some(icon_name) = draft.icon.as_deref().filter(|s| !s.is_empty()) {
            if let Some(texture) = icons.get(ctx, icon_name) {
                let icon_rect = egui::Rect::from_center_size(
                    rect.center(),
                    egui::vec2(28.0, 28.0),
                );
                ui.painter().image(
                    texture.id(),
                    icon_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    icon_tint,
                );
            }
        }
        ui.vertical(|ui| {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new(appearance_hint_text(
                    draft.background_color.is_none(),
                    draft.icon_color.is_none(),
                ))
                .weak(),
            );
        });
    });

    ui.add_space(6.0);
    ui.label("快速配色");
    let selected = preset_index_for(
        draft.background_color.as_deref(),
        draft.icon_color.as_deref(),
    );
    ui.push_id(format!("panel_btn_{button_index}_presets"), |ui| {
        egui::Grid::new("button_color_presets")
            .num_columns(PRESET_COLS)
            .spacing([8.0, 8.0])
            .show(ui, |ui| {
                for (index, preset) in BUTTON_COLOR_PRESETS.iter().enumerate() {
                    let bg = parse_color(preset.background_color)
                        .map(crate::ui::egui_color::to_egui)
                        .unwrap_or(theme.button_bg);
                    let icon = parse_color(preset.icon_color)
                        .map(crate::ui::egui_color::to_egui)
                        .unwrap_or(theme.letter_color);
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(PRESET_CELL, PRESET_CELL),
                        egui::Sense::click(),
                    );
                    ui.painter()
                        .rect_filled(rect, BUTTON_CORNER_RADIUS, bg);
                    ui.painter().circle_filled(rect.center(), 7.0, icon);
                    if selected == Some(index) {
                        ui.painter().rect_stroke(
                            rect,
                            BUTTON_CORNER_RADIUS,
                            egui::Stroke::new(2.0, ui.visuals().selection.stroke.color),
                        );
                    }
                    if response.clicked() {
                        apply_preset(draft, index);
                    }
                    response.on_hover_text(format!(
                        "{}：{} / {}",
                        preset.name, preset.background_color, preset.icon_color
                    ));
                }
            });
    });

    ui.add_space(4.0);
    color_option_field(
        ui,
        &format!("panel_btn_{button_index}_bg"),
        "背景颜色",
        &mut draft.background_color,
        theme.button_bg,
    );
    color_option_field(
        ui,
        &format!("panel_btn_{button_index}_icon_color"),
        "图标颜色",
        &mut draft.icon_color,
        theme.letter_color,
    );
    egui::CollapsingHeader::new("边框")
        .default_open(false)
        .show(ui, |ui| {
            color_option_field(
                ui,
                &format!("panel_btn_{button_index}_border"),
                "边框颜色",
                &mut draft.border_color,
                theme.button_border_color,
            );
        });
}

fn show_scope_editor(
    ui: &mut egui::Ui,
    scope: &mut ButtonScope,
    active_window: Option<&WindowInfo>,
    picking: bool,
    macro_busy: bool,
    pick_request: &mut Option<AppPickRequest>,
) {
    ui.add_space(4.0);
    ui.heading("可见范围");

    let mut when_active = matches!(scope, ButtonScope::WhenActive(_));
    ui.radio_value(&mut when_active, false, "始终显示");
    ui.radio_value(&mut when_active, true, "仅当前应用匹配时显示");

    if when_active {
        if !matches!(scope, ButtonScope::WhenActive(_)) {
            *scope = ButtonScope::WhenActive(empty_match_rule());
        }
    } else {
        *scope = ButtonScope::Always;
    }

    if let ButtonScope::WhenActive(rule) = scope {
        ui.horizontal(|ui| {
            ui.label("Bundle ID");
            let bundle = rule.bundle_id.get_or_insert_with(String::new);
            ui.text_edit_singleline(bundle);
            let pick_label = if picking { "取消" } else { "点选" };
            let pick = ui
                .add_enabled(!macro_busy || picking, egui::Button::new(pick_label))
                .on_hover_text("点击目标应用窗口以填充 Bundle ID 和应用名")
                .on_disabled_hover_text("请先结束宏录制");
            if pick.clicked() {
                *pick_request = Some(if picking {
                    AppPickRequest::Cancel
                } else {
                    AppPickRequest::Start
                });
            }
        });
        if picking {
            ui.label("点击目标应用…（Esc 取消）");
        }
        ui.horizontal(|ui| {
            ui.label("App 名称");
            let app = rule.app_name.get_or_insert_with(String::new);
            ui.text_edit_singleline(app);
        });
        ui.horizontal(|ui| {
            let fill = ui
                .add_enabled(
                    !picking && active_window.is_some_and(|w| !w.is_own_process()),
                    egui::Button::new("使用当前前台应用填充"),
                )
                .on_disabled_hover_text("暂无前台应用信息");
            if fill.clicked() {
                if let Some(window) = active_window.filter(|w| !w.is_own_process()) {
                    fill_scope_from_window(rule, window);
                }
            }
        });
    }
}

fn fill_scope_from_window(rule: &mut MatchRule, window: &WindowInfo) {
    rule.bundle_id = window.bundle_id.clone().filter(|s| !s.is_empty());
    rule.app_name = if window.app_name.is_empty() {
        None
    } else {
        Some(window.app_name.clone())
    };
}

fn icon_field(
    ui: &mut egui::Ui,
    ctx: &Context,
    icons: &mut IconCache,
    id: &str,
    value: &mut String,
    icon_color: Option<&str>,
    fallback: egui::Color32,
    enabled: bool,
) -> bool {
    let mut browse = false;
    ui.horizontal(|ui| {
        let preview = 28.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(preview, preview), egui::Sense::hover());
        if !value.is_empty() {
            if let Some(texture) = icons.get(ctx, value) {
                let tint = resolve_icon_tint(icon_color, fallback, enabled);
                ui.painter().image(
                    texture.id(),
                    rect.shrink(2.0),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    tint,
                );
            }
        }
        ui.push_id(id, |ui| {
            if ui.button("浏览…").clicked() {
                browse = true;
            }
            ui.add(
                egui::TextEdit::singleline(value).desired_width(ui.available_width().max(120.0)),
            );
            if ui.button("清除").clicked() {
                value.clear();
            }
        });
    });
    browse
}

fn color_option_field(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    value: &mut Option<String>,
    fallback: egui::Color32,
) {
    ui.push_id(id, |ui| {
        let inherited = value.is_none();
        let display = value
            .as_deref()
            .and_then(parse_color)
            .map(crate::ui::egui_color::to_egui)
            .unwrap_or(fallback);
        let mut rgba = display.to_srgba_unmultiplied();

        ui.horizontal(|ui| {
            ui.label(label);
            let changed = ui.color_edit_button_srgba_unmultiplied(&mut rgba).changed();
            if !inherited && ui.button("继承全局").clicked() {
                *value = None;
            }
            if changed {
                let color =
                    egui::Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]);
                *value = Some(if color.a() == 0 {
                    "transparent".into()
                } else {
                    color_to_string(crate::ui::egui_color::from_egui(color))
                });
            }
            if inherited {
                ui.label(egui::RichText::new("(继承全局)").weak());
            } else if let Some(text) = value.as_ref() {
                ui.label(text.as_str());
            }
        });
    });
}
