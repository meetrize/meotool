//! NSApplication + 定时轮询：热键 / 手势 / tray / 配置 / 面板 / 设置窗口。

use std::cell::RefCell;
use std::ptr::NonNull;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use block2::RcBlock;
use notify::RecommendedWatcher;
use objc2::rc::Retained;
use objc2::DefinedClass;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSEvent, NSEventMask,
};
use objc2_foundation::{MainThreadMarker, NSNotification, NSObject, NSObjectProtocol, NSTimer};

use crate::app::{
    memory::{self, HIDDEN_IDLE_PURGE_AFTER, PURGE_MIN_INTERVAL},
    AppCore, CoreCommand, CoreEffect, MemoryMonitor, WakeHandle,
};
use crate::action::{simplify_capture_once, simplify_events};
use crate::config::{MouseMacro, save_app_config, save_panel_position, spawn_config_watcher, LoadedConfig, WindowInfo};
use crate::input::{HotkeyAction, HotkeyManager};
use crate::platform::{
    check_accessibility_hint, check_permissions, global_mouse_pos, relaunch_app, AppClickPicker,
    MacroRecordOutcome, MacroRecorder, MouseGestureAction, MouseGestureMonitor, PickOutcome,
    TrayAction, TrayManager,
};
use crate::window::{spawn_watcher, WindowWatcherControl};

use super::activation::install_standard_main_menu;
use super::appearance_editor::AppearanceEditorWindow;
use super::button_edit_session::ButtonEditSession;
use super::button_editor::ScriptEditorWindow;
use super::events::{
    drain as drain_ui_events, install_ui_channel, ButtonMenuAction, IconPickTarget,
    LauncherMenuAction, NativeUiEvent,
};
use super::backup_dialog::{run_backup_restore_flow, show_backup_restore_result};
use super::export_dialog::{run_export_flow, show_export_result};
use super::import_dialog::{run_import_flow, show_import_result};
use super::icon_picker::IconPickerWindow;
use super::panel::{drain_button_clicks, NativePanel};
use super::permissions::PermissionsWindow;
use super::settings::SettingsWindow;

const COLLAPSE_DELAY: Duration = Duration::from_millis(500);
const LAUNCHER_DRAG_THRESHOLD: f32 = 4.0;
const POSITION_SAVE_DEBOUNCE: Duration = Duration::from_millis(300);
const SETTINGS_SAVE_DEBOUNCE: Duration = Duration::from_millis(400);
/// 忽略本进程写盘后 watcher 回弹的时间窗
const SELF_WRITE_IGNORE: Duration = Duration::from_millis(500);

#[derive(Clone, Copy)]
struct LauncherDrag {
    grab_x: f32,
    grab_y: f32,
    origin_x: f64,
    origin_y: f64,
    moved: bool,
}

#[derive(Default)]
struct CollapsedUiState {
    expanded: bool,
    collapse_after: Option<Instant>,
    /// 按桌宠屏幕矩形几何拖窗（不依赖透明像素命中）
    drag: Option<LauncherDrag>,
    left_was_down: bool,
    last_launcher_click: Option<Instant>,
}

/// 拖动跟手：全局/本地鼠标事件监视器（比 50ms tick 平滑）
struct LauncherDragMonitors {
    global: Retained<AnyObject>,
    local: Retained<AnyObject>,
}

struct NativeState {
    core: AppCore,
    panel: NativePanel,
    settings: SettingsWindow,
    script_editor: ScriptEditorWindow,
    appearance_editor: AppearanceEditorWindow,
    button_edit_session: Option<ButtonEditSession>,
    icon_picker: IconPickerWindow,
    permissions: PermissionsWindow,
    ui_rx: mpsc::Receiver<NativeUiEvent>,
    hotkeys: HotkeyManager,
    mouse_gesture: Option<MouseGestureMonitor>,
    app_picker: Option<AppClickPicker>,
    tray: Option<TrayManager>,
    window_rx: mpsc::Receiver<WindowInfo>,
    config_rx: mpsc::Receiver<()>,
    _config_watcher: Option<RecommendedWatcher>,
    mtm: MainThreadMarker,
    permissions_prompted: bool,
    /// 正在退出：禁止 tick 重入（terminate 会嵌套跑 RunLoop，定时器再进 tick 会 RefCell panic）
    terminating: bool,
    collapsed_ui: CollapsedUiState,
    launcher_drag_monitors: Option<LauncherDragMonitors>,
    memory: MemoryMonitor,
    /// 面板需要与 AppCore 同步（配置/显隐/折叠展开等）
    panel_sync_dirty: bool,
    panel_hidden_since: Option<Instant>,
    last_idle_purge: Instant,
    window_watch: std::sync::Arc<WindowWatcherControl>,
    /// 完整面板拖动结束后 debounce 写盘
    pending_position_save: Option<Instant>,
    /// 设置变更防抖写盘
    pending_settings_save: Option<Instant>,
    /// 本进程刚写入配置的时间（忽略 watcher 回弹）
    last_settings_self_write: Option<Instant>,
    /// 动作脚本「捕获点击」
    script_click_recorder: Option<MacroRecorder>,
}

enum TickOutcome {
    Continue,
    Quit,
}

impl NativeState {
    fn tick(&mut self) -> TickOutcome {
        if self.terminating {
            return TickOutcome::Continue;
        }
        self.poll_hotkeys();
        self.poll_tray();
        self.poll_gestures();
        self.poll_button_clicks();
        self.poll_config();
        self.poll_active_window();
        self.settings.poll_actions();
        if let Some(session) = self.button_edit_session.as_mut() {
            self.script_editor.poll(session);
            self.appearance_editor.poll(session);
        }
        self.poll_script_click_capture();
        self.poll_app_picker();
        self.icon_picker.poll();
        self.permissions.poll();
        self.poll_ui_events();
        self.maybe_prompt_permissions();
        self.update_collapsed_hover();
        self.sync_panel_if_needed();
        self.flush_pending_position_save(false);
        self.flush_pending_settings_save(false);
        self.maybe_idle_memory_purge();
        self.memory.maybe_log_native(
            self.panel.memory_counters(),
            self.core.visible,
            self.core.is_collapsed_mode(),
            self.panel.icon_cache_resident_count(),
        );
        if self.core.quit_requested {
            self.flush_pending_position_save(true);
            self.flush_pending_settings_save(true);
            self.terminating = true;
            return TickOutcome::Quit;
        }
        TickOutcome::Continue
    }

