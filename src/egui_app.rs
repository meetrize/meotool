use std::env;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::action::{simplify_capture_once, simplify_events, SimplifyResult};
use crate::app::{AppState, MemoryMonitor, WakeHandle};
use crate::config::{
    debounce_duration, save_app_config, spawn_config_watcher, LoadedConfig,
    PanelConfig, WindowInfo,
};
use crate::input::{handle_panel_keyboard, HotkeyAction, HotkeyManager, KeyboardAction};
use crate::platform::{
    activate_self, check_accessibility_hint, check_permissions, AppClickPicker, MacroRecordOutcome,
    MacroRecorder, MouseGestureAction, MouseGestureMonitor, PickOutcome, RecordMode, TrayAction,
    TrayManager,
};
use crate::ui::overlay::{demote_main_panel, restore_main_panel_level};
use crate::ui::{
    empty_button_config, move_viewport, native_options, reload_theme_fonts, resize_viewport,
    set_viewport_visible, setup_theme_fonts, show_panel, show_panel_header,
    shrink_viewport_when_hidden, AppPickRequest, ButtonContextEditor, ButtonContextMenu,
    ButtonEditorAction, ButtonMenuAction, LauncherContextMenu, LauncherMenuAction,
    MacroEditorRequest, SettingsAction, SettingsEditor, Theme,
};
use crate::window::{
    fetch_active_window, header_app_tooltip, spawn_watcher, WindowWatcherControl,
};
use eframe::egui;
use notify::RecommendedWatcher;

pub fn run() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    check_accessibility_hint();

    let config_path = parse_config_arg();
    let loaded = load_config(config_path.as_deref());

    let app_config = loaded.app_config.clone();
    let config_file_path = loaded.source_path.clone();
    let poll_ms = app_config.poll_interval_ms;
    let icon_dirs = loaded.icon_dirs.clone();
    let hotkeys = app_config.hotkeys.clone();
    let gesture_config = app_config.gesture.clone();

    let default_idx = app_config.default_profile_index();
    let mut default_profile = app_config.profiles[default_idx].name.clone();
    let mut panel = loaded.initial_panel();

    if app_config.context_mode == crate::config::ContextMode::ButtonFilter {
        panel = app_config.build_panel(&app_config.profiles[default_idx]);
        log::info!(
            "initial context_mode=button_filter, profile={}",
            default_profile
        );
    } else if let Some(window) = fetch_active_window().filter(|w| !w.is_own_process()) {
        let (profile, resolved_panel) = loaded.resolver().resolve(&window);
        panel = resolved_panel;
        default_profile = profile.name.clone();
        log::info!(
            "initial profile: {} ({})",
            default_profile,
            window.display_name()
        );
    }

    let (width, height) = (2.0_f32, 2.0_f32); // 冷启动极小后备缓冲，首帧再扩到面板尺寸
    let options = native_options(width, height, panel.position.x, panel.position.y);

    let window_watch = WindowWatcherControl::new(poll_ms);
    let (window_tx, window_rx) = mpsc::channel::<WindowInfo>();
    let _watcher = spawn_watcher(window_tx, window_watch.clone());

    let (config_tx, config_rx) = mpsc::channel::<()>();
    let config_watcher = spawn_config_watcher(config_file_path.clone(), config_tx).ok();

    eframe::run_native(
        "MeoTool",
        options,
        Box::new(move |cc| {
            let letter_font = panel.ui_style.letter_font.as_deref();
            let font_source = setup_theme_fonts(&cc.egui_ctx, letter_font);
            log::info!("UI font source: {font_source:?}");

            let wake = WakeHandle::new();
            let egui_ctx = cc.egui_ctx.clone();
            wake.set_callback(move || egui_ctx.request_repaint());

            let hotkey_manager = HotkeyManager::register(&hotkeys, wake.clone())
                .expect("failed to register global hotkeys");
            let mouse_gesture = MouseGestureMonitor::new(&gesture_config, wake.clone())
                .map_err(|e| {
                    log::warn!("mouse gesture monitor unavailable: {e}");
                    e
                })
                .ok();
            // 宏录制 tap 懒启动：此处仅创建状态，首次录制再挂 EventTap
            let macro_recorder = MacroRecorder::new(wake.clone())
                .map_err(|e| {
                    log::warn!("macro recorder unavailable: {e}");
                    e
                })
                .ok();
            let app_picker = AppClickPicker::new(wake.clone())
                .map_err(|e| {
                    log::warn!("app click picker unavailable: {e}");
                    e
                })
                .ok();
            let tray_icon_name = panel
                .collapsed_icon
                .as_deref()
                .unwrap_or(PanelConfig::DEFAULT_COLLAPSED_ICON);
            let tray = TrayManager::install(wake, &icon_dirs, tray_icon_name)
                .map_err(|e| {
                    log::warn!("tray icon unavailable: {e}");
                    e
                })
                .ok();
            cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            Ok(Box::new(MeoToolApp::new(
                app_config,
                config_file_path,
                panel,
                default_profile,
                icon_dirs,
                window_rx,
                config_rx,
                config_watcher,
                hotkey_manager,
                mouse_gesture,
                macro_recorder,
                app_picker,
                tray,
                window_watch,
            )))
        }),
    )
}

fn initial_viewport_size(panel: &PanelConfig) -> (f32, f32) {
    panel.viewport_size(false)
}

fn parse_config_arg() -> Option<PathBuf> {
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--config" {
            return args.next().map(PathBuf::from);
        }
        if let Some(path) = arg.strip_prefix("--config=") {
            return Some(PathBuf::from(path));
        }
    }
    None
}

fn load_config(cli_path: Option<&std::path::Path>) -> LoadedConfig {
    LoadedConfig::load(cli_path).unwrap_or_else(|e| panic!("failed to load config: {e}"))
}

#[derive(Debug, Clone, Copy)]
enum MacroSessionTarget {
    ButtonEditor,
    Settings { profile: usize, button: usize },
}

