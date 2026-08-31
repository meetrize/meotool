use std::path::PathBuf;
use std::time::Instant;

use crate::action::{log_action_error, ActionExecutor};
use crate::config::{
    save_panel_position, AppConfig, ButtonConfig, ContextMode, LoadedConfig, PanelConfig,
    ProfileResolver, WindowInfo,
};
use crate::ui::{
    CollapsedState, GridView, HintOverlay, IconCache, PermissionDialog, ProfileTransition,
    SettingsEditor, Theme,
};
use crate::window::fetch_active_window;

pub struct AppState {
    pub app_config: AppConfig,
    pub config_path: PathBuf,
    pub panel: PanelConfig,
    pub current_profile: String,
    pub visible: bool,
    pub selected_cell: Option<(u8, u8)>,
    pub collapsed: CollapsedState,
    pub executor: ActionExecutor,
    pub grid: GridView,
    pub icons: IconCache,
    pub flash_index: Option<(usize, egui::Color32)>,
    pub flash_until: Option<Instant>,
    pub transition: Option<ProfileTransition>,
    pub hint: Option<HintOverlay>,
    pub permission_dialog: PermissionDialog,
    pub last_active_window: Option<WindowInfo>,
    pub quit_requested: bool,
    pub request_focus: bool,
    pub config_reload_at: Option<Instant>,
    /// 由右键拖动手势唤起，启用点外收起 / 点按钮收起
    pub gesture_session: bool,
    /// 钉住后面板在失焦或点外时不自动隐藏
    pub pinned: bool,
    /// 渲染/触发用的已解析按钮（固定槽位）
    resolved_buttons: Vec<ButtonConfig>,
    /// 可见槽位指纹，用于检测 button_filter 下可见集变化
    last_visible_mask: u64,
}

impl AppState {
    pub fn new(
        app_config: AppConfig,
        config_path: PathBuf,
        panel: PanelConfig,
        profile_name: String,
        icon_dirs: Vec<std::path::PathBuf>,
    ) -> Self {
        let mut state = Self {
            app_config,
            config_path,
            panel,
            current_profile: profile_name,
            visible: true,
            selected_cell: None,
            collapsed: CollapsedState::default(),
            executor: ActionExecutor::new(),
            grid: GridView::default(),
            icons: IconCache::new(icon_dirs),
            flash_index: None,
            flash_until: None,
            transition: None,
            hint: None,
            permission_dialog: PermissionDialog::new(),
            last_active_window: fetch_active_window().filter(|w| !w.is_own_process()),
            quit_requested: false,
            request_focus: true,
            config_reload_at: None,
            gesture_session: false,
            pinned: false,
            resolved_buttons: Vec::new(),
            last_visible_mask: 0,
        };
        state.refresh_resolved_buttons();
        state.last_visible_mask = visible_slot_mask(&state.resolved_buttons);
        state
    }

    /// 网格绘制与触发使用的按钮列表
    pub fn display_buttons(&self) -> &[ButtonConfig] {
        &self.resolved_buttons
    }

    /// 槽位是否可选（方向键 / 可触发）
    pub fn slot_selectable(&self) -> Vec<bool> {
        self.resolved_buttons
            .iter()
            .map(|b| b.enabled && b.action.is_configured())
            .collect()
    }

    pub fn refresh_resolved_buttons(&mut self) {
        self.resolved_buttons = match self.app_config.context_mode {
            ContextMode::ButtonFilter => {
                let window = self
                    .last_active_window
                    .as_ref()
                    .filter(|w| !w.is_own_process());
                self.panel.resolve_visibility(window)
            }
            ContextMode::Profile => self.panel.padded_buttons(),
        };
    }

    fn ensure_default_panel(&mut self) {
        let idx = self.app_config.default_profile_index();
        let profile = &self.app_config.profiles[idx];
        self.panel = self.app_config.build_panel(profile);
        self.current_profile = profile.name.clone();
    }

