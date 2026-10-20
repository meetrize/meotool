use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Context, ViewportId};

use crate::config::export::app_config_to_file;
use crate::config::model::{Action, ButtonConfig, MouseMacro};
use crate::config::color::{color_to_string, parse_color};
use crate::config::{
    AppConfig, ButtonScope, ContextMode, DisplayMode, GlobalHotkeys, MatchRule, PanelSettings,
    Profile, ThemeSettings,
};

use crate::platform::RecordMode;

use super::action_editor::{show_action_editor, ActionEditorState, MacroEditorRequest};
use super::button_editor::scope_label;
use super::hotkey_recorder::{hotkey_field, HotkeyRecorder, RecordingSlot};
use super::icon_picker::{IconPickTarget, IconPicker};
use super::icons::IconCache;
use super::overlay::settings_viewport;

const SETTINGS_VIEWPORT: &str = "meotool_settings";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    Global,
    Appearance,
    Hotkeys,
    Profiles,
}

#[derive(Debug)]
pub enum SettingsAction {
    None,
    /// 草稿变更：立即在内存生效，并由宿主防抖写盘
    Applied(AppConfig),
    /// 关闭设置窗口（不回滚；变更已生效）
    Closed,
}

pub struct SettingsEditor {
    pub open: bool,
    pub tab: SettingsTab,
    pub draft: AppConfig,
    pub selected_profile: usize,
    pub status_message: Option<String>,
    recorder: HotkeyRecorder,
    centered: bool,
    last_live_fingerprint: u64,
    last_fingerprint_at: Option<Instant>,
    icon_picker: IconPicker,
    /// 宏录制请求：(profile_index, button_index, request)
    pub macro_request: Option<(usize, usize, MacroEditorRequest)>,
    record_mode: RecordMode,
    /// 设置页按钮动作编辑器缓存（避免每帧重建丢失 wizard 状态）
    pub(crate) button_action_cache: Vec<ActionEditorState>,
    button_action_cache_profile: Option<usize>,
}

impl Default for SettingsEditor {
    fn default() -> Self {
        Self {
            open: false,
            tab: SettingsTab::Global,
            draft: empty_app_config(),
            selected_profile: 0,
            status_message: None,
            recorder: HotkeyRecorder::default(),
            centered: false,
            last_live_fingerprint: 0,
            last_fingerprint_at: None,
            icon_picker: IconPicker::default(),
            macro_request: None,
            record_mode: RecordMode::Idle,
            button_action_cache: Vec::new(),
            button_action_cache_profile: None,
        }
    }
}

fn empty_app_config() -> AppConfig {
    AppConfig {
        global_panel: crate::config::GlobalPanelDefaults {
            settings: PanelSettings::default(),
            position: crate::config::model::Position { x: 100.0, y: 200.0 },
        },
        hotkeys: GlobalHotkeys::default(),
        gesture: crate::config::GestureConfig::default(),
        profiles: vec![],
        poll_interval_ms: 200,
        context_mode: crate::config::ContextMode::Profile,
    }
}

impl SettingsEditor {
    pub fn set_record_mode(&mut self, mode: RecordMode) {
        self.record_mode = mode;
    }

    pub fn apply_macro_to_button(
        &mut self,
        profile: usize,
        button: usize,
        macro_: &MouseMacro,
    ) {
        if self
            .draft
            .profiles
            .get(profile)
            .and_then(|p| p.buttons.get(button))
            .is_none()
        {
            return;
        }
        self.sync_button_action_cache(profile);
        let Some(state) = self.button_action_cache.get_mut(button) else {
            return;
        };
        state.append_macro_script(macro_);
        let action = state.to_action();
        if let Some(btn) = self
            .draft
            .profiles
            .get_mut(profile)
            .and_then(|p| p.buttons.get_mut(button))
        {
            btn.enabled = true;
            btn.action = action;
        }
        self.status_message = Some("录制已追加到动作脚本".into());
    }