struct MeoToolApp {
    state: AppState,
    theme: Theme,
    settings: SettingsEditor,
    button_editor: ButtonContextEditor,
    button_context_menu: ButtonContextMenu,
    launcher_context_menu: LauncherContextMenu,
    window_rx: mpsc::Receiver<WindowInfo>,
    config_rx: mpsc::Receiver<()>,
    _config_watcher: Option<RecommendedWatcher>,
    hotkey_manager: HotkeyManager,
    mouse_gesture: Option<MouseGestureMonitor>,
    macro_recorder: Option<MacroRecorder>,
    app_picker: Option<AppClickPicker>,
    macro_session: Option<MacroSessionTarget>,
    tray: Option<TrayManager>,
    last_size: (f32, f32),
    header_dragging: bool,
    last_letter_font: Option<String>,
    /// 延迟一帧再聚焦，避免 macOS 在 Visible(true) 同帧 Focus 崩溃
    pending_panel_focus: bool,
    memory: MemoryMonitor,
    last_tray_icon: String,
    last_numpad_slots: usize,
    last_numpad_armed: bool,
    /// 上一帧是否有独立对话框（设置/编辑/权限）打开
    dialogs_were_open: bool,
    /// 面板上次变为不可见的时间（用于隐藏态字体回收）
    hidden_since: Option<std::time::Instant>,
    window_watch: std::sync::Arc<WindowWatcherControl>,
    /// 设置变更防抖写盘
    pending_settings_save: Option<Instant>,
    /// 手势会话未钉住：鼠标离开后到期隐藏
    gesture_leave_hide_after: Option<Instant>,
}

impl MeoToolApp {
    fn new(
        app_config: crate::config::AppConfig,
        config_path: PathBuf,
        panel: crate::config::PanelConfig,
        profile_name: String,
        icon_dirs: Vec<PathBuf>,
        window_rx: mpsc::Receiver<WindowInfo>,
        config_rx: mpsc::Receiver<()>,
        config_watcher: Option<RecommendedWatcher>,
        hotkey_manager: HotkeyManager,
        mouse_gesture: Option<MouseGestureMonitor>,
        macro_recorder: Option<MacroRecorder>,
        app_picker: Option<AppClickPicker>,
        tray: Option<TrayManager>,
        window_watch: std::sync::Arc<WindowWatcherControl>,
    ) -> Self {
        let size = initial_viewport_size(&panel);
        let last_letter_font = panel.ui_style.letter_font.clone();
        let theme = Theme::from_ui_style(&panel.ui_style);
        let tray_icon_name = panel
            .collapsed_icon
            .clone()
            .unwrap_or_else(|| PanelConfig::DEFAULT_COLLAPSED_ICON.into());
        Self {
            state: AppState::new(app_config, config_path, panel, profile_name, icon_dirs),
            theme,
            settings: SettingsEditor::default(),
            button_editor: ButtonContextEditor::default(),
            button_context_menu: ButtonContextMenu::default(),
            launcher_context_menu: LauncherContextMenu::default(),
            window_rx,
            config_rx,
            _config_watcher: config_watcher,
            hotkey_manager,
            mouse_gesture,
            macro_recorder,
            app_picker,
            macro_session: None,
            tray,
            last_size: size,
            header_dragging: false,
            last_letter_font,
            pending_panel_focus: false,
            memory: MemoryMonitor::default(),
            last_tray_icon: tray_icon_name,
            last_numpad_slots: usize::MAX,
            last_numpad_armed: false,
            dialogs_were_open: false,
            hidden_since: None,
            window_watch,
            pending_settings_save: None,
            gesture_leave_hide_after: None,
        }
    }

    fn dialogs_open(&self) -> bool {
        self.settings.open
            || self.button_editor.is_open()
            || self.state.permission_dialog.should_show()
    }

    /// 独立对话框打开时隐藏/降级主面板；关闭后按原可见性恢复
    fn sync_main_around_dialogs(&mut self, ctx: &egui::Context) {
        let open = self.dialogs_open();
        if open {
            demote_main_panel(ctx);
        } else if self.dialogs_were_open {
            if self.state.visible {
                self.sync_viewport_size(ctx, self.state.effective_viewport_size());
                move_viewport(
                    ctx,
                    egui::pos2(self.state.panel.position.x, self.state.panel.position.y),
                );
                restore_main_panel_level(ctx, true);
            } else {
                set_viewport_visible(ctx, false);
                shrink_viewport_when_hidden(ctx);
                self.last_size = (1.0, 1.0);
            }
        }
        self.dialogs_were_open = open;
    }

    fn apply_panel_shown_viewport(&mut self, ctx: &egui::Context) {
        self.sync_viewport_size(ctx, self.state.effective_viewport_size());
        move_viewport(
            ctx,
            egui::pos2(self.state.panel.position.x, self.state.panel.position.y),
        );
        set_viewport_visible(ctx, true);
        if let Err(err) = activate_self() {
            log::warn!("activate self when showing panel: {err}");
        }
        self.pending_panel_focus = true;
        ctx.request_repaint();
    }

    fn show_main_panel(&mut self, ctx: &egui::Context) {
        if self.state.visible {
            return;
        }
        self.state.visible = true;
        self.hidden_since = None;
        self.state.gesture_session = false;
        self.apply_panel_shown_viewport(ctx);
        log::info!("panel shown");
    }

    fn hide_main_panel(&mut self, ctx: &egui::Context) {
        if !self.state.visible {
            return;
        }
        self.state.hide_panel();
        self.hidden_since = Some(std::time::Instant::now());
        self.button_context_menu.close();
        self.launcher_context_menu.close();
        self.pending_panel_focus = false;
        // 独立对话框打开时主窗已由 sync_main_around_dialogs 管理
        if self.dialogs_open() {
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(false));
        set_viewport_visible(ctx, false);
        shrink_viewport_when_hidden(ctx);
        self.last_size = (1.0, 1.0);
    }

    fn toggle_main_panel(&mut self, ctx: &egui::Context) {
        if self.state.visible {
            self.hide_main_panel(ctx);
        } else {
            self.show_main_panel(ctx);
        }
    }