    fn mark_panel_dirty(&mut self) {
        self.panel_sync_dirty = true;
    }

    fn apply(&mut self, effect: CoreEffect) {
        match effect {
            CoreEffect::None => {}
            CoreEffect::RefreshPanel | CoreEffect::VisibilityChanged => {
                self.mark_panel_dirty();
                self.sync_panel_if_needed();
            }
            CoreEffect::Quit => {
                self.core.quit_requested = true;
            }
            CoreEffect::OpenSettingsUi => self.open_settings(),
            CoreEffect::OpenPermissionsUi => self.permissions.show(),
            CoreEffect::OpenButtonEditor { profile, button } => {
                self.open_button_editor(profile, button, true);
            }
        }
    }

    fn open_settings(&mut self) {
        // 保持面板可见，便于外观即时预览；设置窗使用 Normal 窗级
        let cfg = self.core.app_config.clone();
        self.settings.reload_from_config(&cfg);
        self.settings.show();
    }

    fn on_apply_settings(&mut self, cfg: crate::config::AppConfig) {
        let effect = self.core.apply_config(cfg);
        if let Err(err) = self.hotkeys.replace(&self.core.app_config.hotkeys) {
            log::warn!("re-register hotkeys: {err}");
        }
        self.apply(effect);
        self.pending_settings_save = Some(Instant::now() + SETTINGS_SAVE_DEBOUNCE);
    }

    fn flush_pending_settings_save(&mut self, force: bool) {
        let due = self.pending_settings_save.is_some_and(|deadline| {
            force || Instant::now() >= deadline
        });
        if !due {
            return;
        }
        self.pending_settings_save = None;
        match save_app_config(&self.core.config_path, &self.core.app_config) {
            Ok(()) => {
                self.last_settings_self_write = Some(Instant::now());
                if self.settings.is_visible() {
                    self.settings.notify_saved();
                }
            }
            Err(err) => log::error!("save settings failed: {err}"),
        }
    }

    fn should_ignore_config_watcher(&self) -> bool {
        self.last_settings_self_write
            .is_some_and(|t| t.elapsed() < SELF_WRITE_IGNORE)
    }

    fn ensure_button_session(&mut self, profile: usize, button: usize) {
        let needs_new = self
            .button_edit_session
            .as_ref()
            .map(|s| !s.matches(profile, button))
            .unwrap_or(true);
        if !needs_new {
            return;
        }
        let cfg = self
            .core
            .app_config
            .profiles
            .get(profile)
            .and_then(|p| p.buttons.get(button))
            .cloned()
            .unwrap_or_else(crate::config::empty_slot_button);
        let (theme_button_bg, theme_icon_color) = self
            .core
            .app_config
            .profiles
            .get(profile)
            .map(|p| self.core.app_config.build_panel(p))
            .map(|panel| (panel.ui_style.button_bg, panel.ui_style.letter_color))
            .unwrap_or((
                crate::config::RgbColor::rgba(40, 40, 40, 217),
                crate::config::RgbColor::rgb(224, 224, 224),
            ));
        self.button_edit_session = Some(ButtonEditSession::new(
            profile,
            button,
            cfg,
            &self.core.icon_dirs,
            theme_button_bg,
            theme_icon_color,
        ));
    }

    fn open_button_editor(&mut self, profile: usize, button: usize, script: bool) {
        self.ensure_button_session(profile, button);
        let Some(session) = self.button_edit_session.as_mut() else {
            return;
        };
        if script {
            self.appearance_editor.hide();
            self.script_editor.open(session);
        } else {
            self.script_editor.hide();
            self.appearance_editor.open(session);
        }
    }

    fn close_button_editors(&mut self) {
        self.script_editor.hide();
        self.appearance_editor.hide();
        self.button_edit_session = None;
    }

    fn button_editor_any_visible(&self) -> bool {
        self.script_editor.is_visible() || self.appearance_editor.is_visible()
    }

    fn start_app_pick(&mut self) {
        if self.app_picker.as_ref().is_some_and(|p| p.is_picking()) {
            return;
        }
        let perms = check_permissions();
        if !perms.input_monitoring {
            log::warn!("app pick needs Input Monitoring permission");
            self.permissions.show();
            self.appearance_editor.set_app_picking(false);
            self.script_editor.set_script_app_picking(false);
            return;
        }
        let effect = self.core.handle(CoreCommand::HidePanel);
        self.apply(effect);
        let Some(picker) = self.app_picker.as_ref() else {
            log::warn!("app click picker unavailable");
            self.permissions.show();
            self.script_editor.set_script_app_picking(false);
            return;
        };
        if let Err(err) = picker.start() {
            log::error!("app pick start failed: {err}");
            self.permissions.show();
            self.appearance_editor.set_app_picking(false);
            self.script_editor.set_script_app_picking(false);
            return;
        }
        if !self.script_editor.is_script_app_picking()
            && !self.script_editor.is_script_open_app_picking()
        {
            self.appearance_editor.set_app_picking(true);
        }
    }

    fn cancel_app_pick(&mut self) {
        if let Some(picker) = &self.app_picker {
            picker.cancel();
        }
        self.appearance_editor.set_app_picking(false);
        self.script_editor.set_script_app_picking(false);
        self.script_editor.set_script_open_app_picking(false);
    }

    fn start_script_macro_record(&mut self) {
        let perms = check_permissions();
        if !perms.input_monitoring || !perms.accessibility {
            log::warn!("script macro record needs Accessibility + Input Monitoring");
            self.permissions.show();
            self.script_editor.cancel_macro_recording();
            return;
        }
        if let Some(rec) = self.script_click_recorder.take() {
            rec.cancel();
        }
        match MacroRecorder::new(crate::app::WakeHandle::new()) {
            Ok(rec) => {
                rec.start_sequence();
                self.script_click_recorder = Some(rec);
            }
            Err(err) => {
                log::error!("script macro record start failed: {err}");
                self.script_editor.cancel_macro_recording();
            }
        }
    }