    pub(crate) fn sync_button_action_cache(&mut self, profile_index: usize) {
        let Some(profile) = self.draft.profiles.get(profile_index) else {
            self.button_action_cache.clear();
            self.button_action_cache_profile = None;
            return;
        };
        if self.button_action_cache_profile != Some(profile_index)
            || self.button_action_cache.len() != profile.buttons.len()
        {
            self.button_action_cache = profile
                .buttons
                .iter()
                .map(|b| ActionEditorState::from_action(&b.action))
                .collect();
            self.button_action_cache_profile = Some(profile_index);
        }
    }

    pub fn open_with(&mut self, config: &AppConfig) {
        self.draft = config.clone();
        self.last_live_fingerprint = config_fingerprint(config);
        self.last_fingerprint_at = None;
        self.selected_profile = config.default_profile_index();
        self.tab = SettingsTab::Global;
        self.status_message = None;
        self.recorder.cancel();
        self.centered = false;
        self.button_action_cache.clear();
        self.button_action_cache_profile = None;
        self.open = true;
    }

    /// 仅图标选择器或宏录制需要满帧；设置窗本身降频刷新
    pub fn needs_continuous_repaint(&self) -> bool {
        self.icon_picker.open || self.record_mode != RecordMode::Idle
    }

    /// 设置窗口打开时建议的刷新间隔（非满帧）
    pub fn idle_repaint_interval(&self) -> Option<Duration> {
        if self.open && !self.needs_continuous_repaint() {
            Some(Duration::from_millis(120))
        } else {
            None
        }
    }

    /// 关闭后释放草稿，降低常驻配置克隆占用
    pub fn release_draft_memory(&mut self) {
        self.draft = empty_app_config();
        self.last_live_fingerprint = 0;
        self.last_fingerprint_at = None;
        self.status_message = None;
    }

