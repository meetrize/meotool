//! 与 UI 框架无关的应用核心（阶段 1：原生路径真正执行命令）。

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::action::{log_action_error, ActionExecutor};
use crate::config::{
    AppConfig, ButtonConfig, ContextMode, LoadedConfig, PanelConfig, ProfileResolver, WindowInfo,
};
use crate::window::fetch_active_window;

/// 可由热键 / tray / 手势 / 原生按钮投递的命令。
#[derive(Debug, Clone, PartialEq)]
pub enum CoreCommand {
    ToggleVisibility,
    ReloadConfig,
    Quit,
    OpenSettings,
    OpenConfigFile,
    #[allow(dead_code)]
    OpenPermissions,
    #[allow(dead_code)]
    EditButton {
        profile: usize,
        button: usize,
    },
    TriggerButton(usize),
    ShowPanelAt {
        x: f32,
        y: f32,
    },
    HidePanel,
    ShowPanel,
    /// 前台窗口变化（由轮询推送）
    ActiveWindow(Option<WindowInfo>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreEffect {
    None,
    /// 面板几何或按钮集变化，需刷新原生视图
    RefreshPanel,
    /// 仅显隐变化
    VisibilityChanged,
    Quit,
    OpenSettingsUi,
    OpenPermissionsUi,
    OpenButtonEditor {
        profile: usize,
        button: usize,
    },
}

/// 领域核心：配置 / 可见性 / 动作执行。
pub struct AppCore {
    pub app_config: AppConfig,
    pub config_path: PathBuf,
    pub panel: PanelConfig,
    pub current_profile: String,
    pub visible: bool,
    pub gesture_session: bool,
    pub pinned: bool,
    pub quit_requested: bool,
    pub icon_dirs: Vec<PathBuf>,
    executor: ActionExecutor,
    last_active_window: Option<WindowInfo>,
    resolved_buttons: Vec<ButtonConfig>,
}

impl AppCore {
    pub fn from_loaded(loaded: &LoadedConfig) -> Self {
        let app_config = loaded.app_config.clone();
        let config_path = loaded.source_path.clone();
        let icon_dirs = loaded.icon_dirs.clone();
        let default_idx = app_config.default_profile_index();
        let mut current_profile = app_config.profiles[default_idx].name.clone();
        let mut panel = loaded.initial_panel();

        if app_config.context_mode == ContextMode::ButtonFilter {
            panel = app_config.build_panel(&app_config.profiles[default_idx]);
        } else if let Some(window) = fetch_active_window().filter(|w| !w.is_own_process()) {
            let (profile, resolved_panel) = loaded.resolver().resolve(&window);
            panel = resolved_panel;
            current_profile = profile.name.clone();
        }

        let mut core = Self {
            app_config,
            config_path,
            panel,
            current_profile,
            // 启动即显示：折叠模式为常驻图标，完整模式为按钮面板
            visible: true,
            gesture_session: false,
            pinned: false,
            quit_requested: false,
            icon_dirs,
            executor: ActionExecutor::new(),
            last_active_window: fetch_active_window().filter(|w| !w.is_own_process()),
            resolved_buttons: Vec::new(),
        };
        if core.is_collapsed_mode() {
            ensure_collapsed_icon(&mut core.app_config);
        }
        core.refresh_resolved_buttons();
        core
    }

    pub fn display_buttons(&self) -> &[ButtonConfig] {
        &self.resolved_buttons
    }

    pub fn viewport_size(&self) -> (f32, f32) {
        self.panel.viewport_size(false)
    }

    pub fn is_collapsed_mode(&self) -> bool {
        self.panel.collapsed
    }

    /// 切换折叠模式并写回当前配置档（调用方负责 save_app_config）。
    pub fn set_collapsed_enabled(&mut self, collapsed: bool) -> CoreEffect {
        let Some(profile_index) = self.current_profile_index() else {
            return CoreEffect::None;
        };
        self.app_config.profiles[profile_index].panel.collapsed = Some(collapsed);
        if collapsed {
            ensure_collapsed_icon(&mut self.app_config);
        }
        let profile = &self.app_config.profiles[profile_index];
        self.panel = self.app_config.build_panel(profile);
        self.refresh_resolved_buttons();
        CoreEffect::RefreshPanel
    }

    pub fn toggle_pinned(&mut self) {
        self.pinned = !self.pinned;
    }

    pub fn handle(&mut self, cmd: CoreCommand) -> CoreEffect {
        match cmd {
            CoreCommand::ToggleVisibility => {
                if self.visible {
                    self.hide_panel();
                    CoreEffect::VisibilityChanged
                } else {
                    self.show_panel();
                    CoreEffect::VisibilityChanged
                }
            }
            CoreCommand::ShowPanel => {
                self.show_panel();
                CoreEffect::VisibilityChanged
            }
            CoreCommand::HidePanel => {
                self.hide_panel();
                CoreEffect::VisibilityChanged
            }
            CoreCommand::ShowPanelAt { x, y } => {
                self.show_at(x, y);
                CoreEffect::RefreshPanel
            }
            CoreCommand::TriggerButton(index) => {
                self.trigger_button(index);
                if self.should_hide_after_button() {
                    self.hide_panel();
                    CoreEffect::VisibilityChanged
                } else {
                    CoreEffect::None
                }
            }
            CoreCommand::ReloadConfig => {
                self.reload_config();
                CoreEffect::RefreshPanel
            }
            CoreCommand::OpenSettings => CoreEffect::OpenSettingsUi,
            CoreCommand::OpenConfigFile => {
                open_config_in_editor(&self.config_path);
                CoreEffect::None
            }
            CoreCommand::OpenPermissions => CoreEffect::OpenPermissionsUi,
            CoreCommand::EditButton { profile, button } => {
                CoreEffect::OpenButtonEditor { profile, button }
            }
            CoreCommand::Quit => {
                self.quit_requested = true;
                CoreEffect::Quit
            }
            CoreCommand::ActiveWindow(window) => {
                if self.apply_active_window(window) {
                    CoreEffect::RefreshPanel
                } else {
                    CoreEffect::None
                }
            }
        }
    }

    fn show_panel(&mut self) {
        self.visible = true;
        self.gesture_session = false;
    }

    fn hide_panel(&mut self) {
        self.visible = false;
        self.gesture_session = false;
    }

    fn show_at(&mut self, x: f32, y: f32) {
        let (w, h) = self.viewport_size();
        self.panel.position.x = x - w / 2.0;
        self.panel.position.y = y - h / 2.0;
        self.app_config.global_panel.position.x = self.panel.position.x;
        self.app_config.global_panel.position.y = self.panel.position.y;
        self.visible = true;
        self.gesture_session = true;
    }

    fn should_hide_after_button(&self) -> bool {
        self.gesture_session
            && self.visible
            && !self.pinned
            && self.app_config.gesture.hide_after_button
    }

    fn trigger_button(&mut self, index: usize) {
        let Some(button) = self.resolved_buttons.get(index).cloned() else {
            return;
        };
        if !button.enabled || !button.action.is_configured() {
            return;
        }
        if matches!(button.action, crate::config::Action::Script(_))
            && crate::action::is_script_running()
        {
            log::info!("script already running, ignore button trigger");
            return;
        }
        if let Err(err) = self.executor.execute(&button.action) {
            log_action_error(&err);
        }
    }

    fn reload_config(&mut self) {
        match LoadedConfig::load(Some(&self.config_path)) {
            Ok(loaded) => {
                self.app_config = loaded.app_config.clone();
                self.icon_dirs = loaded.icon_dirs.clone();
                self.rebuild_panel_from_config();
                log::info!("config reloaded (native core)");
            }
            Err(err) => log::error!("reload config failed: {err}"),
        }
    }

    /// 设置保存后应用内存中的新配置（并写盘由调用方完成）。
    pub fn apply_config(&mut self, mut config: AppConfig) -> CoreEffect {
        crate::config::normalize_app_config(&mut config);
        self.app_config = config;
        self.rebuild_panel_from_config();
        CoreEffect::RefreshPanel
    }

    pub fn update_profile_button(
        &mut self,
        profile_idx: usize,
        button_idx: usize,
        button: ButtonConfig,
    ) -> CoreEffect {
        if let Some(profile) = self.app_config.profiles.get_mut(profile_idx) {
            if button_idx < profile.buttons.len() {
                profile.buttons[button_idx] = button;
            } else {
                profile
                    .buttons
                    .resize(button_idx + 1, crate::config::empty_slot_button());
                profile.buttons[button_idx] = button;
            }
        }
        self.rebuild_panel_from_config();
        CoreEffect::RefreshPanel
    }

    pub fn current_profile_index(&self) -> Option<usize> {
        self.app_config
            .profiles
            .iter()
            .position(|p| p.name == self.current_profile)
    }

    /// 按当前面板槽位更新当前配置档按钮（对齐 egui 右键菜单写回）。
    pub fn update_current_profile_button(
        &mut self,
        index: usize,
        button: ButtonConfig,
    ) -> CoreEffect {
        let Some(profile_index) = self.current_profile_index() else {
            return CoreEffect::None;
        };
        let slots = (self.panel.rows as usize) * (self.panel.cols as usize);
        if index >= slots {
            return CoreEffect::None;
        }
        {
            let profile = &mut self.app_config.profiles[profile_index];
            while profile.buttons.len() < slots {
                profile.buttons.push(crate::config::empty_slot_button());
            }
            profile.buttons[index] = button;
        }
        self.rebuild_panel_from_config();
        CoreEffect::RefreshPanel
    }

    fn rebuild_panel_from_config(&mut self) {
        let default_idx = self.app_config.default_profile_index();
        if self.app_config.context_mode == ContextMode::ButtonFilter {
            self.current_profile = self.app_config.profiles[default_idx].name.clone();
            self.panel = self
                .app_config
                .build_panel(&self.app_config.profiles[default_idx]);
        } else if let Some(window) = self.last_active_window.clone() {
            let resolver = ProfileResolver::new(&self.app_config);
            let (profile, panel) = resolver.resolve(&window);
            self.current_profile = profile.name.clone();
            self.panel = panel;
        } else {
            self.current_profile = self.app_config.profiles[default_idx].name.clone();
            self.panel = self
                .app_config
                .build_panel(&self.app_config.profiles[default_idx]);
        }
        self.refresh_resolved_buttons();
    }

    fn apply_active_window(&mut self, window: Option<WindowInfo>) -> bool {
        let changed = match (&self.last_active_window, &window) {
            (Some(a), Some(b)) => a.bundle_id != b.bundle_id || a.title != b.title,
            (None, None) => false,
            _ => true,
        };
        self.last_active_window = window.clone();
        if !changed {
            return false;
        }
        if self.app_config.context_mode == ContextMode::ButtonFilter {
            let before = self.resolved_buttons.clone();
            self.refresh_resolved_buttons();
            return resolved_buttons_differ(&before, &self.resolved_buttons);
        }
        if let Some(w) = window.filter(|w| !w.is_own_process()) {
            let resolver = ProfileResolver::new(&self.app_config);
            let (profile, panel) = resolver.resolve(&w);
            let profile_changed = profile.name != self.current_profile;
            self.current_profile = profile.name.clone();
            self.panel = panel;
            self.refresh_resolved_buttons();
            return profile_changed;
        }
        false
    }

    fn refresh_resolved_buttons(&mut self) {
        self.resolved_buttons = match self.app_config.context_mode {
            ContextMode::ButtonFilter => {
                let window = self.last_active_window.as_ref();
                self.panel.resolve_visibility(window)
            }
            _ => self.panel.padded_buttons(),
        };
    }
}

/// 面板指纹相关字段是否变化（与 `ui_native::panel::fingerprint` 对齐）。
fn resolved_buttons_differ(a: &[ButtonConfig], b: &[ButtonConfig]) -> bool {
    if a.len() != b.len() {
        return true;
    }
    a.iter()
        .zip(b.iter())
        .any(|(x, y)| panel_slot_differs(x, y))
}

fn panel_slot_differs(a: &ButtonConfig, b: &ButtonConfig) -> bool {
    a.enabled != b.enabled
        || a.label != b.label
        || a.icon != b.icon
        || a.action.is_configured() != b.action.is_configured()
        || a.background_color != b.background_color
        || a.border_color != b.border_color
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::app_config::{
        AppConfig, ContextMode, GestureConfig, GlobalHotkeys, GlobalPanelDefaults, PanelSettings,
        Profile,
    };
    use crate::config::model::{
        Action, ButtonConfig, ButtonScope, MatchRule, Position, WindowInfo,
    };

    fn sample_window(app: &str, title: &str) -> WindowInfo {
        WindowInfo {
            app_name: app.into(),
            bundle_id: Some("com.example.app".into()),
            title: title.into(),
            process_id: 42,
            process_path: PathBuf::from("/Applications/Example.app"),
            page_url: None,
        }
    }

    fn button_filter_core() -> AppCore {
        let mut panel_settings = PanelSettings::default();
        panel_settings.rows = Some(1);
        panel_settings.cols = Some(2);
        let app_config = AppConfig {
            global_panel: GlobalPanelDefaults {
                settings: PanelSettings::default(),
                position: Position { x: 0.0, y: 0.0 },
            },
            hotkeys: GlobalHotkeys::default(),
            gesture: GestureConfig::default(),
            context_mode: ContextMode::ButtonFilter,
            poll_interval_ms: 200,
            profiles: vec![Profile {
                name: "default".into(),
                match_rule: None,
                panel: panel_settings,
                buttons: vec![
                    ButtonConfig {
                        label: "A".into(),
                        icon: None,
                        action: Action::None,
                        enabled: true,
                        background_color: None,
                        border_color: None,
                        icon_color: None,
                        scope: ButtonScope::Always,
                    },
                    ButtonConfig {
                        label: "B".into(),
                        icon: None,
                        action: Action::None,
                        enabled: true,
                        background_color: None,
                        border_color: None,
                        icon_color: None,
                        scope: ButtonScope::WhenActive(MatchRule {
                            bundle_id: Some("com.example.app".into()),
                            app_name: None,
                            title_regex: None,
                            title_regex_text: None,
                        }),
                    },
                ],
            }],
        };
        let panel = app_config.build_panel(&app_config.profiles[0]);
        let mut core = AppCore {
            app_config,
            config_path: PathBuf::from("/tmp/meotool-test.toml"),
            panel,
            current_profile: "default".into(),
            visible: false,
            gesture_session: false,
            pinned: false,
            quit_requested: false,
            icon_dirs: vec![],
            executor: ActionExecutor::new(),
            last_active_window: None,
            resolved_buttons: Vec::new(),
        };
        core.refresh_resolved_buttons();
        core
    }

    #[test]
    fn button_filter_title_change_without_visibility_change_is_noop() {
        let mut core = button_filter_core();
        let w1 = sample_window("Example", "doc1.txt");
        assert!(core.handle(CoreCommand::ActiveWindow(Some(w1.clone()))) == CoreEffect::RefreshPanel);
        let w2 = sample_window("Example", "doc2.txt");
        assert!(core.handle(CoreCommand::ActiveWindow(Some(w2))) == CoreEffect::None);
    }

    #[test]
    fn button_filter_bundle_change_refreshes_panel() {
        let mut core = button_filter_core();
        let w1 = sample_window("Example", "doc");
        assert!(core.handle(CoreCommand::ActiveWindow(Some(w1))) == CoreEffect::RefreshPanel);
        let mut other = sample_window("Other", "doc");
        other.bundle_id = Some("com.other.app".into());
        assert!(core.handle(CoreCommand::ActiveWindow(Some(other))) == CoreEffect::RefreshPanel);
    }
}

fn open_config_in_editor(path: &Path) {
    log::info!("opening config: {}", path.display());
    if let Err(err) = Command::new("open").arg(path).spawn() {
        log::warn!("open config failed: {err}");
    }
}

fn ensure_collapsed_icon(app_config: &mut AppConfig) {
    let global_empty = app_config
        .global_panel
        .settings
        .collapsed_icon
        .as_ref()
        .map_or(true, |s| s.trim().is_empty());
    if global_empty {
        app_config.global_panel.settings.collapsed_icon =
            Some(PanelConfig::DEFAULT_COLLAPSED_ICON.into());
    }
}