    pub fn is_collapsed_mode(&self) -> bool {
        self.panel.collapsed
    }

    pub fn keyboard_active(&self) -> bool {
        !self.is_collapsed_mode() || self.collapsed.expanded
    }

    pub fn schedule_config_reload(&mut self, at: Instant) {
        self.config_reload_at = Some(at);
    }

    pub fn poll_config_reload(&mut self) -> bool {
        let Some(at) = self.config_reload_at else {
            return false;
        };
        if Instant::now() < at {
            return false;
        }
        self.config_reload_at = None;
        self.reload_config();
        true
    }

    pub fn show_at_gesture(&mut self, anchor: egui::Pos2) {
        if self.is_collapsed_mode() {
            self.collapsed.expanded = true;
        }
        let (w, h) = self.effective_viewport_size();
        let pos = egui::pos2(anchor.x - w * 0.5, anchor.y - h * 0.5);
        self.panel.position.x = pos.x;
        self.panel.position.y = pos.y;
        self.app_config.global_panel.position.x = pos.x;
        self.app_config.global_panel.position.y = pos.y;
        self.visible = true;
        self.gesture_session = true;
        self.request_focus = true;
        self.selected_cell = None;
        log::info!(
            "panel shown via gesture, anchor ({}, {}), top-left ({}, {})",
            anchor.x,
            anchor.y,
            pos.x,
            pos.y
        );
    }

    pub fn hide_panel(&mut self) {
        self.visible = false;
        self.gesture_session = false;
        self.pinned = false;
        if self.is_collapsed_mode() {
            self.collapsed = CollapsedState::default();
        }
        self.selected_cell = None;
        log::info!("panel hidden");
    }

    pub fn reload_config(&mut self) {
        match LoadedConfig::load(Some(&self.config_path)) {
            Ok(loaded) => {
                self.apply_config(loaded.app_config);
                log::info!("config reloaded from {}", self.config_path.display());
            }
            Err(err) => log::error!("reload config failed: {err}"),
        }
    }

    pub fn apply_config(&mut self, config: AppConfig) {
        self.app_config = config;
        match self.app_config.context_mode {
            ContextMode::ButtonFilter => {
                self.ensure_default_panel();
                self.collapsed = CollapsedState::default();
                self.selected_cell = None;
                if let Some(window) = fetch_active_window().filter(|w| !w.is_own_process()) {
                    self.last_active_window = Some(window);
                }
                self.refresh_resolved_buttons();
                self.last_visible_mask = visible_slot_mask(&self.resolved_buttons);
            }
            ContextMode::Profile => {
                if let Some(window) = fetch_active_window().filter(|w| !w.is_own_process()) {
                    let resolver = ProfileResolver::new(&self.app_config);
                    let (profile, panel) = resolver.resolve(&window);
                    if profile.name == self.current_profile {
                        self.panel = panel;
                        self.collapsed = CollapsedState::default();
                        self.selected_cell = None;
                    } else {
                        self.switch_profile(profile.name.clone(), panel, window.display_name());
                    }
                } else if let Some(index) = self.current_profile_index() {
                    let profile = &self.app_config.profiles[index];
                    self.panel = self.app_config.build_panel(profile);
                    self.collapsed = CollapsedState::default();
                    self.selected_cell = None;
                } else {
                    let idx = self.app_config.default_profile_index();
                    let profile = &self.app_config.profiles[idx];
                    self.force_profile(profile.name.clone(), self.app_config.build_panel(profile));
                }
                self.refresh_resolved_buttons();
                self.last_visible_mask = visible_slot_mask(&self.resolved_buttons);
            }
        }
    }

    pub fn request_quit(&mut self) {
        self.quit_requested = true;
    }

    pub fn save_position(&mut self, pos: egui::Pos2) {
        self.panel.position.x = pos.x;
        self.panel.position.y = pos.y;
        self.app_config.global_panel.position.x = pos.x;
        self.app_config.global_panel.position.y = pos.y;
        if let Err(err) = save_panel_position(&self.config_path, pos.x, pos.y) {
            log::error!("save position failed: {err}");
        }
    }