    pub fn show(
        &mut self,
        ctx: &Context,
        _config_path: &Path,
        icon_dirs: &[PathBuf],
        icons: &mut IconCache,
    ) -> SettingsAction {
        self.macro_request = None;
        if !self.open {
            return SettingsAction::None;
        }

        let mut action = SettingsAction::None;
        let mut should_close = false;

        ctx.show_viewport_immediate(
            ViewportId::from_hash_of(SETTINGS_VIEWPORT),
            settings_viewport(),
            |ctx, _class| {
                if !self.centered {
                    if let Some(cmd) = egui::ViewportCommand::center_on_screen(ctx) {
                        ctx.send_viewport_cmd(cmd);
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    self.centered = true;
                }

                egui::CentralPanel::default().show(ctx, |ui| {
                    self.draw_body(ui, ctx, icons, &mut action, &mut should_close);
                });

                if ctx.input(|i| i.viewport().close_requested()) {
                    should_close = true;
                    action = SettingsAction::Closed;
                }
            },
        );

        if self.open && !should_close {
            let due = self
                .last_fingerprint_at
                .is_none_or(|t| t.elapsed() >= Duration::from_millis(150));
            if due {
                self.last_fingerprint_at = Some(Instant::now());
                let fingerprint = config_fingerprint(&self.draft);
                if fingerprint != self.last_live_fingerprint {
                    self.last_live_fingerprint = fingerprint;
                    if self.normalize_match_rules().is_ok() {
                        crate::config::normalize_app_config(&mut self.draft);
                        action = SettingsAction::Applied(self.draft.clone());
                        self.status_message = Some("已应用".into());
                    }
                }
            }
        }

        if should_close {
            self.open = false;
            self.recorder.cancel();
            self.centered = false;
            self.icon_picker.close(icons);
        }

        if let Some((target, icon_name)) = self.icon_picker.show(ctx, icon_dirs, icons) {
            self.apply_icon_selection(target, &icon_name);
        }

        action
    }

    fn apply_icon_selection(&mut self, target: IconPickTarget, icon_name: &str) {
        match target {
            IconPickTarget::ProfileButton { profile, button } => {
                if let Some(btn) = self
                    .draft
                    .profiles
                    .get_mut(profile)
                    .and_then(|p| p.buttons.get_mut(button))
                {
                    *btn.icon.get_or_insert_with(String::new) = icon_name.to_string();
                }
            }
            IconPickTarget::CollapsedIcon => {
                *self
                    .draft
                    .global_panel
                    .settings
                    .collapsed_icon
                    .get_or_insert_with(String::new) = icon_name.to_string();
            }
        }
    }

    fn draw_body(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &Context,
        icons: &mut IconCache,
        action: &mut SettingsAction,
        should_close: &mut bool,
    ) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, SettingsTab::Global, "全局");
            ui.selectable_value(&mut self.tab, SettingsTab::Appearance, "外观");
            ui.selectable_value(&mut self.tab, SettingsTab::Hotkeys, "快捷键");
            ui.selectable_value(&mut self.tab, SettingsTab::Profiles, "Profiles");
        });
        ui.separator();

        // 先预留底部操作栏，再让 ScrollArea 填满剩余高度，避免内容撑开窗口导致无法滚动
        egui::TopBottomPanel::bottom("settings_footer")
            .resizable(false)
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                ui.separator();
                if let Some(msg) = &self.status_message {
                    ui.colored_label(egui::Color32::LIGHT_GREEN, msg);
                }
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("关闭").clicked() {
                            *action = SettingsAction::Closed;
                            *should_close = true;
                        }
                    });
                });
            });

        egui::ScrollArea::vertical()
            .id_salt("settings_body")
            .auto_shrink([false, false])
            .show(ui, |ui| match self.tab {
                SettingsTab::Global => self.show_global_tab(ui, ctx, icons),
                SettingsTab::Appearance => self.show_appearance_tab(ui),
                SettingsTab::Hotkeys => self.show_hotkeys_tab(ui, ctx),
                SettingsTab::Profiles => self.show_profiles_tab(ui, ctx, icons),
            });
    }

    fn normalize_match_rules(&mut self) -> Result<(), String> {
        for profile in &mut self.draft.profiles {
            if let Some(rule) = &mut profile.match_rule {
                normalize_title_regex(rule)
                    .map_err(|e| format!("profile '{}': invalid title_regex: {e}", profile.name))?;
            }
            for (i, button) in profile.buttons.iter_mut().enumerate() {
                if let ButtonScope::WhenActive(rule) = &mut button.scope {
                    normalize_title_regex(rule).map_err(|e| {
                        format!(
                            "profile '{}' button #{}: invalid title_regex: {e}",
                            profile.name,
                            i + 1
                        )
                    })?;
                }
            }
        }
        Ok(())
    }

    fn show_global_tab(&mut self, ui: &mut egui::Ui, ctx: &Context, icons: &mut IconCache) {
        let panel = &mut self.draft.global_panel.settings;
        let position = &mut self.draft.global_panel.position;

        ui.heading("面板");
        ui.horizontal(|ui| {
            ui.label("行");
            let rows = panel.rows.get_or_insert(3);
            ui.add(egui::DragValue::new(rows).range(1..=9));
            ui.label("列");
            let cols = panel.cols.get_or_insert(3);
            ui.add(egui::DragValue::new(cols).range(1..=9));
        });

        ui.horizontal(|ui| {
            ui.label("显示模式");
            let mut mode = panel.display_mode.unwrap_or(DisplayMode::Text);
            egui::ComboBox::from_id_salt("global_display_mode")
                .selected_text(display_mode_label(mode))
                .show_ui(ui, |ui| {
                    for m in DISPLAY_MODE_OPTIONS {
                        ui.selectable_value(&mut mode, m, display_mode_label(m));
                    }
                });
            panel.display_mode = Some(mode);
        });

        ui.horizontal(|ui| {
            let collapsed = panel.collapsed.get_or_insert(false);
            ui.checkbox(collapsed, "折叠模式");
        });

        ui.horizontal(|ui| {
            ui.label("透明度");
            ui.add(
                egui::Slider::new(panel.opacity.get_or_insert(0.85), 0.2..=1.0).fixed_decimals(2),
            );
        });
        ui.horizontal(|ui| {
            ui.label("位置 X");
            ui.add(egui::DragValue::new(&mut position.x).speed(1.0));
            ui.label("Y");
            ui.add(egui::DragValue::new(&mut position.y).speed(1.0));
        });

        ui.horizontal(|ui| {
            ui.label("窗口轮询 (ms)");
            ui.add(egui::DragValue::new(&mut self.draft.poll_interval_ms).range(50..=2000));
        });

        ui.horizontal(|ui| {
            ui.label("上下文模式");
            egui::ComboBox::from_id_salt("global_context_mode")
                .selected_text(context_mode_label(self.draft.context_mode))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.draft.context_mode,
                        ContextMode::Profile,
                        context_mode_label(ContextMode::Profile),
                    );
                    ui.selectable_value(
                        &mut self.draft.context_mode,
                        ContextMode::ButtonFilter,
                        context_mode_label(ContextMode::ButtonFilter),
                    );
                });
        });
        if self.draft.context_mode == ContextMode::ButtonFilter {
            ui.label(
                egui::RichText::new(
                    "按钮过滤：仅使用 default Profile 的布局与按钮；按钮 scope 才生效。",
                )
                .weak()
                .small(),
            );
        }

        let mut pick_collapsed = false;
        if panel.collapsed.unwrap_or(false) {
            ui.horizontal(|ui| {
                ui.label("折叠图标");
                {
                    let collapsed_icon = panel.collapsed_icon.get_or_insert_with(String::new);
                    pick_collapsed = icon_field(
                        ui,
                        ctx,
                        icons,
                        "collapsed_icon",
                        collapsed_icon,
                        IconPickTarget::CollapsedIcon,
                    );
                }
            });
        }
        if pick_collapsed {
            self.icon_picker.open_for(IconPickTarget::CollapsedIcon);
        }

        ui.add_space(12.0);
        ui.heading("鼠标手势");
        let gesture = &mut self.draft.gesture;
        ui.checkbox(&mut gesture.enabled, "启用右键拖动唤起面板");
        ui.checkbox(&mut gesture.middle_click_enabled, "启用中键点击唤起面板");
        ui.checkbox(
            &mut gesture.global_priority,
            "全局优先（后台可唤起，手势时抢占鼠标事件）",
        );
        ui.add_enabled_ui(gesture.enabled || gesture.middle_click_enabled, |ui| {
            if gesture.enabled {
                ui.horizontal(|ui| {
                    ui.label("拖动阈值 (px)");
                    ui.add(
                        egui::DragValue::new(&mut gesture.drag_threshold_px)
                            .range(2.0..=64.0)
                            .speed(0.5),
                    );
                });
            }
            ui.checkbox(&mut gesture.dismiss_on_outside_click, "点击面板外自动收起");
            ui.checkbox(&mut gesture.hide_after_button, "鼠标移出后自动收起");
        });
        if gesture.global_priority {
            ui.label(
                egui::RichText::new(
                    "需已授予「输入监控」与「辅助功能」权限；开启后手势将优先于其他应用。",
                )
                .weak()
                .small(),
            );
        }
    }

    fn show_appearance_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("全局外观");
        ui.label("以下设置为全局默认，可在 Profile 的「外观覆盖」中单独调整。");
        ui.add_space(4.0);
        show_button_size_editor(
            ui,
            "button_size_global",
            &mut self.draft.global_panel.settings.button_width,
            &mut self.draft.global_panel.settings.button_height,
            (64.0, 64.0),
            false,
        );
        ui.horizontal(|ui| {
            ui.label("左右间距");
            ui.add(
                egui::Slider::new(
                    self.draft
                        .global_panel
                        .settings
                        .gap_x
                        .get_or_insert(4.0),
                    0.0..=32.0,
                )
                .integer(),
            );
        });
        ui.horizontal(|ui| {
            ui.label("上下间距");
            ui.add(
                egui::Slider::new(
                    self.draft
                        .global_panel
                        .settings
                        .gap_y
                        .get_or_insert(4.0),
                    0.0..=32.0,
                )
                .integer(),
            );
        });
        show_theme_settings_editor(
            ui,
            "theme_global",
            &mut self.draft.global_panel.settings.theme,
        );

        ui.add_space(12.0);
        ui.heading("Profile 外观覆盖");
        let profile_index = self.selected_profile;
        let profile_name = self
            .draft
            .profiles
            .get(profile_index)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label("Profile");
            egui::ComboBox::from_id_salt("appearance_profile_select")
                .selected_text(&profile_name)
                .show_ui(ui, |ui| {
                    for (i, p) in self.draft.profiles.iter().enumerate() {
                        ui.selectable_value(&mut self.selected_profile, i, &p.name);
                    }
                });
        });
        if let Some(profile) = self.draft.profiles.get_mut(self.selected_profile) {
            if ui.button("清除 Profile 外观覆盖").clicked() {
                profile.panel.theme = ThemeSettings::default();
                profile.panel.button_width = None;
                profile.panel.button_height = None;
            }
            let inherited = (
                self.draft
                    .global_panel
                    .settings
                    .button_width
                    .unwrap_or(64.0),
                self.draft
                    .global_panel
                    .settings
                    .button_height
                    .unwrap_or(64.0),
            );
            show_button_size_editor(
                ui,
                &format!("button_size_profile_{profile_index}"),
                &mut profile.panel.button_width,
                &mut profile.panel.button_height,
                inherited,
                true,
            );
            show_theme_settings_editor(
                ui,
                &format!("theme_profile_{profile_index}"),
                &mut profile.panel.theme,
            );
        } else {
            ui.label("无 Profile");
        }
    }

    fn show_hotkeys_tab(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        ui.label("点击按钮后按下新的快捷键组合");
        ui.add_space(4.0);

        let hotkeys = &mut self.draft.hotkeys;
        hotkey_field(
            ui,
            ctx,
            "显示/隐藏面板",
            &mut hotkeys.toggle_visibility,
            &mut self.recorder,
            RecordingSlot::ToggleVisibility,
        );
        hotkey_field(
            ui,
            ctx,
            "重载配置",
            &mut hotkeys.reload_config,
            &mut self.recorder,
            RecordingSlot::ReloadConfig,
        );
        hotkey_field(
            ui,
            ctx,
            "退出",
            &mut hotkeys.quit,
            &mut self.recorder,
            RecordingSlot::Quit,
        );
        hotkey_field(
            ui,
            ctx,
            "打开设置",
            &mut hotkeys.open_settings,
            &mut self.recorder,
            RecordingSlot::OpenSettings,
        );
    }

    fn show_profiles_tab(&mut self, ui: &mut egui::Ui, ctx: &Context, icons: &mut IconCache) {
        let button_filter = self.draft.context_mode == ContextMode::ButtonFilter;
        if button_filter {
            ui.label(
                egui::RichText::new(
                    "当前为「按钮过滤」模式：请在 default Profile 中管理全部按钮与布局。",
                )
                .weak()
                .small(),
            );
            ui.add_space(4.0);
        }

        ui.horizontal(|ui| {
            ui.label("Profile");
            egui::ComboBox::from_id_salt("profile_select")
                .selected_text(
                    self.draft
                        .profiles
                        .get(self.selected_profile)
                        .map(|p| p.name.as_str())
                        .unwrap_or("(无)"),
                )
                .show_ui(ui, |ui| {
                    for (i, profile) in self.draft.profiles.iter().enumerate() {
                        ui.selectable_value(&mut self.selected_profile, i, &profile.name);
                    }
                });
            if ui.button("新建").clicked() {
                let name = format!("profile_{}", self.draft.profiles.len());
                self.draft.profiles.push(Profile {
                    name,
                    match_rule: None,
                    panel: PanelSettings::default(),
                    buttons: vec![],
                });
                self.selected_profile = self.draft.profiles.len() - 1;
            }
            if ui
                .add_enabled(
                    self.draft
                        .profiles
                        .get(self.selected_profile)
                        .is_some_and(|p| p.name != "default"),
                    egui::Button::new("删除"),
                )
                .clicked()
            {
                self.draft.profiles.remove(self.selected_profile);
                self.selected_profile = self
                    .selected_profile
                    .min(self.draft.profiles.len().saturating_sub(1));
            }
        });

        let profile_index = self.selected_profile;
        self.sync_button_action_cache(profile_index);
        let Some(profile) = self.draft.profiles.get_mut(profile_index) else {
            ui.label("无 Profile");
            return;
        };

        let is_default = profile.name == "default";
        let match_rules_locked = button_filter && !is_default;

        ui.horizontal(|ui| {
            ui.label("名称");
            ui.text_edit_singleline(&mut profile.name);
        });

        ui.collapsing("匹配规则", |ui| {
            if match_rules_locked {
                ui.label(
                    egui::RichText::new(
                        "按钮过滤模式下，非 default Profile 的匹配规则不参与运行时；请将按钮合并到 default 并配置 scope。",
                    )
                    .weak()
                    .small(),
                );
            }
            ui.add_enabled_ui(!match_rules_locked, |ui| {
                let rule = profile.match_rule.get_or_insert_with(|| MatchRule {
                    bundle_id: None,
                    app_name: None,
                    title_regex: None,
                    title_regex_text: None,
                });
                ui.horizontal(|ui| {
                    ui.label("Bundle ID");
                    let bundle = rule.bundle_id.get_or_insert_with(String::new);
                    ui.text_edit_singleline(bundle);
                });
                ui.horizontal(|ui| {
                    ui.label("App 名称");
                    let app = rule.app_name.get_or_insert_with(String::new);
                    ui.text_edit_singleline(app);
                });
                ui.horizontal(|ui| {
                    ui.label("标题正则");
                    let re = rule.title_regex_text.get_or_insert_with(String::new);
                    ui.text_edit_singleline(re);
                });
                if ui.button("清除匹配规则").clicked() {
                    profile.match_rule = None;
                }
            });
        });

        ui.collapsing("面板覆盖", |ui| {
            let panel = &mut profile.panel;
            ui.horizontal(|ui| {
                ui.label("行");
                let rows = panel.rows.get_or_insert(3);
                ui.add(egui::DragValue::new(rows).range(1..=9));
                ui.label("列");
                let cols = panel.cols.get_or_insert(3);
                ui.add(egui::DragValue::new(cols).range(1..=9));
            });
            if let Some(mode) = &mut panel.display_mode {
                egui::ComboBox::from_id_salt("profile_display_mode")
                    .selected_text(display_mode_label(*mode))
                    .show_ui(ui, |ui| {
                        for m in DISPLAY_MODE_OPTIONS {
                            ui.selectable_value(mode, m, display_mode_label(m));
                        }
                    });
            } else {
                ui.label("继承全局显示模式");
                if ui.button("设置显示模式").clicked() {
                    panel.display_mode = Some(DisplayMode::Text);
                }
            }
            if let Some(collapsed) = &mut panel.collapsed {
                ui.checkbox(collapsed, "折叠模式");
                if ui.button("恢复继承折叠设置").clicked() {
                    panel.collapsed = None;
                }
            } else {
                ui.label("继承全局折叠设置");
                if ui.button("设置折叠模式").clicked() {
                    panel.collapsed = Some(false);
                }
            }
        });

        ui.heading("按钮");
        let rows = profile
            .panel
            .rows
            .or(self.draft.global_panel.settings.rows)
            .unwrap_or(3);
        let cols = profile
            .panel
            .cols
            .or(self.draft.global_panel.settings.cols)
            .unwrap_or(3);
        let slots = (rows as usize) * (cols as usize);
        while profile.buttons.len() < slots {
            profile.buttons.push(ButtonConfig {
                label: String::new(),
                icon: None,
                action: Action::None,
                enabled: false,
                background_color: None,
                border_color: None,
                icon_color: None,
                scope: ButtonScope::Always,
                hotkey: None,
            });
        }
        profile.buttons.truncate(slots);

        let mut pending_pick: Option<IconPickTarget> = None;

        for (i, button) in profile.buttons.iter_mut().enumerate() {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("#{}", i + 1));
                    ui.checkbox(&mut button.enabled, "启用");
                    ui.label(
                        egui::RichText::new(scope_label(&button.scope))
                            .weak()
                            .small(),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("标签");
                    ui.text_edit_singleline(&mut button.label);
                });
                ui.horizontal(|ui| {
                    ui.label("图标");
                    let icon = button.icon.get_or_insert_with(String::new);
                    if icon_field(
                        ui,
                        ctx,
                        icons,
                        &format!("profile_{profile_index}_btn_{i}_icon"),
                        icon,
                        IconPickTarget::ProfileButton {
                            profile: profile_index,
                            button: i,
                        },
                    ) {
                        pending_pick = Some(IconPickTarget::ProfileButton {
                            profile: profile_index,
                            button: i,
                        });
                    }
                });

                let slot = RecordingSlot::Button {
                    profile: self.selected_profile,
                    button: i,
                };
                let mut macro_request = None;
                if let Some(action_state) = self.button_action_cache.get_mut(i) {
                    show_action_editor(
                        ui,
                        ctx,
                        &format!("settings_profile_{profile_index}_btn_{i}_action"),
                        action_state,
                        &mut self.recorder,
                        slot,
                        self.record_mode,
                        &mut macro_request,
                    );
                    button.action = action_state.apply_to_button(&mut button.enabled);
                }
                if let Some(req) = macro_request {
                    self.macro_request = Some((profile_index, i, req));
                }
            });
        }

        if let Some(target) = pending_pick {
            self.icon_picker.open_for(target);
        }
    }
}