    fn finish_script_macro_record(&mut self) {
        if let Some(rec) = self.script_click_recorder.as_ref() {
            rec.finish_sequence();
        }
    }

    fn stop_script_macro_record(&mut self) {
        if let Some(rec) = self.script_click_recorder.take() {
            rec.cancel();
        }
        self.script_editor.cancel_macro_recording();
    }

    fn start_script_click_capture(&mut self) {
        let perms = check_permissions();
        if !perms.input_monitoring || !perms.accessibility {
            log::warn!("script click capture needs Accessibility + Input Monitoring");
            self.permissions.show();
            self.script_editor.cancel_script_capture();
            return;
        }
        if let Some(rec) = self.script_click_recorder.take() {
            rec.cancel();
        }
        match MacroRecorder::new(crate::app::WakeHandle::new()) {
            Ok(rec) => {
                rec.start_capture_once();
                self.script_click_recorder = Some(rec);
            }
            Err(err) => {
                log::error!("script click capture start failed: {err}");
                self.permissions.show();
                self.script_editor.cancel_script_capture();
            }
        }
    }

    fn stop_script_click_capture(&mut self) {
        if let Some(rec) = self.script_click_recorder.take() {
            rec.cancel();
        }
        self.script_editor.cancel_script_capture();
    }

    /// 试运行：先收起 MeoTool 窗体，再后台回放，结束后恢复并反馈错误。
    fn begin_try_run_script(&mut self, source: String, attached_macro: Option<MouseMacro>) {
        use crate::config::ActionScript;

        let program = match crate::action::compile(&source, attached_macro.as_ref()) {
            Ok(p) => p,
            Err(err) => {
                show_try_run_alert(self.mtm, &err.primary_display());
                self.script_editor
                    .set_try_run_status("状态：编译失败，无法试运行");
                return;
            }
        };

        let restore_script = self.script_editor.is_visible();
        let restore_panel = self.core.visible;
        let restore_settings = self.settings.is_visible();

        if restore_script {
            self.script_editor.withdraw_for_try_run();
        }
        if restore_settings {
            self.settings.hide();
        }
        if restore_panel {
            let effect = self.core.handle(CoreCommand::HidePanel);
            self.apply(effect);
        }

        let script = ActionScript {
            source,
            program: Some(program),
            attached_macro,
        };

        std::thread::spawn(move || {
            // 等 Accessory 切换与目标应用重新置前
            std::thread::sleep(Duration::from_millis(280));
            match crate::action::play_script_try_run(script, move |result| {
                dispatch::Queue::main().exec_async(move || {
                    finish_try_run_script(restore_script, restore_panel, restore_settings, result);
                });
            }) {
                Ok(()) => {}
                Err(err) => {
                    dispatch::Queue::main().exec_async(move || {
                        finish_try_run_script(
                            restore_script,
                            restore_panel,
                            restore_settings,
                            Err(err),
                        );
                    });
                }
            }
        });
    }

    fn poll_script_click_capture(&mut self) {
        let Some(recorder) = &self.script_click_recorder else {
            return;
        };
        let Some(outcome) = recorder.poll_outcome() else {
            return;
        };
        self.script_click_recorder = None;
        match outcome {
            MacroRecordOutcome::CaptureOnce { x, y } => {
                let exclude_pid = Some(std::process::id());
                let result = simplify_capture_once(x, y, exclude_pid);
                let (nx, ny) = result
                    .steps
                    .iter()
                    .find_map(|s| match s {
                        crate::config::MacroStep::Click { x, y, .. } => Some((*x, *y)),
                        _ => None,
                    })
                    .unwrap_or((0.5, 0.5));
                self.script_editor.insert_script_click(nx, ny);
            }
            MacroRecordOutcome::Cancelled => {
                if self.script_editor.is_macro_recording() {
                    self.script_editor.cancel_macro_recording();
                } else {
                    self.script_editor.cancel_script_capture();
                }
            }
            MacroRecordOutcome::Sequence { events } => {
                let exclude_pid = Some(std::process::id());
                let result = simplify_events(&events, exclude_pid);
                if result.steps.is_empty() {
                    self.script_editor.cancel_macro_recording();
                    return;
                }
                let macro_ = MouseMacro {
                    steps: result.steps,
                    pre_delay_ms: MouseMacro::DEFAULT_PRE_DELAY_MS,
                    recorded_bundle_id: result.recorded_bundle_id,
                };
                if let Some(session) = self.button_edit_session.as_mut() {
                    self.script_editor.append_macro_script(session, &macro_);
                }
            }
        }
    }

    fn poll_app_picker(&mut self) {
        let Some(picker) = &self.app_picker else {
            return;
        };
        let picking = picker.is_picking();
        let outcome = picker.poll();
        if picking && !self.button_editor_any_visible() {
            if let Some(picker) = &self.app_picker {
                picker.cancel();
            }
            self.appearance_editor.set_app_picking(false);
            self.script_editor.set_script_app_picking(false);
            self.script_editor.set_script_open_app_picking(false);
        }
        let Some(outcome) = outcome else {
            return;
        };
        match outcome {
            PickOutcome::Picked {
                bundle_id,
                app_name,
            } => {
                if self.appearance_editor.is_app_picking() {
                    self.appearance_editor
                        .apply_app_pick(bundle_id, app_name);
                } else {
                    self.script_editor.apply_app_pick(bundle_id, app_name);
                }
            }
            PickOutcome::Cancelled => {
                self.appearance_editor.set_app_picking(false);
                self.script_editor.set_script_app_picking(false);
                self.script_editor.set_script_open_app_picking(false);
            }
        }
    }