    pub fn on_active_window_changed(&mut self, window: &WindowInfo) {
        match self.app_config.context_mode {
            ContextMode::Profile => {
                let resolver = ProfileResolver::new(&self.app_config);
                let (profile, panel) = resolver.resolve(window);
                if profile.name == self.current_profile {
                    return;
                }
                self.switch_profile(profile.name.clone(), panel, window.display_name());
            }
            ContextMode::ButtonFilter => {
                self.ensure_default_panel();
                self.refresh_resolved_buttons();
                let mask = visible_slot_mask(&self.resolved_buttons);
                if mask != self.last_visible_mask {
                    self.last_visible_mask = mask;
                    self.hint = Some(HintOverlay::show(window.display_name()));
                    self.selected_cell = None;
                }
            }
        }
    }

    fn force_profile(&mut self, profile_name: String, panel: PanelConfig) {
        self.panel = panel;
        self.current_profile = profile_name;
        self.selected_cell = None;
        self.collapsed = CollapsedState::default();
        self.refresh_resolved_buttons();
        self.last_visible_mask = visible_slot_mask(&self.resolved_buttons);
    }

    fn switch_profile(&mut self, profile_name: String, panel: PanelConfig, hint: String) {
        log::info!(
            "profile switch: {} -> {}",
            self.current_profile,
            profile_name
        );
        self.transition = Some(ProfileTransition::new(panel, profile_name));
        self.hint = Some(HintOverlay::show(hint));
        self.flash_index = None;
        self.flash_until = None;
        self.selected_cell = None;
        self.collapsed = CollapsedState::default();
    }

    pub fn tick_transition(&mut self) -> Option<(f32, f32, f32)> {
        if self.transition.is_none() {
            return None;
        }

        let is_collapsed = self.is_collapsed_mode();

        let pending = {
            let transition = self.transition.as_mut().unwrap();
            transition.take_pending()
        };
        if let Some((panel, profile_name)) = pending {
            self.panel = panel;
            self.current_profile = profile_name;
            self.collapsed = CollapsedState::default();
            self.refresh_resolved_buttons();
            self.last_visible_mask = visible_slot_mask(&self.resolved_buttons);
        }

        let (alpha, finished) = {
            let transition = self.transition.as_mut().unwrap();
            (transition.content_alpha(), transition.is_finished())
        };

        let size = if is_collapsed {
            self.panel.viewport_size(self.collapsed.expanded)
        } else {
            self.panel.viewport_size(false)
        };

        if finished {
            self.transition = None;
        }

        Some((alpha, size.0, size.1))
    }

    pub fn effective_viewport_size(&self) -> (f32, f32) {
        self.panel.viewport_size(self.collapsed.expanded)
    }

    pub fn content_alpha(&self) -> f32 {
        self.transition
            .as_ref()
            .map(|t| t.content_alpha())
            .unwrap_or(1.0)
    }

    pub fn trigger_button(&mut self, index: usize, theme: &Theme) -> bool {
        let Some(button) = self.resolved_buttons.get(index).cloned() else {
            return false;
        };

        if !button.enabled || !button.action.is_configured() {
            return false;
        }

        if matches!(button.action, crate::config::Action::Script(_))
            && crate::action::is_script_running()
        {
            self.hint = Some(crate::ui::HintOverlay::show("脚本执行中…".into()));
            return false;
        }

        if let Some(window) = self
            .last_active_window
            .as_ref()
            .filter(|w| !w.is_own_process())
        {
            if let Err(err) = restore_target_window_focus(window) {
                log::warn!("restore focus before hotkey: {err}");
            }
        }

        let triggered = match self.executor.execute(&button.action) {
            Ok(()) => {
                self.flash_index = Some((index, theme.success_flash));
                self.flash_until = Some(Instant::now() + std::time::Duration::from_millis(100));
                if matches!(button.action, crate::config::Action::Script(_)) {
                    self.hint = Some(crate::ui::HintOverlay::show("脚本执行中…".into()));
                }
                true
            }
            Err(err) => {
                log_action_error(&err);
                self.flash_index = Some((index, theme.error_flash));
                self.flash_until = Some(Instant::now() + std::time::Duration::from_millis(200));
                false
            }
        };

        triggered
    }