    fn sync_theme_from_panel(&mut self, ctx: &egui::Context) {
        let font_path = self.state.panel.ui_style.letter_font.clone();
        if font_path != self.last_letter_font {
            setup_theme_fonts(ctx, font_path.as_deref());
            self.last_letter_font = font_path;
        }
        self.theme = Theme::from_ui_style(&self.state.panel.ui_style);
    }

    fn poll_window_events(&mut self) {
        self.window_watch
            .set_panel_visible(self.state.visible || self.dialogs_open());
        while let Ok(window) = self.window_rx.try_recv() {
            if window.is_own_process() {
                continue;
            }
            self.state.last_active_window = Some(window.clone());
            self.state.on_active_window_changed(&window);
        }
    }

    fn poll_config_events(&mut self) {
        while self.config_rx.try_recv().is_ok() {
            self.state
                .schedule_config_reload(Instant::now() + debounce_duration());
        }
    }

    fn sync_viewport_size(&mut self, ctx: &egui::Context, size: (f32, f32)) {
        if size != self.last_size {
            resize_viewport(ctx, size.0, size.1);
            self.last_size = size;
        }
    }

    fn poll_hotkeys(&mut self, ctx: &egui::Context) {
        while let Some(action) = self.hotkey_manager.poll() {
            match action {
                HotkeyAction::ToggleVisibility => {
                    self.toggle_main_panel(ctx);
                }
                HotkeyAction::ReloadConfig => {
                    self.state.reload_config();
                    self.sync_tray_icon();
                    self.sync_viewport_size(ctx, self.state.effective_viewport_size());
                }
                HotkeyAction::Quit => {
                    if self.state.visible {
                        self.state.request_quit();
                    }
                }
                HotkeyAction::OpenSettings => {
                    self.open_settings(ctx);
                }
                HotkeyAction::TriggerButton(index) => {
                    if self.state.visible {
                        self.trigger_panel_button(ctx, index);
                    }
                }
            }
        }
    }

    fn poll_tray(&mut self, ctx: &egui::Context) {
        let Some(tray) = &self.tray else {
            return;
        };
        match tray.poll() {
            Some(TrayAction::TogglePanel) => {
                self.toggle_main_panel(ctx);
            }
            Some(TrayAction::OpenSettings) => self.open_settings(ctx),
            Some(TrayAction::Quit) => self.state.request_quit(),
            None => {}
        }
    }

    fn open_settings(&mut self, ctx: &egui::Context) {
        self.state.open_settings(&mut self.settings);
        ctx.request_repaint();
    }

    fn finish_settings(&mut self, _ctx: &egui::Context) {
        self.settings.release_draft_memory();
    }

    fn handle_settings(&mut self, ctx: &egui::Context) {
        if !self.settings.open {
            return;
        }
        let path = self.state.config_path.clone();
        let icon_dirs: Vec<PathBuf> = self.state.icons.search_dirs().to_vec();
        let record_mode = self.macro_record_mode();
        self.settings.set_record_mode(record_mode);
        match self
            .settings
            .show(ctx, &path, &icon_dirs, &mut self.state.icons)
        {
            SettingsAction::Applied(config) => {
                self.apply_live_settings(ctx, config);
                self.pending_settings_save = Some(Instant::now() + Duration::from_millis(400));
            }
            SettingsAction::Closed => {
                self.flush_pending_settings_save(true);
                self.finish_settings(ctx);
            }
            SettingsAction::None => {}
        }
        if let Some((profile, button, req)) = self.settings.macro_request.take() {
            self.handle_macro_request(ctx, req, MacroSessionTarget::Settings { profile, button });
        }
    }

    fn apply_live_settings(&mut self, ctx: &egui::Context, config: crate::config::AppConfig) {
        self.state.apply_config(config);
        self.rebuild_theme(ctx);
        if let Err(err) = self.hotkey_manager.replace(&self.state.app_config.hotkeys) {
            log::error!("re-register hotkeys failed: {err}");
        }
        self.sync_tray_icon();
        // 对话框打开时主面板已隐藏，避免改尺寸引发闪动
        if !self.dialogs_open() {
            self.sync_viewport_size(ctx, self.state.effective_viewport_size());
        }
    }

    fn flush_pending_settings_save(&mut self, force: bool) {
        let due = self.pending_settings_save.is_some_and(|deadline| {
            force || Instant::now() >= deadline
        });
        if !due {
            return;
        }
        self.pending_settings_save = None;
        match save_app_config(&self.state.config_path, &self.state.app_config) {
            Ok(()) => {
                self.settings.status_message = Some("已保存".into());
            }
            Err(err) => log::error!("save settings failed: {err}"),
        }
    }

    fn sync_tray_icon(&mut self) {
        let Some(tray) = &mut self.tray else {
            return;
        };
        let icon_name = self
            .state
            .panel
            .collapsed_icon
            .as_deref()
            .unwrap_or(PanelConfig::DEFAULT_COLLAPSED_ICON);
        if icon_name == self.last_tray_icon {
            return;
        }
        tray.sync_icon(icon_name);
        self.last_tray_icon = icon_name.to_string();
    }

    fn sync_numpad_hotkeys(&mut self) {
        let slots = self.state.panel.slot_count();
        let armed = self.state.visible;
        if slots == self.last_numpad_slots && armed == self.last_numpad_armed {
            return;
        }
        if let Err(err) = self.hotkey_manager.set_numpad_triggers(slots, armed) {
            log::warn!("sync numpad hotkeys: {err}");
            return;
        }
        self.last_numpad_slots = slots;
        self.last_numpad_armed = armed;
    }

    fn maybe_reset_fonts(&mut self, ctx: &egui::Context) {
        let idle_hidden = !self.state.visible
            && !self.dialogs_open()
            && self
                .hidden_since
                .is_some_and(|t| t.elapsed() >= crate::app::memory::HIDDEN_FONT_RESET_AFTER);
        if !self.memory.should_reset_fonts_ex(ctx, idle_hidden) {
            return;
        }
        let font_path = self.state.panel.ui_style.letter_font.as_deref();
        reload_theme_fonts(ctx, font_path);
        self.state.icons.clear_all();
        self.memory.mark_font_reset();
        if idle_hidden {
            self.hidden_since = Some(std::time::Instant::now());
        }
        log::info!("font atlas reset #{}", self.memory.font_resets);
    }