    fn sync_panel_if_needed(&mut self) {
        let visible = self.core.visible;
        if !visible && !self.panel_sync_dirty {
            self.track_panel_hidden();
            return;
        }

        let was_visible = self.panel.is_last_visible();
        let buttons = self.core.display_buttons();
        let icon_dirs = &self.core.icon_dirs;
        let panel_cfg = &self.core.panel;
        let expanded = self.collapsed_ui.expanded;
        let pinned = self.core.pinned;
        if let Some(pos) = self.panel.sync(
            self.mtm, panel_cfg, buttons, icon_dirs, visible, expanded, pinned,
        ) {
            self.core.panel.position.x = pos.x;
            self.core.panel.position.y = pos.y;
            self.core.app_config.global_panel.position.x = pos.x;
            self.core.app_config.global_panel.position.y = pos.y;
            self.schedule_position_save();
        }
        self.panel_sync_dirty = false;

        self.window_watch.set_panel_visible(visible);

        if visible {
            self.panel_hidden_since = None;
        } else if was_visible != Some(false) {
            self.panel.clear_icon_cache();
            self.panel_hidden_since = Some(Instant::now());
        } else {
            self.track_panel_hidden();
        }
        self.window_watch.set_panel_visible(visible);
    }

    fn track_panel_hidden(&mut self) {
        if self.panel_hidden_since.is_none() {
            self.panel_hidden_since = Some(Instant::now());
        }
    }

    fn maybe_idle_memory_purge(&mut self) {
        if self.core.visible {
            return;
        }
        let Some(hidden_since) = self.panel_hidden_since else {
            return;
        };
        if hidden_since.elapsed() < HIDDEN_IDLE_PURGE_AFTER {
            return;
        }
        if self.last_idle_purge.elapsed() < PURGE_MIN_INTERVAL {
            return;
        }
        self.last_idle_purge = Instant::now();
        memory::purge_allocator();
        log::info!("idle memory purge (panel hidden {:.0}s)", hidden_since.elapsed().as_secs_f64());
    }

    #[allow(dead_code)]
    fn sync_panel(&mut self) {
        self.mark_panel_dirty();
        self.sync_panel_if_needed();
    }

    fn update_collapsed_hover(&mut self) {
        if !self.core.is_collapsed_mode() || !self.core.visible {
            self.clear_launcher_drag();
            self.collapsed_ui.left_was_down = false;
            self.panel.set_ignores_mouse_events(false);
            return;
        }

        let buttons_down = unsafe { NSEvent::pressedMouseButtons() };
        let left_down = (buttons_down & 1) != 0;
        let was_expanded = self.collapsed_ui.expanded;
        let Some((mx, my)) = global_mouse_pos() else {
            return;
        };
        let over_launcher = self.panel.pointer_over_launcher(self.mtm, mx, my);

        // 几何拖动：位置跟手由 NSEvent monitor 驱动；tick 只负责收尾 / 兜底
        if self.collapsed_ui.drag.is_some() {
            if left_down {
                self.apply_launcher_drag_at(mx, my);
                self.collapsed_ui.left_was_down = true;
                self.panel.set_ignores_mouse_events(false);
                return;
            }
            self.clear_launcher_drag();
        }

        if left_down && !self.collapsed_ui.left_was_down && over_launcher {
            let interval = Duration::from_secs_f64(unsafe { NSEvent::doubleClickInterval() });
            let now = Instant::now();
            let is_double = self
                .collapsed_ui
                .last_launcher_click
                .is_some_and(|t| now.duration_since(t) <= interval);
            if is_double {
                self.collapsed_ui.last_launcher_click = None;
                self.collapsed_ui.left_was_down = true;
                self.set_collapsed_mode(false);
                return;
            }
            self.collapsed_ui.last_launcher_click = Some(now);
            let (ox, oy) = self.panel.top_left(self.mtm);
            self.collapsed_ui.drag = Some(LauncherDrag {
                grab_x: mx,
                grab_y: my,
                origin_x: ox,
                origin_y: oy,
                moved: false,
            });
            self.start_launcher_drag_monitors();
            self.collapsed_ui.left_was_down = true;
            self.panel.set_ignores_mouse_events(false);
            return;
        }

        self.collapsed_ui.left_was_down = left_down;

        // 其它按键按下时禁止切换展开/穿透
        if buttons_down != 0 {
            self.panel.set_ignores_mouse_events(false);
            return;
        }

        let over_content = if self.collapsed_ui.expanded {
            self.panel.pointer_over_collapsed_content(self.mtm, mx, my)
        } else {
            over_launcher
        };

        if over_launcher || (self.collapsed_ui.expanded && over_content) {
            if !self.collapsed_ui.expanded {
                self.collapsed_ui.expanded = true;
                self.collapsed_ui.collapse_after = None;
            } else {
                self.collapsed_ui.collapse_after = None;
            }
        } else if self.collapsed_ui.expanded {
            if self.collapsed_ui.collapse_after.is_none() {
                self.collapsed_ui.collapse_after = Some(Instant::now() + COLLAPSE_DELAY);
            } else if Instant::now() >= self.collapsed_ui.collapse_after.unwrap() {
                self.collapsed_ui.expanded = false;
                self.collapsed_ui.collapse_after = None;
            }
        }

        // 收起且未悬停桌宠：穿透，避免挡桌面
        let passthrough = !self.collapsed_ui.expanded && !over_launcher;
        self.panel.set_ignores_mouse_events(passthrough);

        if self.collapsed_ui.expanded != was_expanded {
            self.mark_panel_dirty();
        }
    }

    fn apply_launcher_drag_at(&mut self, mx: f32, my: f32) {
        let Some(mut drag) = self.collapsed_ui.drag else {
            return;
        };
        let dx = mx - drag.grab_x;
        let dy = my - drag.grab_y;
        if !drag.moved && dx.hypot(dy) >= LAUNCHER_DRAG_THRESHOLD {
            drag.moved = true;
        }
        if drag.moved {
            let nx = drag.origin_x + dx as f64;
            let ny = drag.origin_y + dy as f64;
            self.panel.move_top_left(self.mtm, nx, ny);
            self.core.panel.position.x = nx as f32;
            self.core.panel.position.y = ny as f32;
            self.core.app_config.global_panel.position.x = nx as f32;
            self.core.app_config.global_panel.position.y = ny as f32;
            self.collapsed_ui.last_launcher_click = None;
        }
        self.collapsed_ui.drag = Some(drag);
    }