const DISPLAY_MODE_OPTIONS: [DisplayMode; 3] =
    [DisplayMode::Text, DisplayMode::Icon, DisplayMode::IndexIcon];

fn display_mode_label(mode: DisplayMode) -> &'static str {
    match mode {
        DisplayMode::Text => "text",
        DisplayMode::Icon => "icon",
        DisplayMode::IndexIcon => "index_icon",
    }
}

fn context_mode_label(mode: ContextMode) -> &'static str {
    match mode {
        ContextMode::Profile => "Profile 切换（默认）",
        ContextMode::ButtonFilter => "按钮过滤",
    }
}

fn normalize_title_regex(rule: &mut MatchRule) -> Result<(), String> {
    if let Some(text) = rule.title_regex_text.as_deref().filter(|s| !s.is_empty()) {
        rule.title_regex = Some(regex::Regex::new(text).map_err(|e| e.to_string())?);
    } else {
        rule.title_regex = None;
        rule.title_regex_text = None;
    }
    Ok(())
}

/// 图标字段：预览 + 浏览 + 手动输入。返回 true 表示用户点击了「浏览」
fn icon_field(
    ui: &mut egui::Ui,
    ctx: &Context,
    icons: &mut IconCache,
    id: &str,
    value: &mut String,
    _target: IconPickTarget,
) -> bool {
    let mut browse = false;
    ui.horizontal(|ui| {
        let preview = 28.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(preview, preview), egui::Sense::hover());
        if !value.is_empty() {
            if let Some(texture) = icons.get(ctx, value) {
                ui.painter().image(
                    texture.id(),
                    rect.shrink(2.0),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
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

fn show_button_size_editor(
    ui: &mut egui::Ui,
    id_prefix: &str,
    width: &mut Option<f32>,
    height: &mut Option<f32>,
    inherited: (f32, f32),
    allow_inherit: bool,
) {
    egui::CollapsingHeader::new("按钮尺寸")
        .id_salt(format!("{id_prefix}_button_size"))
        .default_open(true)
        .show(ui, |ui| {
            if allow_inherit {
                let has_override = width.is_some() || height.is_some();
                if has_override {
                    if ui.button("恢复继承全局").clicked() {
                        *width = None;
                        *height = None;
                    }
                } else {
                    ui.label("当前继承全局设置；拖动下方数值可为该 Profile 单独覆盖。");
                }
            }
            ui.horizontal(|ui| {
                ui.label("宽度");
                if allow_inherit {
                    let mut w = width.unwrap_or(inherited.0);
                    if ui
                        .add(egui::DragValue::new(&mut w).range(32.0..=128.0))
                        .changed()
                    {
                        *width = Some(w);
                    }
                } else {
                    let w = width.get_or_insert(inherited.0);
                    ui.add(egui::DragValue::new(w).range(32.0..=128.0));
                }
                ui.label("高度");
                if allow_inherit {
                    let mut h = height.unwrap_or(inherited.1);
                    if ui
                        .add(egui::DragValue::new(&mut h).range(32.0..=128.0))
                        .changed()
                    {
                        *height = Some(h);
                    }
                } else {
                    let h = height.get_or_insert(inherited.1);
                    ui.add(egui::DragValue::new(h).range(32.0..=128.0));
                }
            });
        });
}

fn show_theme_settings_editor(ui: &mut egui::Ui, id_prefix: &str, theme: &mut ThemeSettings) {
    egui::CollapsingHeader::new("颜色")
        .id_salt(format!("{id_prefix}_colors"))
        .show(ui, |ui| {
            color_field(
                ui,
                id_prefix,
                "background",
                "面板背景色",
                &mut theme.background_color,
            );
            color_field(
                ui,
                id_prefix,
                "button",
                "按钮背景色",
                &mut theme.button_color,
            );
            color_field(
                ui,
                id_prefix,
                "letter_bg",
                "字母/序号背景色",
                &mut theme.letter_background_color,
            );
            color_field(
                ui,
                id_prefix,
                "letter_color",
                "字母/文字颜色",
                &mut theme.letter_color,
            );
        });

    egui::CollapsingHeader::new("文字")
        .id_salt(format!("{id_prefix}_text"))
        .show(ui, |ui| {
            ui.push_id(format!("{id_prefix}_letter_size"), |ui| {
                ui.horizontal(|ui| {
                    ui.label("字号");
                    let size = theme.letter_size.get_or_insert(11.0);
                    ui.add(egui::DragValue::new(size).range(6.0..=48.0).speed(0.5));
                });
            });
            ui.push_id(format!("{id_prefix}_letter_font"), |ui| {
                ui.horizontal(|ui| {
                    ui.label("字体文件路径");
                    let font = theme.letter_font.get_or_insert_with(String::new);
                    ui.text_edit_singleline(font);
                });
            });
            ui.label("留空则使用系统默认中文字体；可填 .ttf / .ttc 绝对路径。");
        });

    egui::CollapsingHeader::new("按钮边框")
        .id_salt(format!("{id_prefix}_button_border"))
        .show(ui, |ui| {
            ui.push_id(format!("{id_prefix}_button_border_enabled"), |ui| {
                let enabled = theme.button_border_enabled.get_or_insert(false);
                ui.checkbox(enabled, "显示按钮边框");
            });
            ui.push_id(format!("{id_prefix}_button_border_width"), |ui| {
                ui.horizontal(|ui| {
                    ui.label("粗细");
                    let width = theme.button_border_width.get_or_insert(1.0);
                    ui.add(egui::Slider::new(width, 0.0..=8.0).fixed_decimals(1));
                });
            });
            color_field(
                ui,
                id_prefix,
                "button_border_color",
                "颜色",
                &mut theme.button_border_color,
            );
        });

    ui.push_id(format!("{id_prefix}_button_corner_radius"), |ui| {
        ui.horizontal(|ui| {
            ui.label("按钮圆角");
            let radius = theme.button_corner_radius.get_or_insert(8.0);
            ui.add(egui::Slider::new(radius, 0.0..=32.0).fixed_decimals(0));
        });
        ui.label("为 0 时按钮为直角。");
    });
}

fn color_field(
    ui: &mut egui::Ui,
    id_prefix: &str,
    field: &str,
    label: &str,
    value: &mut Option<String>,
) {
    let widget_id = format!("{id_prefix}_{field}");
    ui.push_id(&widget_id, |ui| {
        let current = value
            .as_deref()
            .and_then(parse_color)
            .map(crate::ui::egui_color::to_egui)
            .unwrap_or(egui::Color32::TRANSPARENT);
        let mut rgba = current.to_srgba_unmultiplied();

        ui.horizontal(|ui| {
            ui.label(label);
            let mut changed = ui.color_edit_button_srgba_unmultiplied(&mut rgba).changed();
            if ui.button("透明").clicked() {
                rgba[3] = 0;
                changed = true;
            }
            if changed {
                let color =
                    egui::Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]);
                if color.a() == 0 {
                    *value = Some("transparent".into());
                } else {
                    *value = Some(color_to_string(crate::ui::egui_color::from_egui(color)));
                }
            }
            if let Some(text) = value.as_ref() {
                ui.label(text.as_str());
            }
        });
    });
}

fn config_fingerprint(config: &AppConfig) -> u64 {
    let text = toml::to_string(&app_config_to_file(config)).unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}