    fn save_button_update(
        &mut self,
        index: usize,
        button: crate::config::ButtonConfig,
        hint: &str,
    ) {
        if self.state.update_current_profile_button(index, button) {
            match save_app_config(&self.state.config_path, &self.state.app_config) {
                Ok(()) => {
                    self.state.hint = Some(crate::ui::HintOverlay::show(hint.into()));
                }
                Err(err) => log::error!("save button config failed: {err}"),
            }
        }
    }

    fn handle_launcher_menu(&mut self, ctx: &egui::Context, action: LauncherMenuAction) {
        match action {
            LauncherMenuAction::TogglePin => self.toggle_pin(ctx),
            LauncherMenuAction::ExpandToFull => self.expand_to_full_mode(ctx),
            LauncherMenuAction::Settings => self.open_settings(ctx),
            LauncherMenuAction::Quit => self.state.request_quit(),
        }
    }

    fn set_collapsed_mode(&mut self, ctx: &egui::Context, collapsed: bool, hint: &str) {
        if !self.state.set_collapsed_enabled(collapsed) {
            return;
        }
        // 离开折叠桌宠时务必关闭穿透，否则完整模式窗口可能点不到
        ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(false));
        match save_app_config(&self.state.config_path, &self.state.app_config) {
            Ok(()) => {
                self.state.hint = Some(crate::ui::HintOverlay::show(hint.into()));
            }
            Err(err) => log::error!("save collapsed mode change failed: {err}"),
        }
        self.sync_viewport_size(ctx, self.state.effective_viewport_size());
        ctx.request_repaint();
    }

    fn expand_to_full_mode(&mut self, ctx: &egui::Context) {
        self.set_collapsed_mode(ctx, false, "已切换为完整模式");
    }

    fn collapse_to_fold_mode(&mut self, ctx: &egui::Context) {
        self.set_collapsed_mode(ctx, true, "已切换为折叠模式");
    }

    fn handle_button_menu(&mut self, ctx: &egui::Context, index: usize, action: ButtonMenuAction) {
        let buttons = self.state.panel.padded_buttons();
        let Some(mut button) = buttons.get(index).cloned() else {
            return;
        };

        match action {
            ButtonMenuAction::Edit => self.open_button_script_editor(ctx, index),
            ButtonMenuAction::EditAppearance => self.open_button_appearance_editor(ctx, index),
            ButtonMenuAction::Settings => self.open_settings(ctx),
            ButtonMenuAction::Quit => self.state.request_quit(),
            ButtonMenuAction::Enable => {
                button.enabled = true;
                self.save_button_update(index, button, "按钮已启用");
            }
            ButtonMenuAction::Disable => {
                button.enabled = false;
                self.save_button_update(index, button, "按钮已禁用");
            }
            ButtonMenuAction::ClearAction => {
                button.action = crate::config::Action::None;
                button.enabled = false;
                self.save_button_update(index, button, "按钮动作已清空");
            }
            ButtonMenuAction::Delete => {
                self.save_button_update(index, empty_button_config(), "按钮已删除");
            }
        }
    }

    fn handle_button_editor(&mut self, ctx: &egui::Context) {
        if !self.button_editor.is_open() {
            return;
        }
        let icon_dirs: Vec<PathBuf> = self.state.icons.search_dirs().to_vec();
        let record_mode = self.macro_record_mode();
        match self.button_editor.show(
            ctx,
            &icon_dirs,
            &mut self.state.icons,
            &self.theme,
            record_mode,
            self.state.last_active_window.as_ref(),
        ) {
            ButtonEditorAction::Saved { index, button } => {
                self.save_button_update(index, button, "按钮已保存");
            }
            ButtonEditorAction::Cancelled | ButtonEditorAction::None => {}
        }
        if let Some(req) = self.button_editor.macro_request.take() {
            self.handle_macro_request(ctx, req, MacroSessionTarget::ButtonEditor);
        }
        if let Some(req) = self.button_editor.app_pick_request.take() {
            self.handle_app_pick_request(ctx, req);
        }
    }

    fn handle_app_pick_request(&mut self, ctx: &egui::Context, request: AppPickRequest) {
        match request {
            AppPickRequest::Cancel => {
                self.cancel_app_pick();
                return;
            }
            AppPickRequest::Start
            | AppPickRequest::StartForScriptActivate
            | AppPickRequest::StartForScriptOpenApp => {}
        }

        if self.macro_record_mode() != RecordMode::Idle {
            self.state.hint = Some(crate::ui::HintOverlay::show(
                "请先结束宏录制再点选应用".into(),
            ));
            return;
        }

        let perms = check_permissions();
        if !perms.input_monitoring {
            self.state.permission_dialog = crate::ui::PermissionDialog::new();
            self.state.hint = Some(crate::ui::HintOverlay::show(
                "点选应用需要输入监控权限".into(),
            ));
            return;
        }

        let Some(picker) = self.app_picker.as_ref() else {
            self.state.hint = Some(crate::ui::HintOverlay::show(
                "点选应用不可用（事件监听未启动）".into(),
            ));
            return;
        };

        if let Err(err) = picker.start() {
            log::error!("app pick start failed: {err}");
            self.state.permission_dialog = crate::ui::PermissionDialog::new();
            self.state.hint = Some(crate::ui::HintOverlay::show(
                "点选应用需要输入监控权限".into(),
            ));
            return;
        }

        self.button_editor.app_picking = true;
        self.button_editor.script_app_picking =
            matches!(request, AppPickRequest::StartForScriptActivate);
        self.button_editor.script_open_app_picking =
            matches!(request, AppPickRequest::StartForScriptOpenApp);
        self.pause_gestures();
        self.hide_main_panel(ctx);
        self.state.hint = Some(crate::ui::HintOverlay::show(
            if self.button_editor.script_open_app_picking {
                "点击目标应用窗口以插入打开应用（Esc 取消）".into()
            } else if self.button_editor.script_app_picking {
                "点击目标应用窗口以插入激活应用（Esc 取消）".into()
            } else {
                "点击目标应用窗口（Esc 取消）".into()
            },
        ));
        ctx.request_repaint();
    }

    fn cancel_app_pick(&mut self) {
        if let Some(picker) = &self.app_picker {
            picker.cancel();
        }
        self.button_editor.app_picking = false;
        self.button_editor.script_app_picking = false;
        self.button_editor.script_open_app_picking = false;
        self.resume_gestures();
    }

    fn poll_app_picker(&mut self, ctx: &egui::Context) {
        let Some(picker) = &self.app_picker else {
            return;
        };
        let picking = picker.is_picking();
        let outcome = picker.poll();
        if picking && !self.button_editor.is_open() {
            if let Some(picker) = &self.app_picker {
                picker.cancel();
            }
            self.button_editor.app_picking = false;
            self.button_editor.script_app_picking = false;
            self.button_editor.script_open_app_picking = false;
            self.resume_gestures();
        }
        let Some(outcome) = outcome else {
            return;
        };
        self.resume_gestures();
        match outcome {
            PickOutcome::Picked {
                bundle_id,
                app_name,
            } => {
                let for_open = self.button_editor.script_open_app_picking;
                let for_script = self.button_editor.script_app_picking;
                self.button_editor.apply_app_pick(bundle_id, app_name);
                self.state.hint = Some(crate::ui::HintOverlay::show(if for_open {
                    "已插入打开应用脚本行".into()
                } else if for_script {
                    "已插入激活应用脚本行".into()
                } else {
                    "已填充可见范围应用".into()
                }));
            }
            PickOutcome::Cancelled => {
                self.button_editor.app_picking = false;
                self.button_editor.script_app_picking = false;
                self.button_editor.script_open_app_picking = false;
                if self.button_editor.is_open() {
                    self.state.hint = Some(crate::ui::HintOverlay::show("已取消点选应用".into()));
                }
            }
        }
        ctx.request_repaint();
    }

    fn macro_record_mode(&self) -> RecordMode {
        self.macro_recorder
            .as_ref()
            .map(|r| r.mode())
            .unwrap_or(RecordMode::Idle)
    }

    fn handle_macro_request(
        &mut self,
        ctx: &egui::Context,
        request: MacroEditorRequest,
        target: MacroSessionTarget,
    ) {
        match request {
            MacroEditorRequest::CancelRecord => {
                if let Some(recorder) = &self.macro_recorder {
                    recorder.cancel();
                }
                self.macro_session = None;
                self.resume_gestures();
                return;
            }
            MacroEditorRequest::FinishMacroRecord => {
                if let Some(recorder) = &self.macro_recorder {
                    recorder.finish_sequence();
                }
                return;
            }
            MacroEditorRequest::TryRunScript => {
                self.try_run_script_from_editor(&target);
                return;
            }
            MacroEditorRequest::ScriptCaptureApp => {
                if self.macro_record_mode() != RecordMode::Idle {
                    if let Some(recorder) = &self.macro_recorder {
                        recorder.cancel();
                    }
                    self.macro_session = None;
                    self.resume_gestures();
                }
                self.handle_app_pick_request(ctx, AppPickRequest::StartForScriptActivate);
                return;
            }
            MacroEditorRequest::ScriptPickOpenApp => {
                if self.macro_record_mode() != RecordMode::Idle {
                    if let Some(recorder) = &self.macro_recorder {
                        recorder.cancel();
                    }
                    self.macro_session = None;
                    self.resume_gestures();
                }
                self.handle_app_pick_request(ctx, AppPickRequest::StartForScriptOpenApp);
                return;
            }
            MacroEditorRequest::ScriptCaptureClick | MacroEditorRequest::StartMacroRecord => {
                if self.button_editor.app_picking
                    || self.app_picker.as_ref().is_some_and(|p| p.is_picking())
                {
                    self.cancel_app_pick();
                }
            }
        }

        let perms = check_permissions();
        if !perms.accessibility || !perms.input_monitoring {
            self.state.permission_dialog = crate::ui::PermissionDialog::new();
            self.state.hint = Some(crate::ui::HintOverlay::show(
                "录制鼠标宏需要辅助功能与输入监控权限".into(),
            ));
            return;
        }

        if self.macro_recorder.is_none() {
            self.state.hint = Some(crate::ui::HintOverlay::show(
                "鼠标宏录制不可用（事件监听未启动）".into(),
            ));
            return;
        }

        self.macro_session = Some(target);
        self.pause_gestures();
        self.hide_main_panel(ctx);

        match request {
            MacroEditorRequest::ScriptCaptureClick => {
                if let Some(recorder) = &self.macro_recorder {
                    recorder.start_capture_once();
                }
                self.state.hint = Some(crate::ui::HintOverlay::show(
                    "请到目标窗口点击一次（Esc 取消）".into(),
                ));
            }
            MacroEditorRequest::StartMacroRecord => {
                if let Some(recorder) = &self.macro_recorder {
                    recorder.start_sequence();
                }
                self.state.hint = Some(crate::ui::HintOverlay::show(
                    "序列录制中：点「完成录制」结束，Esc 取消".into(),
                ));
            }
            MacroEditorRequest::CancelRecord
            | MacroEditorRequest::FinishMacroRecord
            | MacroEditorRequest::TryRunScript
            | MacroEditorRequest::ScriptCaptureApp
            | MacroEditorRequest::ScriptPickOpenApp => {}
        }
        ctx.request_repaint();
    }

    fn try_run_script_from_editor(&mut self, target: &MacroSessionTarget) {
        use crate::config::ActionScript;

        let (source, attached) = match target {
            MacroSessionTarget::ButtonEditor => {
                let ed = &self.button_editor.action_editor;
                (ed.script_source.clone(), ed.script_attached_macro.clone())
            }
            MacroSessionTarget::Settings { profile, button } => {
                if let Some(state) = self.settings.button_action_cache.get(*button) {
                    (
                        state.script_source.clone(),
                        state.script_attached_macro.clone(),
                    )
                } else if let Some(btn) = self
                    .settings
                    .draft
                    .profiles
                    .get(*profile)
                    .and_then(|p| p.buttons.get(*button))
                {
                    match &btn.action {
                        crate::config::Action::Script(s) => {
                            (s.source.clone(), s.attached_macro.clone())
                        }
                        _ => return,
                    }
                } else {
                    return;
                }
            }
        };

        let program = match crate::action::compile(&source, attached.as_ref()) {
            Ok(p) => p,
            Err(err) => {
                self.state.hint = Some(crate::ui::HintOverlay::show(err.primary_display()));
                return;
            }
        };
        let script = ActionScript {
            source,
            program: Some(program),
            attached_macro: attached,
        };
        match self.state.executor.try_run_script(&script) {
            Ok(()) => {
                self.state.hint = Some(crate::ui::HintOverlay::show(
                    "试运行已执行（请确认目标窗口已在前台）".into(),
                ));
            }
            Err(err) => {
                self.state.hint = Some(crate::ui::HintOverlay::show(format!("试运行失败：{err}")));
            }
        }
    }

    fn pause_gestures(&mut self) {
        if let Some(monitor) = &mut self.mouse_gesture {
            let mut cfg = self.state.app_config.gesture.clone();
            cfg.enabled = false;
            cfg.middle_click_enabled = false;
            monitor.update_config(&cfg);
        }
    }

    fn resume_gestures(&mut self) {
        if let Some(monitor) = &mut self.mouse_gesture {
            monitor.update_config(&self.state.app_config.gesture);
        }
    }

    fn poll_macro_recorder(&mut self, ctx: &egui::Context) {
        let Some(recorder) = &self.macro_recorder else {
            return;
        };
        let Some(outcome) = recorder.poll_outcome() else {
            return;
        };
        let target = self.macro_session.take();
        self.resume_gestures();
        let exclude_pid = Some(std::process::id());

        match outcome {
            MacroRecordOutcome::Cancelled => {
                self.state.hint = Some(crate::ui::HintOverlay::show("已取消宏录制".into()));
            }
            MacroRecordOutcome::CaptureOnce { x, y } => {
                std::thread::sleep(Duration::from_millis(50));
                let result = simplify_capture_once(x, y, exclude_pid);
                let (nx, ny) = result
                    .steps
                    .iter()
                    .find_map(|s| match s {
                        crate::config::MacroStep::Click { x, y, .. } => Some((*x, *y)),
                        _ => None,
                    })
                    .unwrap_or((x as f32, y as f32));
                self.apply_script_click(ctx, target, nx, ny);
            }
            MacroRecordOutcome::Sequence { events } => {
                std::thread::sleep(Duration::from_millis(50));
                let result = simplify_events(&events, exclude_pid);
                self.apply_macro_sequence(ctx, target, result);
            }
        }
        ctx.request_repaint();
    }

    fn apply_script_click(
        &mut self,
        ctx: &egui::Context,
        target: Option<MacroSessionTarget>,
        x: f32,
        y: f32,
    ) {
        match target {
            Some(MacroSessionTarget::ButtonEditor) => {
                self.button_editor.action_editor.insert_script_click(x, y);
                self.state.hint = Some(crate::ui::HintOverlay::show(
                    "已插入点击坐标到动作脚本".into(),
                ));
            }
            Some(MacroSessionTarget::Settings { profile, button }) => {
                self.settings.sync_button_action_cache(profile);
                if let Some(state) = self.settings.button_action_cache.get_mut(button) {
                    state.insert_script_click(x, y);
                    if let Some(btn) = self
                        .settings
                        .draft
                        .profiles
                        .get_mut(profile)
                        .and_then(|p| p.buttons.get_mut(button))
                    {
                        btn.enabled = true;
                        btn.action = state.to_action();
                    }
                }
                self.state.hint = Some(crate::ui::HintOverlay::show(
                    "已插入点击坐标到动作脚本".into(),
                ));
            }
            None => {
                log::warn!("script click capture without session target");
            }
        }
        let _ = ctx;
    }

    fn apply_macro_sequence(
        &mut self,
        ctx: &egui::Context,
        target: Option<MacroSessionTarget>,
        result: SimplifyResult,
    ) {
        if result.steps.is_empty() {
            self.state.hint = Some(crate::ui::HintOverlay::show(
                "未捕获到有效步骤，请重试".into(),
            ));
            return;
        }
        let macro_ = crate::config::MouseMacro {
            steps: result.steps,
            pre_delay_ms: crate::config::MouseMacro::DEFAULT_PRE_DELAY_MS,
            recorded_bundle_id: result.recorded_bundle_id,
        };
        match target {
            Some(MacroSessionTarget::ButtonEditor) => {
                self.button_editor.action_editor.append_macro_script(&macro_);
                self.button_editor.draft_enable_for_macro();
                self.state.hint = Some(crate::ui::HintOverlay::show(
                    "录制已追加到动作脚本".into(),
                ));
            }
            Some(MacroSessionTarget::Settings { profile, button }) => {
                self.settings.apply_macro_to_button(profile, button, &macro_);
                self.state.hint = Some(crate::ui::HintOverlay::show(
                    "录制已追加到动作脚本".into(),
                ));
            }
            None => {
                log::warn!("macro sequence without session target");
            }
        }
        let _ = ctx;
    }

    fn open_button_script_editor(&mut self, ctx: &egui::Context, index: usize) {
        let Some(profile_index) = self.state.current_profile_index() else {
            return;
        };
        let buttons = self.state.panel.padded_buttons();
        let Some(button) = buttons.get(index) else {
            return;
        };
        self.button_editor.open_script(
            index,
            profile_index,
            &self.state.current_profile,
            button,
        );
        ctx.request_repaint();
    }

    fn open_button_appearance_editor(&mut self, ctx: &egui::Context, index: usize) {
        let Some(profile_index) = self.state.current_profile_index() else {
            return;
        };
        let buttons = self.state.panel.padded_buttons();
        let Some(button) = buttons.get(index) else {
            return;
        };
        self.button_editor.open_appearance(
            index,
            profile_index,
            &self.state.current_profile,
            button,
        );
        ctx.request_repaint();
    }

    fn rebuild_theme(&mut self, ctx: &egui::Context) {
        self.last_letter_font = self.state.panel.ui_style.letter_font.clone();
        setup_theme_fonts(ctx, self.last_letter_font.as_deref());
        self.theme = Theme::from_ui_style(&self.state.panel.ui_style);
    }

    fn handle_keyboard(&mut self, ctx: &egui::Context) {
        if !self.state.visible || !self.state.keyboard_active() {
            return;
        }

        let rows = self.state.panel.rows;
        let cols = self.state.panel.cols;
        let selectable = self.state.slot_selectable();
        match handle_panel_keyboard(ctx, rows, cols, &mut self.state.selected_cell, &selectable) {
            KeyboardAction::Trigger(index) => {
                self.trigger_panel_button(ctx, index);
            }
            KeyboardAction::ClearSelection => {}
            KeyboardAction::None => {}
        }
    }

    fn trigger_panel_button(&mut self, ctx: &egui::Context, index: usize) {
        let is_mouse_macro = self
            .state
            .display_buttons()
            .get(index)
            .is_some_and(|b| matches!(b.action, crate::config::Action::MouseMacro(_)));
        // 鼠标宏回放前先隐藏面板，避免点到自身；置顶时也隐藏，否则焦点常留在 MeoTool
        let hide_before = is_mouse_macro && self.state.visible;
        let was_pinned = self.state.pinned;
        if hide_before {
            self.gesture_leave_hide_after = None;
            self.hide_main_panel(ctx);
        }
        let _ok = self.state.trigger_button(index, &self.theme);
        // 未钉住手势会话：点按后不立刻收起，等鼠标离开面板再藏
        // 置顶模式下宏回放后恢复显示
        if hide_before && was_pinned {
            self.state.pinned = true;
            self.show_main_panel(ctx);
        }
    }

    fn update_gesture_leave_hide(&mut self, ctx: &egui::Context) {
        let should = self.state.gesture_session
            && self.state.visible
            && !self.state.pinned
            && self.state.app_config.gesture.hide_after_button;
        if !should {
            self.gesture_leave_hide_after = None;
            return;
        }
        let over = ctx.input(|i| {
            let Some(pos) = i.pointer.hover_pos().or_else(|| i.pointer.interact_pos()) else {
                // 无指针信息时用全局坐标 + outer_rect
                return None;
            };
            Some(
                i.viewport()
                    .outer_rect
                    .is_some_and(|r| r.contains(pos)),
            )
        });
        let over = match over {
            Some(v) => v,
            None => {
                let Some((mx, my)) = crate::platform::global_mouse_pos() else {
                    return;
                };
                ctx.input(|i| {
                    i.viewport()
                        .outer_rect
                        .is_some_and(|r| r.contains(egui::pos2(mx, my)))
                })
            }
        };
        if over {
            self.gesture_leave_hide_after = None;
            return;
        }
        const DELAY: std::time::Duration = std::time::Duration::from_millis(280);
        match self.gesture_leave_hide_after {
            None => {
                self.gesture_leave_hide_after = Some(Instant::now() + DELAY);
                ctx.request_repaint_after(DELAY);
            }
            Some(deadline) if Instant::now() >= deadline => {
                self.gesture_leave_hide_after = None;
                self.hide_main_panel(ctx);
            }
            Some(deadline) => {
                ctx.request_repaint_after(deadline.saturating_duration_since(Instant::now()));
            }
        }
    }

    fn toggle_pin(&mut self, ctx: &egui::Context) {
        let was_pinned = self.state.pinned;
        self.state.pinned = !self.state.pinned;
        log::info!(
            "panel pin {}",
            if self.state.pinned {
                "enabled"
            } else {
                "disabled"
            }
        );
        if was_pinned && !self.state.pinned && self.state.gesture_session {
            self.hide_main_panel(ctx);
        }
    }

    fn poll_mouse_gesture(&mut self, ctx: &egui::Context) {
        let Some(monitor) = &mut self.mouse_gesture else {
            return;
        };

        if self.app_picker.as_ref().is_some_and(|p| p.is_picking())
            || self.button_editor.app_picking
        {
            let mut cfg = self.state.app_config.gesture.clone();
            cfg.enabled = false;
            cfg.middle_click_enabled = false;
            monitor.update_config(&cfg);
            let _ = monitor.poll();
            return;
        }

        monitor.update_config(&self.state.app_config.gesture);

        let ignore_rect = if self.state.visible {
            ctx.input(|i| {
                i.viewport()
                    .outer_rect
                    .map(|r| [r.min.x, r.min.y, r.max.x, r.max.y])
            })
        } else {
            None
        };
        monitor.set_ignore_rect(ignore_rect);
        monitor.set_dismiss_armed(
            self.state.visible
                && self.state.gesture_session
                && !self.state.pinned
                && self.state.app_config.gesture.dismiss_on_outside_click,
        );

        let actions = monitor.poll();
        for action in actions {
            match action {
                MouseGestureAction::ShowAt { x, y } => {
                    let anchor = egui::pos2(x, y);
                    self.gesture_leave_hide_after = None;
                    self.state.show_at_gesture(anchor);
                    self.apply_panel_shown_viewport(ctx);
                }
                MouseGestureAction::LeftMouseDown { x, y } => {
                    if !self.state.gesture_session || !self.state.visible {
                        continue;
                    }
                    let click = egui::pos2(x, y);
                    let outside = ctx
                        .input(|i| i.viewport().outer_rect)
                        .is_none_or(|rect| !rect.contains(click));
                    if outside && !self.state.pinned {
                        self.gesture_leave_hide_after = None;
                        self.hide_main_panel(ctx);
                    }
                }
            }
        }
    }
}