    fn follow_launcher_drag_event(&mut self) {
        if self.collapsed_ui.drag.is_none() {
            return;
        }
        let left_down = (unsafe { NSEvent::pressedMouseButtons() } & 1) != 0;
        if !left_down {
            self.clear_launcher_drag();
            self.collapsed_ui.left_was_down = false;
            return;
        }
        let Some((mx, my)) = global_mouse_pos() else {
            return;
        };
        self.apply_launcher_drag_at(mx, my);
        self.panel.set_ignores_mouse_events(false);
    }

    fn start_launcher_drag_monitors(&mut self) {
        if self.launcher_drag_monitors.is_some() {
            return;
        }
        let mask = NSEventMask::LeftMouseDragged | NSEventMask::LeftMouseUp;
        let global_handler = RcBlock::new(|_event: NonNull<NSEvent>| {
            on_launcher_drag_monitor_event();
        });
        let local_handler = RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
            on_launcher_drag_monitor_event();
            event.as_ptr()
        });
        let global = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, &global_handler);
        let local =
            unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &local_handler) };
        if let (Some(global), Some(local)) = (global, local) {
            self.launcher_drag_monitors = Some(LauncherDragMonitors { global, local });
        }
    }

    fn stop_launcher_drag_monitors(&mut self) {
        if let Some(monitors) = self.launcher_drag_monitors.take() {
            unsafe {
                NSEvent::removeMonitor(&monitors.global);
                NSEvent::removeMonitor(&monitors.local);
            }
        }
    }

    fn clear_launcher_drag(&mut self) {
        let moved = self
            .collapsed_ui
            .drag
            .as_ref()
            .is_some_and(|drag| drag.moved);
        self.collapsed_ui.drag = None;
        self.stop_launcher_drag_monitors();
        if moved {
            self.save_panel_position_now();
        }
    }

    fn schedule_position_save(&mut self) {
        self.pending_position_save = Some(Instant::now() + POSITION_SAVE_DEBOUNCE);
    }

    fn flush_pending_position_save(&mut self, force: bool) {
        let due = self.pending_position_save.is_some_and(|deadline| {
            force || Instant::now() >= deadline
        });
        if !due {
            return;
        }
        self.pending_position_save = None;
        self.save_panel_position_now();
    }

    fn save_panel_position_now(&mut self) {
        let pos = &self.core.app_config.global_panel.position;
        if let Err(err) = save_panel_position(&self.core.config_path, pos.x, pos.y) {
            log::error!("save position failed: {err}");
        }
    }

    fn set_collapsed_mode(&mut self, collapsed: bool) {
        self.clear_launcher_drag();
        let effect = self.core.set_collapsed_enabled(collapsed);
        self.collapsed_ui = CollapsedUiState::default();
        self.panel.set_ignores_mouse_events(false);
        if let Err(err) = save_app_config(&self.core.config_path, &self.core.app_config) {
            log::error!("save collapsed mode failed: {err}");
        }
        // 切折叠时尽量保持可见，方便立刻看到桌宠
        if collapsed {
            let _ = self.core.handle(CoreCommand::ShowPanel);
        }
        self.apply(effect);
        log::info!(
            "{}",
            if collapsed {
                "已切换为折叠模式"
            } else {
                "已切换为完整模式"
            }
        );
    }

    fn poll_ui_events(&mut self) {
        for ev in drain_ui_events(&self.ui_rx) {
            match ev {
                NativeUiEvent::ApplySettings(cfg) => {
                    self.on_apply_settings(cfg);
                }
                NativeUiEvent::CloseSettings => {
                    self.flush_pending_settings_save(true);
                    self.settings.hide();
                }
                NativeUiEvent::OpenConfigFile => {
                    let _ = self.core.handle(CoreCommand::OpenConfigFile);
                }
                NativeUiEvent::ExportConfig(cfg) => {
                    if let Some(result) =
                        run_export_flow(self.mtm, &cfg, &self.core.icon_dirs)
                    {
                        show_export_result(self.mtm, result);
                    }
                }
                NativeUiEvent::ImportConfig => {
                    if let Some(result) = run_import_flow(
                        self.mtm,
                        &self.core.app_config,
                        &self.core.config_path,
                    ) {
                        match result {
                            Ok(imported) => {
                                let cfg = imported.app_config.clone();
                                let effect = self.core.apply_config(cfg);
                                self.core.icon_dirs = crate::config::icon_search_dirs();
                                if let Err(err) =
                                    self.hotkeys.replace(&self.core.app_config.hotkeys)
                                {
                                    log::warn!("re-register hotkeys after import: {err}");
                                }
                                self.apply(effect);
                                if self.settings.is_visible() {
                                    self.settings.reload_from_config(&self.core.app_config);
                                }
                                show_import_result(self.mtm, Ok(imported));
                            }
                            Err(err) => show_import_result(self.mtm, Err(err)),
                        }
                    }
                }
                NativeUiEvent::ViewBackups => {
                    if let Some(result) =
                        run_backup_restore_flow(self.mtm, &self.core.config_path)
                    {
                        match result {
                            Ok(cfg) => {
                                let effect = self.core.apply_config(cfg);
                                if let Err(err) =
                                    self.hotkeys.replace(&self.core.app_config.hotkeys)
                                {
                                    log::warn!("re-register hotkeys after backup restore: {err}");
                                }
                                self.apply(effect);
                                if self.settings.is_visible() {
                                    self.settings.reload_from_config(&self.core.app_config);
                                }
                                show_backup_restore_result(self.mtm, Ok(()));
                            }
                            Err(err) => show_backup_restore_result(self.mtm, Err(err)),
                        }
                    }
                }
                NativeUiEvent::OpenPermissions => self.permissions.show(),
                NativeUiEvent::OpenButtonEditor { profile, button, script } => {
                    self.open_button_editor(profile, button, script);
                }
                NativeUiEvent::SaveButton {
                    profile,
                    button,
                    config,
                } => {
                    if let Some(session) = self.button_edit_session.as_mut() {
                        if session.matches(profile, button) {
                            session.apply_saved(config.clone());
                        }
                    }
                    let effect = self.core.update_profile_button(profile, button, config);
                    if let Err(err) = save_app_config(&self.core.config_path, &self.core.app_config)
                    {
                        log::error!("save button failed: {err}");
                    }
                    self.apply(effect);
                    self.cancel_app_pick();
                    self.script_editor.hide();
                    self.appearance_editor.hide();
                    if !self.button_editor_any_visible() {
                        self.button_edit_session = None;
                    }
                    if self.settings.is_visible() {
                        let cfg = self.core.app_config.clone();
                        self.settings.reload_from_config(&cfg);
                    }
                }
                NativeUiEvent::CancelButtonEditor => {
                    self.cancel_app_pick();
                    self.stop_script_click_capture();
                    self.stop_script_macro_record();
                    self.close_button_editors();
                }
                NativeUiEvent::TryRunScript {
                    source,
                    attached_macro,
                } => {
                    self.begin_try_run_script(source, attached_macro);
                }
                NativeUiEvent::StartScriptClickCapture => {
                    self.start_script_click_capture();
                }
                NativeUiEvent::CancelScriptClickCapture => {
                    self.stop_script_click_capture();
                }
                NativeUiEvent::StartScriptMacroRecord => {
                    self.start_script_macro_record();
                }
                NativeUiEvent::FinishScriptMacroRecord => {
                    self.finish_script_macro_record();
                }
                NativeUiEvent::StartAppPick => self.start_app_pick(),
                NativeUiEvent::CancelAppPick => self.cancel_app_pick(),
                NativeUiEvent::OpenIconPicker(target) => {
                    let dirs = self.core.icon_dirs.clone();
                    self.icon_picker.open(&dirs, target);
                }
                NativeUiEvent::IconPicked { target, name } => match target {
                    IconPickTarget::CollapsedIcon => {
                        if self.settings.is_visible() {
                            self.settings.apply_collapsed_icon(&name);
                        } else {
                            self.core.app_config.global_panel.settings.collapsed_icon =
                                Some(name.clone());
                            if let Err(err) =
                                save_app_config(&self.core.config_path, &self.core.app_config)
                            {
                                log::error!("save collapsed icon failed: {err}");
                            }
                            let cfg = self.core.app_config.clone();
                            let _ = self.core.apply_config(cfg);
                        }
                    }
                    IconPickTarget::Button { profile, button } => {
                        if self.appearance_editor.is_visible()
                            && self
                                .button_edit_session
                                .as_ref()
                                .is_some_and(|s| s.matches(profile, button))
                        {
                            self.appearance_editor.set_icon_name(&name);
                        }
                    }
                },
                NativeUiEvent::CancelIconPicker => self.icon_picker.hide(),
                NativeUiEvent::RelaunchForPermissions => {
                    relaunch_app();
                }
                NativeUiEvent::ButtonMenu { index, action } => {
                    self.handle_button_menu(index, action);
                }
                NativeUiEvent::EnterCollapsedMode => self.set_collapsed_mode(true),
                NativeUiEvent::ExitCollapsedMode => self.set_collapsed_mode(false),
                NativeUiEvent::LauncherInteract => {
                    if self.core.is_collapsed_mode() {
                        self.collapsed_ui.expanded = true;
                        self.collapsed_ui.collapse_after = None;
                        self.panel.set_ignores_mouse_events(false);
                    }
                }
                NativeUiEvent::LauncherMenu(action) => match action {
                    LauncherMenuAction::TogglePin => {
                        self.core.toggle_pinned();
                    }
                    LauncherMenuAction::ExpandToFull => self.set_collapsed_mode(false),
                    LauncherMenuAction::Settings => self.open_settings(),
                    LauncherMenuAction::Quit => {
                        let _ = self.core.handle(CoreCommand::Quit);
                    }
                },
            }
        }
    }

    fn handle_button_menu(&mut self, index: usize, action: ButtonMenuAction) {
        let buttons = self.core.display_buttons().to_vec();
        let Some(mut button) = buttons.get(index).cloned() else {
            return;
        };
        match action {
            ButtonMenuAction::Edit => {
                let profile = self.core.current_profile_index().unwrap_or(0);
                self.open_button_editor(profile, index, true);
            }
            ButtonMenuAction::EditAppearance => {
                let profile = self.core.current_profile_index().unwrap_or(0);
                self.open_button_editor(profile, index, false);
            }
            ButtonMenuAction::Settings => self.open_settings(),
            ButtonMenuAction::Quit => {
                let _ = self.core.handle(CoreCommand::Quit);
            }
            ButtonMenuAction::Enable => {
                button.enabled = true;
                self.save_slot_button(index, button);
            }
            ButtonMenuAction::Disable => {
                button.enabled = false;
                self.save_slot_button(index, button);
            }
            ButtonMenuAction::ClearAction => {
                button.action = crate::config::Action::None;
                button.enabled = false;
                self.save_slot_button(index, button);
            }
            ButtonMenuAction::Delete => {
                self.save_slot_button(index, crate::config::empty_slot_button());
            }
        }
    }

    fn save_slot_button(&mut self, index: usize, button: crate::config::ButtonConfig) {
        let effect = self.core.update_current_profile_button(index, button);
        if let Err(err) = save_app_config(&self.core.config_path, &self.core.app_config) {
            log::error!("save button from context menu failed: {err}");
        }
        self.apply(effect);
        if self.settings.is_visible() {
            let cfg = self.core.app_config.clone();
            self.settings.reload_from_config(&cfg);
        }
    }

    fn maybe_prompt_permissions(&mut self) {
        if self.permissions_prompted {
            return;
        }
        self.permissions_prompted = true;
        let status = check_permissions();
        if status.all_granted() {
            log::info!("permissions ok (accessibility + input monitoring)");
            return;
        }
        log::warn!(
            "missing permissions (ax={}, input={}), showing guide",
            status.accessibility,
            status.input_monitoring
        );
        self.permissions.show();
    }

    fn poll_hotkeys(&mut self) {
        while let Some(action) = self.hotkeys.poll() {
            let cmd = match action {
                HotkeyAction::ToggleVisibility => CoreCommand::ToggleVisibility,
                HotkeyAction::ReloadConfig => CoreCommand::ReloadConfig,
                HotkeyAction::Quit => CoreCommand::Quit,
                HotkeyAction::OpenSettings => CoreCommand::OpenSettings,
                HotkeyAction::TriggerButton(i) => CoreCommand::TriggerButton(i),
            };
            let effect = self.core.handle(cmd);
            self.apply(effect);
        }
    }

    fn poll_tray(&mut self) {
        let actions: Vec<_> = {
            let Some(tray) = self.tray.as_ref() else {
                return;
            };
            let mut out = Vec::new();
            while let Some(action) = tray.poll() {
                out.push(action);
            }
            out
        };
        for action in actions {
            let cmd = match action {
                TrayAction::TogglePanel => CoreCommand::ToggleVisibility,
                TrayAction::OpenSettings => CoreCommand::OpenSettings,
                TrayAction::Quit => CoreCommand::Quit,
            };
            let effect = self.core.handle(cmd);
            self.apply(effect);
        }
    }

    fn poll_gestures(&mut self) {
        let Some(monitor) = self.mouse_gesture.as_mut() else {
            return;
        };
        if self.app_picker.as_ref().is_some_and(|p| p.is_picking()) {
            let mut cfg = self.core.app_config.gesture.clone();
            cfg.enabled = false;
            monitor.update_config(&cfg);
            let _ = monitor.poll();
            return;
        }
        monitor.update_config(&self.core.app_config.gesture);
        monitor.set_dismiss_armed(
            self.core.visible && self.core.gesture_session && !self.core.pinned,
        );
        for action in monitor.poll() {
            match action {
                MouseGestureAction::ShowAt { x, y } => {
                    let effect = self.core.handle(CoreCommand::ShowPanelAt { x, y });
                    self.apply(effect);
                }
                MouseGestureAction::LeftMouseDown { .. } => {
                    if self.core.gesture_session && self.core.visible && !self.core.pinned {
                        let effect = self.core.handle(CoreCommand::HidePanel);
                        self.apply(effect);
                    }
                }
            }
        }
    }

    fn poll_button_clicks(&mut self) {
        for index in drain_button_clicks(self.panel.click_receiver()) {
            let effect = self.core.handle(CoreCommand::TriggerButton(index));
            // 折叠模式：点完按钮只收起网格，猫头继续留在桌面（钉住时尤其如此）
            if self.core.is_collapsed_mode() && self.core.visible {
                self.collapsed_ui.expanded = false;
                self.collapsed_ui.collapse_after = None;
            }
            self.apply(effect);
        }
    }

    fn poll_config(&mut self) {
        let mut dirty = false;
        while self.config_rx.try_recv().is_ok() {
            dirty = true;
        }
        if dirty {
            if self.should_ignore_config_watcher() {
                return;
            }
            let effect = self.core.handle(CoreCommand::ReloadConfig);
            self.apply(effect);
            if let Err(err) = self.hotkeys.replace(&self.core.app_config.hotkeys) {
                log::warn!("re-register hotkeys after reload: {err}");
            }
            if self.settings.is_visible() {
                let cfg = self.core.app_config.clone();
                self.settings.reload_from_config(&cfg);
            }
        }
    }

    fn poll_active_window(&mut self) {
        while let Ok(w) = self.window_rx.try_recv() {
            let effect = self.core.handle(CoreCommand::ActiveWindow(
                Some(w).filter(|w| !w.is_own_process()),
            ));
            self.apply(effect);
        }
    }
}