    pub fn tick_flash(&mut self) {
        if let Some(until) = self.flash_until {
            if Instant::now() >= until {
                self.flash_index = None;
                self.flash_until = None;
            }
        }
    }

    pub fn needs_repaint(&self) -> bool {
        // 折叠模式不再无脑每帧重绘；悬停检测依赖主循环定时唤醒即可
        self.flash_until.is_some()
            || self.transition.is_some()
            || self.hint.as_ref().is_some_and(|h| h.is_active())
            || self.config_reload_at.is_some()
    }

    pub fn current_profile_index(&self) -> Option<usize> {
        self.app_config
            .profiles
            .iter()
            .position(|p| p.name == self.current_profile)
    }

    pub fn update_current_profile_button(&mut self, index: usize, button: ButtonConfig) -> bool {
        let Some(profile_index) = self.current_profile_index() else {
            return false;
        };
        let slots = (self.panel.rows as usize) * (self.panel.cols as usize);
        if index >= slots {
            return false;
        }
        {
            let profile = &mut self.app_config.profiles[profile_index];
            while profile.buttons.len() < slots {
                profile.buttons.push(crate::config::empty_slot_button());
            }
            profile.buttons[index] = button;
        }
        let profile = &self.app_config.profiles[profile_index];
        self.panel = self.app_config.build_panel(profile);
        self.refresh_resolved_buttons();
        self.last_visible_mask = visible_slot_mask(&self.resolved_buttons);
        true
    }

    pub fn open_settings(&mut self, editor: &mut SettingsEditor) {
        editor.open_with(&self.app_config);
    }

    pub fn set_collapsed_enabled(&mut self, collapsed: bool) -> bool {
        let Some(profile_index) = self.current_profile_index() else {
            return false;
        };
        self.app_config.profiles[profile_index].panel.collapsed = Some(collapsed);
        if collapsed {
            ensure_collapsed_icon(&mut self.app_config);
        }
        let profile = &self.app_config.profiles[profile_index];
        self.panel = self.app_config.build_panel(profile);
        self.collapsed = CollapsedState::default();
        self.refresh_resolved_buttons();
        self.last_visible_mask = visible_slot_mask(&self.resolved_buttons);
        true
    }
}

fn visible_slot_mask(buttons: &[ButtonConfig]) -> u64 {
    let mut mask = 0u64;
    for (i, button) in buttons.iter().enumerate().take(64) {
        let empty = button.label.is_empty()
            && !button.enabled
            && matches!(button.action, crate::config::Action::None);
        if !empty {
            mask |= 1u64 << i;
        }
    }
    mask
}

fn ensure_collapsed_icon(app_config: &mut crate::config::AppConfig) {
    let global_empty = app_config
        .global_panel
        .settings
        .collapsed_icon
        .as_ref()
        .map_or(true, |s| s.trim().is_empty());
    if global_empty {
        app_config.global_panel.settings.collapsed_icon =
            Some(crate::config::PanelConfig::DEFAULT_COLLAPSED_ICON.into());
    }
}

fn restore_target_window_focus(window: &WindowInfo) -> Result<(), String> {
    use std::time::{Duration, Instant};

    if let Some(active) = fetch_active_window() {
        if active.process_id == window.process_id {
            return Ok(());
        }
    }

    crate::platform::activate_app(window)?;

    let deadline = Instant::now() + Duration::from_millis(80);
    while Instant::now() < deadline {
        if let Some(active) = fetch_active_window() {
            if active.process_id == window.process_id {
                return Ok(());
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    Ok(())
}