impl eframe::App for MeoToolApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.header_dragging && !ctx.input(|i| i.pointer.any_down()) {
            if let Some(pos) = ctx.input(|i| i.viewport().outer_rect.map(|r| r.min)) {
                self.state.save_position(pos);
            }
            self.header_dragging = false;
        }

        self.poll_window_events();
        self.poll_config_events();
        self.sync_tray_icon();
        self.sync_numpad_hotkeys();
        self.poll_hotkeys(ctx);
        self.poll_tray(ctx);
        self.poll_mouse_gesture(ctx);
        self.update_gesture_leave_hide(ctx);
        self.poll_macro_recorder(ctx);

        self.handle_settings(ctx);
        self.flush_pending_settings_save(false);
        self.handle_button_editor(ctx);
        self.poll_app_picker(ctx);
        if self.state.permission_dialog.show(ctx) {
            ctx.request_repaint();
        }
        self.sync_main_around_dialogs(ctx);

        self.maybe_reset_fonts(ctx);
        self.memory.maybe_log(
            ctx,
            &self.state.icons,
            self.state.visible || self.dialogs_open(),
        );

        let continuous = self.settings.needs_continuous_repaint()
            || self.button_editor.needs_continuous_repaint()
            || self.button_context_menu.is_open()
            || self.launcher_context_menu.is_open()
            || self.macro_record_mode() != RecordMode::Idle
            || self.button_editor.app_picking;

        if continuous {
            ctx.request_repaint();
        } else if let Some(interval) = self
            .settings
            .idle_repaint_interval()
            .or_else(|| self.button_editor.idle_repaint_interval())
        {
            ctx.request_repaint_after(interval);
        } else if self.dialogs_open() {
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
        } else if !self.state.visible {
            ctx.request_repaint_after(std::time::Duration::from_millis(400));
        } else if self.state.is_collapsed_mode() && !self.state.collapsed.expanded {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }

        if self.state.poll_config_reload() {
            if let Err(err) = self.hotkey_manager.replace(&self.state.app_config.hotkeys) {
                log::error!("re-register hotkeys after reload: {err}");
            }
            self.rebuild_theme(ctx);
            self.sync_tray_icon();
            if !self.dialogs_open() {
                self.sync_viewport_size(ctx, self.state.effective_viewport_size());
            }
        }

        if self.state.quit_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        if self.dialogs_open() {
            return;
        }

        if self.pending_panel_focus && self.state.visible {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.pending_panel_focus = false;
        } else if self.state.request_focus {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.state.request_focus = false;
        }

        if !self.state.visible {
            self.button_context_menu.close();
            self.launcher_context_menu.close();
            return;
        }

        self.sync_theme_from_panel(ctx);
        self.handle_keyboard(ctx);
        self.state.tick_flash();

        if let Some((_alpha, w, h)) = self.state.tick_transition() {
            self.sync_viewport_size(ctx, (w, h));
        }

        let content_alpha = self.state.content_alpha();
        let opacity = self.state.panel.opacity;
        let flash = self.state.flash_index;
        let selected = self.state.selected_cell;
        let pet_mode = self.state.is_collapsed_mode();
        let theme = self.theme.clone();

        // 小克隆：解开 grid 对 state 的拆借冲突（按钮数通常 ≤ 16）
        let display_buttons = self.state.display_buttons().to_vec();

        show_panel(ctx, &theme, opacity, content_alpha, pet_mode, |ui| {
            if !self.state.is_collapsed_mode() {
                let active_window = self
                    .state
                    .last_active_window
                    .as_ref()
                    .filter(|window| !window.is_own_process());
                let active_app_name = active_window.map(|window| window.display_name());
                let active_app_tooltip =
                    active_window.and_then(|window| header_app_tooltip(window));
                let header_tooltip = active_window.and_then(|window| window.page_url.as_deref());
                let collapse_icon = self
                    .state
                    .panel
                    .collapsed_icon
                    .as_deref()
                    .unwrap_or(PanelConfig::DEFAULT_COLLAPSED_ICON);
                let header = show_panel_header(
                    ui,
                    ctx,
                    &mut self.state.icons,
                    self.state.pinned,
                    collapse_icon,
                    active_app_name.as_deref(),
                    active_app_tooltip.as_deref(),
                    header_tooltip,
                );
                if header.native_drag_started {
                    self.header_dragging = true;
                }
                if header.pin_clicked {
                    self.toggle_pin(ctx);
                }
                if header.collapse_clicked {
                    self.collapse_to_fold_mode(ctx);
                }
            }

            let suppress_collapse = self.header_dragging
                || self.button_context_menu.is_open()
                || self.launcher_context_menu.is_open();

            let response = self.state.grid.show(
                ui,
                &theme,
                &self.state.panel,
                &display_buttons,
                &mut self.state.icons,
                flash,
                selected,
                &mut self.state.collapsed,
                suppress_collapse,
                &mut self.button_context_menu,
                &mut self.launcher_context_menu,
            );
            if response.native_drag_started {
                self.header_dragging = true;
            }
            if response.expand_to_full {
                self.expand_to_full_mode(ctx);
            }
            if let Some(index) = response.triggered {
                self.trigger_panel_button(ctx, index);
            }
            if let Some(size) = response.viewport_size {
                self.sync_viewport_size(ctx, size);
            }
            if response.needs_repaint {
                ctx.request_repaint();
            }
        });

        let buttons = self.state.panel.padded_buttons();
        if let Some((index, action)) = self.button_context_menu.show(ctx, &buttons) {
            self.handle_button_menu(ctx, index, action);
        }

        if let Some(action) = self.launcher_context_menu.show(ctx, self.state.pinned) {
            self.handle_launcher_menu(ctx, action);
        }

        if let Some(hint) = &self.state.hint {
            hint.paint(ctx);
            if !hint.is_active() {
                self.state.hint = None;
            }
        }

        if self.state.needs_repaint() {
            ctx.request_repaint();
        }
    }
}