thread_local! {
    static STATE: RefCell<Option<NativeState>> = const { RefCell::new(None) };
}

#[derive(Default)]
struct AppDelegateIvars {
    _timer: RefCell<Option<Retained<NSTimer>>>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "MeoToolNativeAppDelegate"]
    #[ivars = AppDelegateIvars]
    struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &NSNotification) {
            let mtm = self.mtm();
            let app = NSApplication::sharedApplication(mtm);
            app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
            install_standard_main_menu(mtm);

            let timer = unsafe {
                NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                    0.05,
                    self,
                    objc2::sel!(meotoolTick:),
                    None,
                    true,
                )
            };
            *self.ivars()._timer.borrow_mut() = Some(timer);
            log::info!("native UI ready (settings/editor/permissions wired)");

            // 等 RunLoop 就绪后再弹权限窗（Accessory 下立即 makeKey 常不可见）
            dispatch::Queue::main().exec_after(std::time::Duration::from_millis(250), || {
                STATE.with(|slot| {
                    if let Ok(mut guard) = slot.try_borrow_mut() {
                        if let Some(state) = guard.as_mut() {
                            state.maybe_prompt_permissions();
                        }
                    }
                });
            });
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window_closed(&self, _sender: &NSApplication) -> bool {
            false
        }
    }

    impl AppDelegate {
        #[unsafe(method(meotoolTick:))]
        fn meotool_tick(&self, _timer: &NSTimer) {
            let outcome = run_tick();
            if matches!(outcome, TickOutcome::Quit) {
                if let Some(timer) = self.ivars()._timer.borrow_mut().take() {
                    timer.invalidate();
                }
                let mtm = self.mtm();
                unsafe {
                    NSApplication::sharedApplication(mtm).terminate(None);
                }
            }
        }
    }
);

impl AppDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(AppDelegateIvars::default());
        unsafe { msg_send![super(this), init] }
    }
}

fn run_tick() -> TickOutcome {
    STATE.with(|slot| match slot.try_borrow_mut() {
        Ok(mut guard) => guard
            .as_mut()
            .map(NativeState::tick)
            .unwrap_or(TickOutcome::Continue),
        // terminate / 权限回调嵌套进 RunLoop 时可能重入；跳过避免 RefCell panic
        Err(_) => TickOutcome::Continue,
    })
}

fn on_launcher_drag_monitor_event() {
    STATE.with(|slot| {
        let Ok(mut guard) = slot.try_borrow_mut() else {
            return;
        };
        if let Some(state) = guard.as_mut() {
            state.follow_launcher_drag_event();
        }
    });
}

pub fn run_native() -> Result<(), String> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cli = crate::config::parse_cli_args();
    if cli.show_help {
        return crate::config::run_cli(&cli);
    }
    if cli.command.is_some() {
        return crate::config::run_cli(&cli);
    }

    check_accessibility_hint();

    let config_path = parse_config_arg();
    let loaded = load_config(config_path.as_deref());

    let mtm = MainThreadMarker::new().ok_or("native UI must run on main thread")?;
    let app = NSApplication::sharedApplication(mtm);

    let wake = WakeHandle::new();
    wake.set_callback(|| {
        dispatch::Queue::main().exec_async(|| {
            if matches!(run_tick(), TickOutcome::Quit) {
                let mtm = MainThreadMarker::new().expect("main thread");
                unsafe {
                    NSApplication::sharedApplication(mtm).terminate(None);
                }
            }
        });
    });

    let ui_rx = install_ui_channel();
    let core = AppCore::from_loaded(&loaded);
    let hotkeys = HotkeyManager::register(&core.app_config.hotkeys, wake.clone())
        .map_err(|e| format!("hotkeys: {e}"))?;
    let mouse_gesture = MouseGestureMonitor::new(&core.app_config.gesture, wake.clone())
        .map_err(|e| {
            log::warn!("mouse gesture unavailable: {e}");
            e
        })
        .ok();
    let app_picker = AppClickPicker::new(wake.clone())
        .map_err(|e| {
            log::warn!("app click picker unavailable: {e}");
            e
        })
        .ok();

    let tray_icon = core
        .panel
        .collapsed_icon
        .as_deref()
        .unwrap_or(crate::config::PanelConfig::DEFAULT_COLLAPSED_ICON);
    let tray = TrayManager::install(wake.clone(), &core.icon_dirs, tray_icon)
        .map_err(|e| {
            log::warn!("tray unavailable: {e}");
            e
        })
        .ok();

    let poll_ms = core.app_config.poll_interval_ms;
    let window_watch = WindowWatcherControl::new(poll_ms);
    let (window_tx, window_rx) = mpsc::channel::<WindowInfo>();
    let _watcher = spawn_watcher(window_tx, window_watch.clone());

    let (config_tx, config_rx) = mpsc::channel::<()>();
    let config_watcher = spawn_config_watcher(core.config_path.clone(), config_tx).ok();

    let panel = NativePanel::new(mtm);
    let settings = SettingsWindow::new(mtm, &core.app_config);
    let script_editor = ScriptEditorWindow::new(mtm);
    let appearance_editor = AppearanceEditorWindow::new(mtm);
    let icon_picker = IconPickerWindow::new(mtm);
    let permissions = PermissionsWindow::new(mtm);

    let mut state = NativeState {
        core,
        panel,
        settings,
        script_editor,
        appearance_editor,
        button_edit_session: None,
        icon_picker,
        permissions,
        ui_rx,
        hotkeys,
        mouse_gesture,
        app_picker,
        tray,
        window_rx,
        config_rx,
        _config_watcher: config_watcher,
        mtm,
        permissions_prompted: false,
        terminating: false,
        collapsed_ui: CollapsedUiState::default(),
        launcher_drag_monitors: None,
        memory: MemoryMonitor::default(),
        panel_sync_dirty: true,
        panel_hidden_since: None,
        last_idle_purge: Instant::now(),
        window_watch,
        pending_position_save: None,
        pending_settings_save: None,
        last_settings_self_write: None,
        script_click_recorder: None,
    };
    state.sync_panel_if_needed();
    STATE.with(|slot| *slot.borrow_mut() = Some(state));

    let delegate = AppDelegate::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    std::mem::forget(delegate);

    log::info!("entering NSApplication run loop (ui-native phase 2)");
    if let (Some(rss), Some(phys)) = (
        crate::app::memory::read_rss_mb(),
        crate::app::memory::read_phys_mb(),
    ) {
        log::info!(
            "startup memory ui=native rss_mb={rss:.1} phys_mb={phys:.1}"
        );
    } else {
        log::info!("startup memory ui=native");
    }
    app.run();
    Ok(())
}

fn show_try_run_alert(mtm: MainThreadMarker, message: &str) {
    use objc2_app_kit::NSAlert;
    use objc2_foundation::NSString;
    unsafe {
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str("试运行失败"));
        alert.setInformativeText(&NSString::from_str(message));
        alert.addButtonWithTitle(&NSString::from_str("好"));
        let _ = alert.runModal();
    }
}

fn finish_try_run_script(
    restore_script: bool,
    restore_panel: bool,
    restore_settings: bool,
    result: Result<(), String>,
) {
    let mut alert: Option<(MainThreadMarker, String)> = None;
    STATE.with(|slot| {
        let Ok(mut guard) = slot.try_borrow_mut() else {
            return;
        };
        let Some(state) = guard.as_mut() else {
            return;
        };

        if restore_panel {
            let effect = state.core.handle(CoreCommand::ShowPanel);
            state.apply(effect);
        }
        if restore_settings {
            state.settings.show();
        }
        if restore_script && state.button_edit_session.is_some() {
            state.script_editor.restore_after_try_run();
            match &result {
                Ok(()) => state
                    .script_editor
                    .set_try_run_status("状态：试运行完成"),
                Err(err) => {
                    let short = err.replace('\n', " ");
                    state
                        .script_editor
                        .set_try_run_status(&format!("状态：试运行失败 — {short}"));
                }
            }
        }

        if let Err(err) = result {
            log::warn!("try-run script failed: {err}");
            alert = Some((state.mtm, err));
        }
    });
    if let Some((mtm, err)) = alert {
        show_try_run_alert(mtm, &err);
    }
}

fn parse_config_arg() -> Option<std::path::PathBuf> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--config" {
            return args.next().map(std::path::PathBuf::from);
        }
        if let Some(path) = arg.strip_prefix("--config=") {
            return Some(std::path::PathBuf::from(path));
        }
    }
    None
}

fn load_config(cli_path: Option<&std::path::Path>) -> LoadedConfig {
    LoadedConfig::load(cli_path).unwrap_or_else(|e| panic!("failed to load config: {e}"))
}
