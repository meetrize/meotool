//! 原生设置窗口：侧栏导航 + 滚动分组表单（即改即生效，无保存底栏）。

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, ClassType, MainThreadOnly};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSApplication, NSBackingStoreType, NSBox,
    NSBoxType, NSButton, NSColor, NSColorSpace, NSColorWell, NSControlStateValueOff,
    NSControlStateValueOn, NSEvent, NSEventMask, NSEventModifierFlags, NSLayoutConstraint,
    NSPopUpButton, NSScrollView, NSSlider, NSStackView, NSSwitch, NSTextField, NSTextView, NSView,
    NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize,
    NSString,
};

use crate::config::color::{color_to_string, parse_color};
use crate::config::{
    empty_slot_button, normalize_app_config, Action, AppConfig, ButtonConfig, ButtonScope,
    ContextMode, DisplayMode, GlobalHotkeys, MatchRule, PanelSettings, Profile, ThemeSettings,
};
use crate::platform::{
    check_permissions, executable_path, open_accessibility_settings, open_input_monitoring_settings,
};

use super::events::{emit, IconPickTarget, NativeUiEvent};
use super::layout::{
    add_arranged, field_string, help_label, h_stack, install_page, install_pref_sidebar_chrome,
    labeled_field, labeled_popup, labeled_switch, layout_pref_sidebar_chrome, make_button,
    section_title, scroll_document_to_top, set_field, set_switch, sidebar_io_button,
    sidebar_nav_button, switch_on, sync_page_height, v_stack, LABEL_W,
};

const NAV_TITLES: &[&str] = &["通用", "外观", "快捷键", "配置档", "权限"];
const WINDOW_W: f64 = 840.0;
const WINDOW_H: f64 = 680.0;
const UNDO_DEPTH: usize = 20;
const ROWS_COLS_MIN: f64 = 1.0;
const ROWS_COLS_MAX: f64 = 9.0;
const BTN_SIZE_MIN: f64 = 32.0;
const BTN_SIZE_MAX: f64 = 128.0;

#[derive(Default)]
struct SettingsTargetIvars;

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "MeoToolSettingsTarget"]
    #[ivars = SettingsTargetIvars]
    struct SettingsTarget;

    unsafe impl NSObjectProtocol for SettingsTarget {}

    unsafe impl NSWindowDelegate for SettingsTarget {
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, _sender: &NSWindow) -> bool {
            emit_tag(SettingsAction::CloseAttempt);
            false
        }

        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, _notification: &NSNotification) {
            // live resize 时 RunLoop 可能不跑定时器，必须同步布局，避免旧 frame / 约束崩溃
            layout_settings_chrome_now();
            emit_tag(SettingsAction::WindowResized);
        }
    }

    impl SettingsTarget {
        #[unsafe(method(openFileClicked:))]
        fn open_file_clicked(&self, _sender: &AnyObject) {
            emit(NativeUiEvent::OpenConfigFile);
        }
        #[unsafe(method(exportClicked:))]
        fn export_clicked(&self, _sender: &AnyObject) {
            emit_tag(SettingsAction::Export);
        }
        #[unsafe(method(importClicked:))]
        fn import_clicked(&self, _sender: &AnyObject) {
            emit_tag(SettingsAction::Import);
        }
        #[unsafe(method(backupsClicked:))]
        fn backups_clicked(&self, _sender: &AnyObject) {
            emit_tag(SettingsAction::ViewBackups);
        }
        #[unsafe(method(navClicked:))]
        fn nav_clicked(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            if (0..NAV_TITLES.len() as isize).contains(&tag) {
                emit_tag(SettingsAction::Nav(tag as usize));
            }
        }
        #[unsafe(method(fieldChanged:))]
        fn field_changed(&self, _sender: &AnyObject) {
            emit_tag(SettingsAction::VisualChanged);
        }
        #[unsafe(method(profileChanged:))]
        fn profile_changed(&self, _sender: &AnyObject) {
            emit_tag(SettingsAction::ProfileChanged);
        }
        #[unsafe(method(addProfileClicked:))]
        fn add_profile(&self, _sender: &AnyObject) {
            emit_tag(SettingsAction::AddProfile);
        }
        #[unsafe(method(deleteProfileClicked:))]
        fn delete_profile(&self, _sender: &AnyObject) {
            emit_tag(SettingsAction::DeleteProfile);
        }
        #[unsafe(method(editButtonClicked:))]
        fn edit_button(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            if tag >= 0 {
                emit_tag(SettingsAction::EditButton(tag as usize));
            }
        }
        #[unsafe(method(pickCollapsedIcon:))]
        fn pick_collapsed(&self, _sender: &AnyObject) {
            emit(NativeUiEvent::OpenIconPicker(IconPickTarget::CollapsedIcon));
        }
        #[unsafe(method(clearProfileTheme:))]
        fn clear_profile_theme(&self, _sender: &AnyObject) {
            emit_tag(SettingsAction::ClearProfileTheme);
        }
        #[unsafe(method(recordHotkey:))]
        fn record_hotkey(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            if (0..4).contains(&tag) {
                emit_tag(SettingsAction::StartRecord(tag as usize));
            }
        }
        #[unsafe(method(refreshPerms:))]
        fn refresh_perms(&self, _sender: &AnyObject) {
            emit_tag(SettingsAction::RefreshPerms);
        }
        #[unsafe(method(openAx:))]
        fn open_ax(&self, _sender: &AnyObject) {
            open_accessibility_settings();
        }
        #[unsafe(method(openInput:))]
        fn open_input(&self, _sender: &AnyObject) {
            open_input_monitoring_settings();
        }
        #[unsafe(method(relaunchPerms:))]
        fn relaunch_perms(&self, _sender: &AnyObject) {
            emit(NativeUiEvent::RelaunchForPermissions);
        }
    }
);

impl SettingsTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SettingsTargetIvars);
        unsafe { msg_send![super(this), init] }
    }
}

#[derive(Debug, Clone)]
enum SettingsAction {
    CloseAttempt,
    Undo,
    Nav(usize),
    VisualChanged,
    ProfileChanged,
    AddProfile,
    DeleteProfile,
    EditButton(usize),
    ClearProfileTheme,
    StartRecord(usize),
    HotkeyCaptured(usize, String),
    CancelRecord,
    RefreshPerms,
    Export,
    Import,
    ViewBackups,
    WindowResized,
}

static SETTINGS_ACTION_TX: OnceLock<Mutex<Option<Sender<SettingsAction>>>> = OnceLock::new();
static RECORDING_SLOT: OnceLock<Mutex<Option<usize>>> = OnceLock::new();

fn settings_action_tx() -> &'static Mutex<Option<Sender<SettingsAction>>> {
    SETTINGS_ACTION_TX.get_or_init(|| Mutex::new(None))
}

fn recording_slot() -> &'static Mutex<Option<usize>> {
    RECORDING_SLOT.get_or_init(|| Mutex::new(None))
}

fn emit_tag(action: SettingsAction) {
    if let Ok(guard) = settings_action_tx().lock() {
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(action);
        }
    }
}

/// 供 windowDidResize 同步调用的 chrome 引用（仅主线程）。
struct LiveChrome {
    window: Retained<NSWindow>,
    sidebar: Retained<NSView>,
    nav_stack: Retained<NSStackView>,
    io_stack: Retained<NSStackView>,
    scroll: Retained<NSScrollView>,
    page_host: Retained<NSView>,
    page: Retained<NSStackView>,
    saved_label: Retained<NSTextField>,
}

thread_local! {
    static LIVE_CHROME: RefCell<Option<LiveChrome>> = const { RefCell::new(None) };
}

fn set_live_chrome(chrome: Option<LiveChrome>) {
    LIVE_CHROME.with(|slot| *slot.borrow_mut() = chrome);
}

fn layout_settings_chrome_now() {
    LIVE_CHROME.with(|slot| {
        let Ok(guard) = slot.try_borrow() else {
            return;
        };
        let Some(chrome) = guard.as_ref() else {
            return;
        };
        let Some(content) = chrome.window.contentView() else {
            return;
        };
        layout_pref_sidebar_chrome(
            &content,
            &chrome.sidebar,
            &chrome.nav_stack,
            &chrome.io_stack,
            &chrome.scroll,
            &chrome.saved_label,
        );
        sync_page_height(&chrome.page_host, &chrome.page);
    });
}

struct GlobalFields {
    rows: Retained<NSSlider>,
    rows_label: Retained<NSTextField>,
    cols: Retained<NSSlider>,
    cols_label: Retained<NSTextField>,
    display_mode: Retained<NSPopUpButton>,
    collapsed: Retained<NSSwitch>,
    collapsed_icon: Retained<NSTextField>,
    opacity: Retained<NSSlider>,
    opacity_label: Retained<NSTextField>,
    gap: Retained<NSTextField>,
    pos_x: Retained<NSTextField>,
    pos_y: Retained<NSTextField>,
    poll_ms: Retained<NSTextField>,
    context_mode: Retained<NSPopUpButton>,
    status: Retained<NSTextField>,
}

struct AppearanceFields {
    btn_w: Retained<NSSlider>,
    btn_w_label: Retained<NSTextField>,
    btn_h: Retained<NSSlider>,
    btn_h_label: Retained<NSTextField>,
    bg: Retained<NSColorWell>,
    button_bg: Retained<NSColorWell>,
    letter_bg: Retained<NSColorWell>,
    letter_color: Retained<NSColorWell>,
    letter_size: Retained<NSTextField>,
    letter_font: Retained<NSTextField>,
    win_border_on: Retained<NSSwitch>,
    win_border_w: Retained<NSTextField>,
    win_border_color: Retained<NSColorWell>,
    btn_border_on: Retained<NSSwitch>,
    btn_border_w: Retained<NSTextField>,
    btn_border_color: Retained<NSColorWell>,
    profile_popup: Retained<NSPopUpButton>,
    p_btn_w: Retained<NSTextField>,
    p_btn_h: Retained<NSTextField>,
    preview_host: Retained<NSStackView>,
}

struct GestureFields {
    enabled: Retained<NSSwitch>,
    middle_click: Retained<NSSwitch>,
    global_priority: Retained<NSSwitch>,
    threshold: Retained<NSSlider>,
    threshold_label: Retained<NSTextField>,
    dismiss_outside: Retained<NSSwitch>,
    hide_after: Retained<NSSwitch>,
}

struct HotkeyFields {
    labels: [Retained<NSTextField>; 4],
    buttons: [Retained<NSButton>; 4],
}

struct ProfileFields {
    popup: Retained<NSPopUpButton>,
    name: Retained<NSTextField>,
    match_bundle: Retained<NSTextField>,
    match_app: Retained<NSTextField>,
    match_title: Retained<NSTextField>,
    rows: Retained<NSTextField>,
    cols: Retained<NSTextField>,
    button_host: Retained<NSStackView>,
}

struct PermFields {
    status_ax: Retained<NSTextField>,
    status_input: Retained<NSTextField>,
    path_label: Retained<NSTextField>,
}

pub struct SettingsWindow {
    window: Retained<NSWindow>,
    target: Retained<SettingsTarget>,
    nav_buttons: Vec<Retained<NSButton>>,
    pages: [Retained<NSStackView>; 5],
    page_host: Retained<NSView>,
    scroll: Retained<objc2_app_kit::NSScrollView>,
    sidebar: Retained<NSView>,
    nav_stack: Retained<NSStackView>,
    io_stack: Retained<NSStackView>,
    saved_label: Retained<NSTextField>,
    saved_until: Option<Instant>,
    global: GlobalFields,
    appearance: AppearanceFields,
    gesture: GestureFields,
    hotkeys: HotkeyFields,
    profile: ProfileFields,
    perms: PermFields,
    draft: AppConfig,
    /// 最近一次已成功 apply 的配置（撤销回退点）
    last_applied: AppConfig,
    undo_stack: VecDeque<AppConfig>,
    action_rx: Receiver<SettingsAction>,
    current_nav: usize,
    current_profile: usize,
    pending_apply: bool,
    last_apply_at: Option<Instant>,
    key_monitor: Option<Retained<AnyObject>>,
    mtm: MainThreadMarker,
    front_held: bool,
    status_message: Option<String>,
}

impl SettingsWindow {
    pub fn new(mtm: MainThreadMarker, config: &AppConfig) -> Self {
        let (tx, rx) = mpsc::channel();
        *settings_action_tx().lock().unwrap() = Some(tx);

        let target = SettingsTarget::new(mtm);

        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc(),
                NSRect::new(
                    NSPoint::new(120.0, 80.0),
                    NSSize::new(WINDOW_W, WINDOW_H),
                ),
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe {
            window.setReleasedWhenClosed(false);
            window.setTitle(&NSString::from_str("MeoTool 设置"));
            window.setContentMinSize(NSSize::new(720.0, 480.0));
            window.setDelegate(Some(ProtocolObject::from_ref(&*target)));
        }

        let content = window.contentView().expect("content view");
        let chrome = install_pref_sidebar_chrome(mtm, &content, NSSize::new(WINDOW_W, WINDOW_H));

        let mut nav_buttons = Vec::new();
        for (i, title) in NAV_TITLES.iter().enumerate() {
            let btn = sidebar_nav_button(mtm, title, i as isize);
            unsafe {
                btn.setTarget(Some(&target));
                btn.setAction(Some(objc2::sel!(navClicked:)));
            }
            add_arranged(&chrome.nav_stack, &btn);
            nav_buttons.push(btn);
        }

        // 侧栏底部：配置 IO
        let open_file = sidebar_io_button(mtm, "打开配置…");
        let export_btn = sidebar_io_button(mtm, "导出…");
        let import_btn = sidebar_io_button(mtm, "导入…");
        let backups_btn = sidebar_io_button(mtm, "查看备份…");
        unsafe {
            open_file.setTarget(Some(&target));
            open_file.setAction(Some(objc2::sel!(openFileClicked:)));
            export_btn.setTarget(Some(&target));
            export_btn.setAction(Some(objc2::sel!(exportClicked:)));
            import_btn.setTarget(Some(&target));
            import_btn.setAction(Some(objc2::sel!(importClicked:)));
            backups_btn.setTarget(Some(&target));
            backups_btn.setAction(Some(objc2::sel!(backupsClicked:)));
        }
        add_arranged(&chrome.io_stack, &open_file);
        add_arranged(&chrome.io_stack, &export_btn);
        add_arranged(&chrome.io_stack, &import_btn);
        add_arranged(&chrome.io_stack, &backups_btn);

        if let Some(content) = window.contentView() {
            layout_pref_sidebar_chrome(
                &content,
                &chrome.sidebar,
                &chrome.nav_stack,
                &chrome.io_stack,
                &chrome.scroll,
                &chrome.saved_label,
            );
        }

        let (page_global, global, gesture) = build_general_page(mtm, &target);
        let (page_appearance, appearance) = build_appearance_page(mtm, &target);
        let (page_hotkeys, hotkeys) = build_hotkeys_page(mtm, &target);
        let (page_profiles, profile) = build_profiles_page(mtm, &target);
        let (page_perms, perms) = build_perms_page(mtm, &target);

        let pages = [
            page_global,
            page_appearance,
            page_hotkeys,
            page_profiles,
            page_perms,
        ];

        let mut win = Self {
            window,
            target,
            nav_buttons,
            pages,
            page_host: chrome.page_host,
            scroll: chrome.scroll,
            sidebar: chrome.sidebar,
            nav_stack: chrome.nav_stack,
            io_stack: chrome.io_stack,
            saved_label: chrome.saved_label,
            saved_until: None,
            global,
            appearance,
            gesture,
            hotkeys,
            profile,
            perms,
            draft: config.clone(),
            last_applied: config.clone(),
            undo_stack: VecDeque::new(),
            action_rx: rx,
            current_nav: 0,
            current_profile: config.default_profile_index(),
            pending_apply: false,
            last_apply_at: None,
            key_monitor: None,
            mtm,
            front_held: false,
            status_message: None,
        };
        win.load_from_draft();
        win.show_nav(0);
        win
    }

    pub fn show(&mut self) {
        self.install_key_monitor();
        super::activation::present_settings_window(self.mtm, &self.window, &mut self.front_held);
        self.publish_live_chrome();
        self.on_window_resized();
    }

    pub fn hide(&mut self) {
        set_live_chrome(None);
        self.remove_key_monitor();
        *recording_slot().lock().unwrap() = None;
        super::activation::dismiss_utility_window(self.mtm, &self.window, &mut self.front_held);
    }

    pub fn is_visible(&self) -> bool {
        unsafe { self.window.isVisible() }
    }

    pub fn reload_from_config(&mut self, config: &AppConfig) {
        self.draft = config.clone();
        self.last_applied = config.clone();
        self.undo_stack.clear();
        self.current_profile = config.default_profile_index().min(
            config.profiles.len().saturating_sub(1),
        );
        self.status_message = None;
        self.pending_apply = false;
        self.load_from_draft();
        self.show_nav(self.current_nav);
    }

    pub fn apply_collapsed_icon(&mut self, name: &str) {
        self.draft.global_panel.settings.collapsed_icon = Some(name.to_string());
        set_field(&self.global.collapsed_icon, name);
        self.queue_apply(true);
    }

    pub fn notify_saved(&mut self) {
        if !self.is_visible() {
            return;
        }
        set_field(&self.saved_label, "已保存");
        unsafe { self.saved_label.setHidden(false) };
        self.saved_until = Some(Instant::now() + Duration::from_secs(1));
    }

    fn clear_saved_toast_if_due(&mut self) {
        let Some(until) = self.saved_until else {
            return;
        };
        if Instant::now() < until {
            return;
        }
        self.saved_until = None;
        set_field(&self.saved_label, "");
        unsafe { self.saved_label.setHidden(true) };
    }

    pub fn poll_actions(&mut self) {
        self.clear_saved_toast_if_due();
        while let Ok(action) = self.action_rx.try_recv() {
            match action {
                SettingsAction::CloseAttempt => self.handle_close(),
                SettingsAction::Undo => self.try_undo(),
                SettingsAction::Nav(i) => {
                    self.sync_all_into_draft();
                    self.show_nav(i);
                }
                SettingsAction::VisualChanged => {
                    self.sync_visual_into_draft();
                    self.update_slider_labels();
                    self.rebuild_preview();
                    // 行列/尺寸滑条拖动时尽快刷新面板
                    self.queue_apply(false);
                }
                SettingsAction::ProfileChanged => {
                    self.sync_profile_fields_into_draft();
                    self.current_profile =
                        unsafe { self.profile.popup.indexOfSelectedItem() }.max(0) as usize;
                    self.load_profile_fields();
                    self.rebuild_button_table();
                    self.queue_apply(true);
                }
                SettingsAction::AddProfile => {
                    self.sync_profile_fields_into_draft();
                    let name = format!("profile_{}", self.draft.profiles.len());
                    self.draft.profiles.push(Profile {
                        name,
                        match_rule: None,
                        panel: PanelSettings::default(),
                        buttons: vec![],
                    });
                    self.current_profile = self.draft.profiles.len() - 1;
                    self.reload_profile_popup();
                    self.load_profile_fields();
                    self.rebuild_button_table();
                    self.queue_apply(true);
                }
                SettingsAction::DeleteProfile => {
                    if self
                        .draft
                        .profiles
                        .get(self.current_profile)
                        .is_some_and(|p| p.name != "default")
                    {
                        if !self.confirm_delete_profile() {
                            continue;
                        }
                        self.draft.profiles.remove(self.current_profile);
                        self.current_profile = self
                            .current_profile
                            .min(self.draft.profiles.len().saturating_sub(1));
                        self.reload_profile_popup();
                        self.load_profile_fields();
                        self.rebuild_button_table();
                        self.queue_apply(true);
                    }
                }
                SettingsAction::EditButton(button) => {
                    self.sync_profile_fields_into_draft();
                    emit(NativeUiEvent::OpenButtonEditor {
                        profile: self.current_profile,
                        button,
                        script: true,
                    });
                }
                SettingsAction::ClearProfileTheme => {
                    if let Some(p) = self.draft.profiles.get_mut(self.current_profile) {
                        p.panel.theme = ThemeSettings::default();
                        p.panel.button_width = None;
                        p.panel.button_height = None;
                    }
                    self.load_appearance_profile_fields();
                    self.queue_apply(true);
                }
                SettingsAction::StartRecord(slot) => {
                    *recording_slot().lock().unwrap() = Some(slot);
                    self.update_hotkey_buttons();
                }
                SettingsAction::HotkeyCaptured(slot, spec) => {
                    *recording_slot().lock().unwrap() = None;
                    match slot {
                        0 => self.draft.hotkeys.toggle_visibility = spec,
                        1 => self.draft.hotkeys.reload_config = spec,
                        2 => self.draft.hotkeys.quit = spec,
                        3 => self.draft.hotkeys.open_settings = spec,
                        _ => {}
                    }
                    self.load_hotkeys();
                    self.update_hotkey_buttons();
                    self.queue_apply(true);
                }
                SettingsAction::CancelRecord => {
                    *recording_slot().lock().unwrap() = None;
                    self.update_hotkey_buttons();
                }
                SettingsAction::RefreshPerms => self.refresh_permissions(),
                SettingsAction::Export => self.try_export(),
                SettingsAction::Import => self.try_import(),
                SettingsAction::ViewBackups => self.try_view_backups(),
                SettingsAction::WindowResized => self.on_window_resized(),
            }
        }

        // 防抖 apply（滑块连续拖动时合并）；持久化 debounce 在 runtime
        if self.is_visible() && self.pending_apply {
            let due = self
                .last_apply_at
                .is_none_or(|t| t.elapsed() >= Duration::from_millis(150));
            if due {
                self.pending_apply = false;
                self.try_apply();
            }
        }
    }

    fn on_window_resized(&mut self) {
        self.publish_live_chrome();
        layout_settings_chrome_now();
    }

    fn publish_live_chrome(&self) {
        if !self.is_visible() {
            set_live_chrome(None);
            return;
        }
        let page = self.pages[self.current_nav.min(self.pages.len() - 1)].clone();
        set_live_chrome(Some(LiveChrome {
            window: self.window.clone(),
            sidebar: self.sidebar.clone(),
            nav_stack: self.nav_stack.clone(),
            io_stack: self.io_stack.clone(),
            scroll: self.scroll.clone(),
            page_host: self.page_host.clone(),
            page,
            saved_label: self.saved_label.clone(),
        }));
    }

    fn show_nav(&mut self, idx: usize) {
        self.current_nav = idx.min(NAV_TITLES.len() - 1);
        for (i, btn) in self.nav_buttons.iter().enumerate() {
            unsafe {
                btn.setState(if i == self.current_nav {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
            }
        }
        install_page(&self.page_host, &self.pages[self.current_nav]);
        scroll_document_to_top(&self.scroll);
        self.publish_live_chrome();
        if self.current_nav == 4 {
            self.refresh_permissions();
        }
    }

    fn queue_apply(&mut self, immediate: bool) {
        self.pending_apply = true;
        if immediate {
            self.last_apply_at = Some(Instant::now() - Duration::from_millis(200));
        } else if self.last_apply_at.is_none() {
            self.last_apply_at = Some(Instant::now());
        }
    }

    fn handle_close(&mut self) {
        // 先刷出挂起的 apply，再通知 runtime flush 写盘并关窗
        if self.pending_apply {
            self.pending_apply = false;
            self.try_apply();
        }
        emit(NativeUiEvent::CloseSettings);
    }

    fn try_apply(&mut self) {
        match self.collect_draft_validated() {
            Ok(cfg) => {
                self.status_message = None;
                set_field(&self.global.status, "");
                self.last_apply_at = Some(Instant::now());
                self.push_undo_if_changed(&cfg);
                self.last_applied = cfg.clone();
                emit(NativeUiEvent::ApplySettings(cfg));
            }
            Err(err) => {
                self.status_message = Some(err.clone());
                set_field(&self.global.status, &err);
                // 校验失败不 apply；保持当前 core 配置
            }
        }
    }

    fn push_undo_if_changed(&mut self, next: &AppConfig) {
        // 用指针比较以外的廉价判据：若与 last_applied 同引用语义则跳过；
        // AppConfig 无 Eq，这里始终压栈（滑块步进可撤销），仅限深度。
        let _ = next;
        self.undo_stack.push_back(self.last_applied.clone());
        while self.undo_stack.len() > UNDO_DEPTH {
            self.undo_stack.pop_front();
        }
    }

    fn try_undo(&mut self) {
        let Some(prev) = self.undo_stack.pop_back() else {
            set_field(&self.saved_label, "无可撤销");
            unsafe { self.saved_label.setHidden(false) };
            self.saved_until = Some(Instant::now() + Duration::from_millis(800));
            return;
        };
        self.pending_apply = false;
        self.draft = prev.clone();
        self.last_applied = prev.clone();
        self.load_from_draft();
        self.rebuild_button_table();
        self.rebuild_preview();
        emit(NativeUiEvent::ApplySettings(prev));
        set_field(&self.saved_label, "已撤销");
        unsafe { self.saved_label.setHidden(false) };
        self.saved_until = Some(Instant::now() + Duration::from_secs(1));
    }

    fn confirm_delete_profile(&self) -> bool {
        let name = self
            .draft
            .profiles
            .get(self.current_profile)
            .map(|p| p.name.as_str())
            .unwrap_or("?");
        let alert = NSAlert::new(self.mtm);
        unsafe {
            alert.setAlertStyle(NSAlertStyle::Warning);
            alert.setMessageText(&NSString::from_str("删除配置档？"));
            alert.setInformativeText(&NSString::from_str(&format!(
                "确定删除配置档「{name}」？此操作会立即写入配置文件。"
            )));
            alert.addButtonWithTitle(&NSString::from_str("删除"));
            alert.addButtonWithTitle(&NSString::from_str("取消"));
            alert.runModal() == NSAlertFirstButtonReturn
        }
    }

    fn try_export(&mut self) {
        self.sync_all_into_draft();
        emit(NativeUiEvent::ExportConfig(self.draft.clone()));
    }

    fn try_import(&mut self) {
        emit(NativeUiEvent::ImportConfig);
    }

    fn try_view_backups(&mut self) {
        emit(NativeUiEvent::ViewBackups);
    }

    fn install_key_monitor(&mut self) {
        if self.key_monitor.is_some() {
            return;
        }
        let block = block2::RcBlock::new(|event: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
            let event = unsafe { event.as_ref() };
            let flags = event.modifierFlags();
            let chars = event
                .charactersIgnoringModifiers()
                .map(|s| s.to_string())
                .unwrap_or_default();
            let keycode = event.keyCode();

            // Esc
            if keycode == 53 {
                if recording_slot().lock().ok().and_then(|g| *g).is_some() {
                    emit_tag(SettingsAction::CancelRecord);
                    return std::ptr::null_mut();
                }
                emit_tag(SettingsAction::CloseAttempt);
                return std::ptr::null_mut();
            }

            // Cmd+Z 撤销（录制快捷键 / 正在编辑文本时不抢）
            if recording_slot().lock().ok().and_then(|g| *g).is_none()
                && flags.contains(NSEventModifierFlags::Command)
                && !flags.contains(NSEventModifierFlags::Shift)
                && chars.eq_ignore_ascii_case("z")
                && !text_field_is_editing()
            {
                emit_tag(SettingsAction::Undo);
                return std::ptr::null_mut();
            }

            if let Ok(guard) = recording_slot().lock() {
                if let Some(slot) = *guard {
                    if let Some(spec) = event_to_hotkey_spec(event) {
                        drop(guard);
                        emit_tag(SettingsAction::HotkeyCaptured(slot, spec));
                        return std::ptr::null_mut();
                    }
                }
            }
            event as *const NSEvent as *mut NSEvent
        });
        let monitor = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &block)
        };
        self.key_monitor = monitor;
    }

    fn remove_key_monitor(&mut self) {
        if let Some(mon) = self.key_monitor.take() {
            unsafe { NSEvent::removeMonitor(&mon) };
        }
    }

    fn load_from_draft(&mut self) {
        let g = &self.draft.global_panel.settings;
        unsafe {
            self.global
                .rows
                .setDoubleValue(g.rows.unwrap_or(3).clamp(1, 9) as f64);
            self.global
                .cols
                .setDoubleValue(g.cols.unwrap_or(3).clamp(1, 9) as f64);
            self.global.display_mode.selectItemAtIndex(match g.display_mode {
                Some(DisplayMode::Icon) => 1,
                Some(DisplayMode::IndexIcon) => 2,
                _ => 0,
            });
            self.global.context_mode.selectItemAtIndex(match self.draft.context_mode {
                ContextMode::ButtonFilter => 1,
                ContextMode::Profile => 0,
            });
        }
        set_switch(&self.global.collapsed, g.collapsed.unwrap_or(false));
        set_field(
            &self.global.collapsed_icon,
            g.collapsed_icon
                .as_deref()
                .unwrap_or(crate::config::PanelConfig::DEFAULT_COLLAPSED_ICON),
        );
        let opacity = g.opacity.unwrap_or(0.85) as f64;
        unsafe {
            self.global.opacity.setDoubleValue(opacity);
            self.appearance.btn_w.setDoubleValue(
                g.button_width
                    .unwrap_or(64.0)
                    .clamp(BTN_SIZE_MIN as f32, BTN_SIZE_MAX as f32) as f64,
            );
            self.appearance.btn_h.setDoubleValue(
                g.button_height
                    .unwrap_or(64.0)
                    .clamp(BTN_SIZE_MIN as f32, BTN_SIZE_MAX as f32) as f64,
            );
        }
        set_field(
            &self.global.gap,
            &g.gap
                .map(|v| format!("{v:.0}"))
                .unwrap_or_else(|| "4".into()),
        );
        set_field(
            &self.global.pos_x,
            &format!("{:.0}", self.draft.global_panel.position.x),
        );
        set_field(
            &self.global.pos_y,
            &format!("{:.0}", self.draft.global_panel.position.y),
        );
        set_field(
            &self.global.poll_ms,
            &self.draft.poll_interval_ms.to_string(),
        );
        set_field(
            &self.global.status,
            self.status_message.as_deref().unwrap_or(""),
        );

        // Appearance global theme
        load_theme_into_wells(&self.appearance, &g.theme, true);
        self.load_appearance_profile_fields();

        // Gesture
        set_switch(&self.gesture.enabled, self.draft.gesture.enabled);
        set_switch(
            &self.gesture.middle_click,
            self.draft.gesture.middle_click_enabled,
        );
        set_switch(
            &self.gesture.global_priority,
            self.draft.gesture.global_priority,
        );
        unsafe {
            self.gesture
                .threshold
                .setDoubleValue(self.draft.gesture.drag_threshold_px as f64);
        }
        set_switch(
            &self.gesture.dismiss_outside,
            self.draft.gesture.dismiss_on_outside_click,
        );
        set_switch(
            &self.gesture.hide_after,
            self.draft.gesture.hide_after_button,
        );
        self.update_slider_labels();

        self.load_hotkeys();
        self.reload_profile_popup();
        self.load_profile_fields();
        self.rebuild_button_table();
        self.rebuild_preview();
    }

    fn load_hotkeys(&self) {
        let hk = &self.draft.hotkeys;
        set_field(&self.hotkeys.labels[0], &hk.toggle_visibility);
        set_field(&self.hotkeys.labels[1], &hk.reload_config);
        set_field(&self.hotkeys.labels[2], &hk.quit);
        set_field(&self.hotkeys.labels[3], &hk.open_settings);
        self.update_hotkey_buttons();
    }

    fn update_hotkey_buttons(&self) {
        let active = recording_slot().lock().ok().and_then(|g| *g);
        for (i, btn) in self.hotkeys.buttons.iter().enumerate() {
            let title = if Some(i) == active {
                "按下新快捷键…（Esc 取消）"
            } else {
                "点击录制"
            };
            unsafe { btn.setTitle(&NSString::from_str(title)) };
        }
    }

    fn update_slider_labels(&self) {
        let rows = slider_int(&self.global.rows, 1, 9);
        let cols = slider_int(&self.global.cols, 1, 9);
        set_field(&self.global.rows_label, &rows.to_string());
        set_field(&self.global.cols_label, &cols.to_string());
        let opacity = unsafe { self.global.opacity.doubleValue() };
        set_field(&self.global.opacity_label, &format!("{opacity:.2}"));
        let thr = unsafe { self.gesture.threshold.doubleValue() };
        set_field(&self.gesture.threshold_label, &format!("{thr:.0} px"));
        let bw = slider_size(&self.appearance.btn_w);
        let bh = slider_size(&self.appearance.btn_h);
        set_field(&self.appearance.btn_w_label, &format!("{bw:.0}"));
        set_field(&self.appearance.btn_h_label, &format!("{bh:.0}"));
    }

    fn reload_profile_popup(&self) {
        unsafe {
            self.profile.popup.removeAllItems();
            let titles: Vec<_> = self
                .draft
                .profiles
                .iter()
                .map(|p| NSString::from_str(&p.name))
                .collect();
            self.profile
                .popup
                .addItemsWithTitles(&NSArray::from_retained_slice(&titles));
            if !self.draft.profiles.is_empty() {
                self.profile
                    .popup
                    .selectItemAtIndex(self.current_profile as isize);
            }

            self.appearance.profile_popup.removeAllItems();
            self.appearance
                .profile_popup
                .addItemsWithTitles(&NSArray::from_retained_slice(&titles));
            if !self.draft.profiles.is_empty() {
                self.appearance
                    .profile_popup
                    .selectItemAtIndex(self.current_profile as isize);
            }
        }
    }

    fn load_profile_fields(&self) {
        let Some(profile) = self.draft.profiles.get(self.current_profile) else {
            return;
        };
        set_field(&self.profile.name, &profile.name);
        let (bundle, app, title) = match &profile.match_rule {
            Some(r) => (
                r.bundle_id.clone().unwrap_or_default(),
                r.app_name.clone().unwrap_or_default(),
                r.title_regex_text.clone().unwrap_or_default(),
            ),
            None => (String::new(), String::new(), String::new()),
        };
        set_field(&self.profile.match_bundle, &bundle);
        set_field(&self.profile.match_app, &app);
        set_field(&self.profile.match_title, &title);
        set_field(
            &self.profile.rows,
            &profile
                .panel
                .rows
                .map(|v| v.to_string())
                .unwrap_or_else(|| "".into()),
        );
        set_field(
            &self.profile.cols,
            &profile
                .panel
                .cols
                .map(|v| v.to_string())
                .unwrap_or_else(|| "".into()),
        );
        self.load_appearance_profile_fields();
    }

    fn load_appearance_profile_fields(&self) {
        let Some(profile) = self.draft.profiles.get(self.current_profile) else {
            return;
        };
        set_field(
            &self.appearance.p_btn_w,
            &profile
                .panel
                .button_width
                .map(|v| format!("{v:.0}"))
                .unwrap_or_default(),
        );
        set_field(
            &self.appearance.p_btn_h,
            &profile
                .panel
                .button_height
                .map(|v| format!("{v:.0}"))
                .unwrap_or_default(),
        );
    }

    fn rebuild_button_table(&self) {
        unsafe {
            let subs = self.profile.button_host.subviews();
            for view in subs.iter() {
                view.removeFromSuperview();
            }
        }
        let Some(profile) = self.draft.profiles.get(self.current_profile) else {
            return;
        };
        for (i, button) in profile.buttons.iter().enumerate() {
            let label = if button.label.trim().is_empty() {
                format!("按钮 {}", i + 1)
            } else {
                button.label.clone()
            };
            let summary = format!(
                "#{i} {} [{}] {} · {}",
                label,
                if button.enabled { "开" } else { "关" },
                action_summary(&button.action),
                scope_label(&button.scope)
            );
            let row = h_stack(self.mtm);
            let tf = unsafe {
                let t = NSTextField::labelWithString(&NSString::from_str(&summary), self.mtm);
                t.setFrameSize(NSSize::new(420.0, 22.0));
                t
            };
            let btn = make_button(self.mtm, "编辑…", 72.0);
            unsafe {
                btn.setTag(i as isize);
                btn.setTarget(Some(&self.target));
                btn.setAction(Some(objc2::sel!(editButtonClicked:)));
            }
            add_arranged(&row, &tf);
            add_arranged(&row, &btn);
            add_arranged(&self.profile.button_host, &row);
        }
    }

    fn rebuild_preview(&self) {
        unsafe {
            let subs = self.appearance.preview_host.subviews();
            for v in subs.iter() {
                v.removeFromSuperview();
            }
        }
        let g = &self.draft.global_panel.settings;
        let rows = g.rows.unwrap_or(3).max(1).min(9);
        let cols = g.cols.unwrap_or(3).max(1).min(9);
        let theme = g.theme.resolve();
        let bg = ns_color_from_rgb(theme.button_bg);
        for _r in 0..rows {
            let row = h_stack(self.mtm);
            for _c in 0..cols {
                let cell = unsafe {
                    let b = NSBox::initWithFrame(
                        self.mtm.alloc(),
                        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(28.0, 28.0)),
                    );
                    b.setBoxType(NSBoxType::Custom);
                    b.setTitlePosition(objc2_app_kit::NSTitlePosition::NoTitle);
                    b.setFillColor(&bg);
                    b.setBorderWidth(1.0);
                    b.setBorderColor(&ns_color_from_rgb(theme.button_border_color));
                    NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(&[
                        b.widthAnchor().constraintEqualToConstant(28.0),
                        b.heightAnchor().constraintEqualToConstant(28.0),
                    ]));
                    b
                };
                add_arranged(&row, &cell);
            }
            add_arranged(&self.appearance.preview_host, &row);
        }
    }

    fn refresh_permissions(&self) {
        let status = check_permissions();
        let ax = if status.accessibility {
            "辅助功能：已授权 ✓"
        } else {
            "辅助功能：未授权 — 请打开系统设置并勾选 MeoTool"
        };
        let input = if status.input_monitoring {
            "输入监控：已授权 ✓"
        } else {
            "输入监控：未授权 — 请打开系统设置并勾选 MeoTool"
        };
        let path = executable_path()
            .map(|p| format!("可执行文件：{}", p.display()))
            .unwrap_or_else(|| "可执行文件：未知".into());
        set_field(&self.perms.status_ax, ax);
        set_field(&self.perms.status_input, input);
        set_field(&self.perms.path_label, &path);
    }

    fn sync_all_into_draft(&mut self) {
        self.sync_visual_into_draft();
        self.sync_profile_fields_into_draft();
        // Hotkeys already live in draft when captured
    }

    fn sync_visual_into_draft(&mut self) {
        let rows = slider_int(&self.global.rows, 1, 9);
        let cols = slider_int(&self.global.cols, 1, 9);
        let opacity = unsafe { self.global.opacity.doubleValue() }.clamp(0.05, 1.0) as f32;
        let gap: f32 = field_string(&self.global.gap).parse().unwrap_or(4.0);
        let poll: u64 = field_string(&self.global.poll_ms).parse().unwrap_or(500);
        let mode_idx = unsafe { self.global.display_mode.indexOfSelectedItem() };
        let ctx_idx = unsafe { self.global.context_mode.indexOfSelectedItem() };
        let pos_x: f32 = field_string(&self.global.pos_x).parse().unwrap_or(100.0);
        let pos_y: f32 = field_string(&self.global.pos_y).parse().unwrap_or(200.0);

        self.draft.global_panel.settings.rows = Some(rows);
        self.draft.global_panel.settings.cols = Some(cols);
        self.draft.global_panel.settings.opacity = Some(opacity);
        self.draft.global_panel.settings.gap = Some(gap);
        self.draft.global_panel.settings.collapsed = Some(switch_on(&self.global.collapsed));
        self.draft.global_panel.settings.collapsed_icon =
            Some(field_string(&self.global.collapsed_icon));
        self.draft.global_panel.settings.display_mode = Some(match mode_idx {
            1 => DisplayMode::Icon,
            2 => DisplayMode::IndexIcon,
            _ => DisplayMode::Text,
        });
        self.draft.global_panel.position.x = pos_x;
        self.draft.global_panel.position.y = pos_y;
        self.draft.poll_interval_ms = poll;
        self.draft.context_mode = if ctx_idx == 1 {
            ContextMode::ButtonFilter
        } else {
            ContextMode::Profile
        };

        self.draft.global_panel.settings.button_width = Some(slider_size(&self.appearance.btn_w));
        self.draft.global_panel.settings.button_height = Some(slider_size(&self.appearance.btn_h));
        store_theme_from_wells(&self.appearance, &mut self.draft.global_panel.settings.theme, true);

        self.draft.gesture.enabled = switch_on(&self.gesture.enabled);
        self.draft.gesture.middle_click_enabled = switch_on(&self.gesture.middle_click);
        self.draft.gesture.global_priority = switch_on(&self.gesture.global_priority);
        self.draft.gesture.drag_threshold_px =
            unsafe { self.gesture.threshold.doubleValue() }.clamp(2.0, 64.0) as f32;
        self.draft.gesture.dismiss_on_outside_click = switch_on(&self.gesture.dismiss_outside);
        self.draft.gesture.hide_after_button = switch_on(&self.gesture.hide_after);

        // Appearance profile override selection follows appearance popup if on that page
        let ap_idx = unsafe { self.appearance.profile_popup.indexOfSelectedItem() }.max(0) as usize;
        if ap_idx < self.draft.profiles.len() {
            self.current_profile = ap_idx;
        }
        if let Some(p) = self.draft.profiles.get_mut(self.current_profile) {
            let w = field_string(&self.appearance.p_btn_w);
            let h = field_string(&self.appearance.p_btn_h);
            p.panel.button_width = if w.trim().is_empty() {
                None
            } else {
                w.parse().ok()
            };
            p.panel.button_height = if h.trim().is_empty() {
                None
            } else {
                h.parse().ok()
            };
        }
    }

    fn sync_profile_fields_into_draft(&mut self) {
        let name = field_string(&self.profile.name);
        let bundle = field_string(&self.profile.match_bundle);
        let app = field_string(&self.profile.match_app);
        let title = field_string(&self.profile.match_title);
        let rows = field_string(&self.profile.rows);
        let cols = field_string(&self.profile.cols);
        if let Some(profile) = self.draft.profiles.get_mut(self.current_profile) {
            if !name.trim().is_empty() {
                profile.name = name;
            }
            if bundle.is_empty() && app.is_empty() && title.is_empty() {
                profile.match_rule = None;
            } else {
                profile.match_rule = Some(MatchRule {
                    bundle_id: nonempty(bundle),
                    app_name: nonempty(app),
                    title_regex: None,
                    title_regex_text: nonempty(title),
                });
            }
            profile.panel.rows = if rows.trim().is_empty() {
                None
            } else {
                rows.parse().ok()
            };
            profile.panel.cols = if cols.trim().is_empty() {
                None
            } else {
                cols.parse().ok()
            };
        }
    }

    fn collect_draft_validated(&mut self) -> Result<AppConfig, String> {
        self.sync_all_into_draft();
        let rows = slider_int(&self.global.rows, 1, 9);
        let cols = slider_int(&self.global.cols, 1, 9);
        self.draft.global_panel.settings.rows = Some(rows);
        self.draft.global_panel.settings.cols = Some(cols);

        for profile in &mut self.draft.profiles {
            if let Some(rule) = &mut profile.match_rule {
                if let Some(text) = rule.title_regex_text.as_deref().filter(|s| !s.is_empty()) {
                    rule.title_regex = Some(
                        regex::Regex::new(text)
                            .map_err(|e| format!("配置档「{}」标题正则无效: {e}", profile.name))?,
                    );
                } else {
                    rule.title_regex = None;
                    rule.title_regex_text = None;
                }
            }
        }

        // 滑条实时缩格：空槽可裁掉；已配置按钮保留（面板只显示网格内，展开后仍可编辑）
        let global_rows = self.draft.global_panel.settings.rows;
        let global_cols = self.draft.global_panel.settings.cols;
        for profile in &mut self.draft.profiles {
            let r = profile.panel.rows.or(global_rows).unwrap_or(3) as usize;
            let c = profile.panel.cols.or(global_cols).unwrap_or(3) as usize;
            let slots = r * c;
            while profile.buttons.len() < slots {
                profile.buttons.push(empty_slot_button());
            }
            if profile.buttons.len() > slots {
                let overflow_configured = profile.buttons[slots..]
                    .iter()
                    .any(button_is_configured);
                if !overflow_configured {
                    profile.buttons.truncate(slots);
                }
            }
        }
        self.draft.hotkeys = GlobalHotkeys {
            toggle_visibility: field_string(&self.hotkeys.labels[0]),
            reload_config: field_string(&self.hotkeys.labels[1]),
            quit: field_string(&self.hotkeys.labels[2]),
            open_settings: field_string(&self.hotkeys.labels[3]),
        };
        normalize_app_config(&mut self.draft);
        Ok(self.draft.clone())
    }
}

fn build_general_page(
    mtm: MainThreadMarker,
    target: &SettingsTarget,
) -> (Retained<NSStackView>, GlobalFields, GestureFields) {
    let page = v_stack(mtm);
    add_arranged(&page, &section_title(mtm, "面板"));
    let (r1, rows, rows_label) = labeled_int_slider(
        mtm,
        target,
        "行数",
        ROWS_COLS_MIN,
        ROWS_COLS_MAX,
        "3",
    );
    let (r2, cols, cols_label) = labeled_int_slider(
        mtm,
        target,
        "列数",
        ROWS_COLS_MIN,
        ROWS_COLS_MAX,
        "3",
    );
    let (r3, display_mode) = labeled_popup(
        mtm,
        "显示模式",
        &["文字", "图标", "序号图标"],
        200.0,
    );
    let (r4, collapsed) = labeled_switch(mtm, "折叠模式");
    let (r5, collapsed_icon) = labeled_field(mtm, "折叠图标", 220.0);
    let pick = make_button(mtm, "选择…", 72.0);
    unsafe {
        pick.setTarget(Some(target));
        pick.setAction(Some(objc2::sel!(pickCollapsedIcon:)));
    }
    add_arranged(&r5, &pick);

    let (opacity_row, opacity, opacity_label) =
        labeled_float_slider(mtm, target, "透明度", 0.2, 1.0, "0.85");

    let (r6, gap) = labeled_field(mtm, "间距", 80.0);
    let (r7, pos_x) = labeled_field(mtm, "位置 X", 100.0);
    let (r8, pos_y) = labeled_field(mtm, "位置 Y", 100.0);
    let (r9, poll_ms) = labeled_field(mtm, "窗口轮询 ms", 100.0);
    let (r10, context_mode) = labeled_popup(
        mtm,
        "上下文模式",
        &["Profile 切换（默认）", "按钮过滤"],
        260.0,
    );
    let help = help_label(
        mtm,
        "按钮过滤：仅使用 default 配置档的布局与按钮；按钮 scope 才生效。拖动行列滑条可实时改变面板网格。",
    );
    let status = unsafe { NSTextField::labelWithString(&NSString::from_str(""), mtm) };
    unsafe {
        status.setTextColor(Some(&NSColor::systemRedColor()));
    }

    for control in [&collapsed_icon, &gap, &pos_x, &pos_y, &poll_ms] {
        unsafe {
            control.setTarget(Some(target));
            control.setAction(Some(objc2::sel!(fieldChanged:)));
        }
    }
    unsafe {
        display_mode.setTarget(Some(target));
        display_mode.setAction(Some(objc2::sel!(fieldChanged:)));
        collapsed.setTarget(Some(target));
        collapsed.setAction(Some(objc2::sel!(fieldChanged:)));
        context_mode.setTarget(Some(target));
        context_mode.setAction(Some(objc2::sel!(fieldChanged:)));
    }

    for row in [&r1, &r2, &r3, &r4, &r5, &opacity_row, &r6, &r7, &r8, &r9, &r10] {
        add_arranged(&page, row);
    }
    add_arranged(&page, &help);
    add_arranged(&page, &status);

    let gesture = append_gesture_section(&page, mtm, target);

    (
        page,
        GlobalFields {
            rows,
            rows_label,
            cols,
            cols_label,
            display_mode,
            collapsed,
            collapsed_icon,
            opacity,
            opacity_label,
            gap,
            pos_x,
            pos_y,
            poll_ms,
            context_mode,
            status,
        },
        gesture,
    )
}

fn build_appearance_page(
    mtm: MainThreadMarker,
    target: &SettingsTarget,
) -> (Retained<NSStackView>, AppearanceFields) {
    let page = v_stack(mtm);
    add_arranged(&page, &section_title(mtm, "面板预览"));
    let preview_host = v_stack(mtm);
    unsafe { preview_host.setSpacing(4.0) };
    add_arranged(&page, &preview_host);

    add_arranged(&page, &section_title(mtm, "全局外观"));
    let (r1, btn_w, btn_w_label) = labeled_size_slider(
        mtm,
        target,
        "按钮宽度",
        BTN_SIZE_MIN,
        BTN_SIZE_MAX,
        "64",
    );
    let (r2, btn_h, btn_h_label) = labeled_size_slider(
        mtm,
        target,
        "按钮高度",
        BTN_SIZE_MIN,
        BTN_SIZE_MAX,
        "64",
    );
    add_arranged(&page, &r1);
    add_arranged(&page, &r2);
    add_arranged(
        &page,
        &help_label(mtm, "拖动滑条可实时改变面板按钮大小（32–128）。"),
    );

    let (bg_row, bg) = color_row(mtm, "面板背景", target);
    let (button_bg_row, button_bg) = color_row(mtm, "按钮背景", target);
    let (letter_bg_row, letter_bg) = color_row(mtm, "字母背景", target);
    let (letter_color_row, letter_color) = color_row(mtm, "文字颜色", target);
    add_arranged(&page, &bg_row);
    add_arranged(&page, &button_bg_row);
    add_arranged(&page, &letter_bg_row);
    add_arranged(&page, &letter_color_row);

    let (r3, letter_size) = labeled_field(mtm, "字号", 80.0);
    let (r4, letter_font) = labeled_field(mtm, "字体路径", 320.0);
    add_arranged(&page, &r3);
    add_arranged(&page, &r4);
    add_arranged(
        &page,
        &help_label(mtm, "留空则使用系统默认中文字体；可填 .ttf / .ttc 绝对路径。"),
    );

    let (r5, win_border_on) = labeled_switch(mtm, "窗口边框");
    let (r6, win_border_w) = labeled_field(mtm, "边框粗细", 80.0);
    let (win_border_color_row, win_border_color) = color_row(mtm, "边框颜色", target);
    let (r7, btn_border_on) = labeled_switch(mtm, "按钮边框");
    let (r8, btn_border_w) = labeled_field(mtm, "边框粗细", 80.0);
    let (btn_border_color_row, btn_border_color) = color_row(mtm, "边框颜色", target);
    for row in [
        &r5,
        &r6,
        &win_border_color_row,
        &r7,
        &r8,
        &btn_border_color_row,
    ] {
        add_arranged(&page, row);
    }

    add_arranged(&page, &section_title(mtm, "配置档外观覆盖"));
    let (prof_row, profile_popup) = labeled_popup(mtm, "配置档", &["default"], 220.0);
    unsafe {
        profile_popup.setTarget(Some(target));
        profile_popup.setAction(Some(objc2::sel!(profileChanged:)));
    }
    add_arranged(&page, &prof_row);
    let clear = make_button(mtm, "清除配置档外观覆盖", 160.0);
    unsafe {
        clear.setTarget(Some(target));
        clear.setAction(Some(objc2::sel!(clearProfileTheme:)));
    }
    add_arranged(&page, &clear);
    let (r9, p_btn_w) = labeled_field(mtm, "覆盖宽度", 80.0);
    let (r10, p_btn_h) = labeled_field(mtm, "覆盖高度", 80.0);
    add_arranged(&page, &r9);
    add_arranged(&page, &r10);
    add_arranged(
        &page,
        &help_label(mtm, "留空表示继承全局；填写后仅对该配置档生效。"),
    );

    for f in [
        &letter_size, &letter_font, &win_border_w, &btn_border_w, &p_btn_w, &p_btn_h,
    ] {
        unsafe {
            f.setTarget(Some(target));
            f.setAction(Some(objc2::sel!(fieldChanged:)));
        }
    }
    for sw in [&win_border_on, &btn_border_on] {
        unsafe {
            sw.setTarget(Some(target));
            sw.setAction(Some(objc2::sel!(fieldChanged:)));
        }
    }

    (
        page,
        AppearanceFields {
            btn_w,
            btn_w_label,
            btn_h,
            btn_h_label,
            bg,
            button_bg,
            letter_bg,
            letter_color,
            letter_size,
            letter_font,
            win_border_on,
            win_border_w,
            win_border_color,
            btn_border_on,
            btn_border_w,
            btn_border_color,
            profile_popup,
            p_btn_w,
            p_btn_h,
            preview_host,
        },
    )
}

fn append_gesture_section(
    page: &NSStackView,
    mtm: MainThreadMarker,
    target: &SettingsTarget,
) -> GestureFields {
    add_arranged(page, &section_title(mtm, "鼠标手势"));
    let (r1, enabled) = labeled_switch(mtm, "启用右键拖动唤起");
    let (r_mid, middle_click) = labeled_switch(mtm, "启用中键点击唤起");
    let (r2, global_priority) = labeled_switch(mtm, "全局优先");
    add_arranged(page, &r1);
    add_arranged(page, &r_mid);
    add_arranged(
        page,
        &help_label(mtm, "中键单击时在光标位置显示面板，可与右键拖动同时开启。"),
    );
    add_arranged(page, &r2);
    add_arranged(
        page,
        &help_label(
            mtm,
            "全局优先需已授予「输入监控」与「辅助功能」；开启后手势将优先于其他应用。",
        ),
    );

    let thr_row = h_stack(mtm);
    let thr_lab = unsafe { NSTextField::labelWithString(&NSString::from_str("拖动阈值"), mtm) };
    unsafe {
        NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(&[thr_lab
            .widthAnchor()
            .constraintEqualToConstant(LABEL_W)]));
    }
    let threshold = unsafe {
        let s = NSSlider::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(220.0, 24.0)),
        );
        s.setMinValue(2.0);
        s.setMaxValue(64.0);
        s.setContinuous(true);
        s.setTarget(Some(target));
        s.setAction(Some(objc2::sel!(fieldChanged:)));
        s
    };
    let threshold_label = unsafe { NSTextField::labelWithString(&NSString::from_str("8 px"), mtm) };
    add_arranged(&thr_row, &thr_lab);
    add_arranged(&thr_row, &threshold);
    add_arranged(&thr_row, &threshold_label);
    add_arranged(page, &thr_row);

    let (r3, dismiss_outside) = labeled_switch(mtm, "点击面板外收起");
    let (r4, hide_after) = labeled_switch(mtm, "鼠标移出后收起");
    add_arranged(page, &r3);
    add_arranged(page, &r4);

    for sw in [
        &enabled,
        &middle_click,
        &global_priority,
        &dismiss_outside,
        &hide_after,
    ] {
        unsafe {
            sw.setTarget(Some(target));
            sw.setAction(Some(objc2::sel!(fieldChanged:)));
        }
    }

    GestureFields {
        enabled,
        middle_click,
        global_priority,
        threshold,
        threshold_label,
        dismiss_outside,
        hide_after,
    }
}

fn build_hotkeys_page(
    mtm: MainThreadMarker,
    target: &SettingsTarget,
) -> (Retained<NSStackView>, HotkeyFields) {
    let page = v_stack(mtm);
    add_arranged(&page, &section_title(mtm, "全局快捷键"));
    add_arranged(
        &page,
        &help_label(mtm, "点击「点击录制」后按下新的快捷键组合；Esc 取消录制。"),
    );

    let titles = ["显示/隐藏面板", "重载配置", "退出", "打开设置"];
    let mut labels = Vec::new();
    let mut buttons = Vec::new();
    for (i, title) in titles.iter().enumerate() {
        let row = h_stack(mtm);
        let lab = unsafe { NSTextField::labelWithString(&NSString::from_str(title), mtm) };
        unsafe {
            NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(&[lab
                .widthAnchor()
                .constraintEqualToConstant(LABEL_W)]));
        }
        let value = unsafe {
            let t = NSTextField::labelWithString(&NSString::from_str(""), mtm);
            t.setFrameSize(NSSize::new(180.0, 22.0));
            t
        };
        let btn = make_button(mtm, "点击录制", 160.0);
        unsafe {
            btn.setTag(i as isize);
            btn.setTarget(Some(target));
            btn.setAction(Some(objc2::sel!(recordHotkey:)));
        }
        add_arranged(&row, &lab);
        add_arranged(&row, &value);
        add_arranged(&row, &btn);
        add_arranged(&page, &row);
        labels.push(value);
        buttons.push(btn);
    }

    (
        page,
        HotkeyFields {
            labels: labels.try_into().unwrap(),
            buttons: buttons.try_into().unwrap(),
        },
    )
}

fn build_profiles_page(
    mtm: MainThreadMarker,
    target: &SettingsTarget,
) -> (Retained<NSStackView>, ProfileFields) {
    let page = v_stack(mtm);
    add_arranged(&page, &section_title(mtm, "配置档"));

    let top = h_stack(mtm);
    let (popup_row, popup) = labeled_popup(mtm, "配置档", &["default"], 200.0);
    unsafe {
        popup.setTarget(Some(target));
        popup.setAction(Some(objc2::sel!(profileChanged:)));
    }
    let add = make_button(mtm, "新建", 64.0);
    let del = make_button(mtm, "删除", 64.0);
    unsafe {
        add.setTarget(Some(target));
        add.setAction(Some(objc2::sel!(addProfileClicked:)));
        del.setTarget(Some(target));
        del.setAction(Some(objc2::sel!(deleteProfileClicked:)));
    }
    add_arranged(&top, &popup_row);
    add_arranged(&top, &add);
    add_arranged(&top, &del);
    add_arranged(&page, &top);

    let (r1, name) = labeled_field(mtm, "名称", 220.0);
    let (r2, match_bundle) = labeled_field(mtm, "Bundle ID", 260.0);
    let (r3, match_app) = labeled_field(mtm, "应用名", 260.0);
    let (r4, match_title) = labeled_field(mtm, "标题正则", 260.0);
    let (r5, rows) = labeled_field(mtm, "覆盖行数", 80.0);
    let (r6, cols) = labeled_field(mtm, "覆盖列数", 80.0);
    for row in [&r1, &r2, &r3, &r4, &r5, &r6] {
        add_arranged(&page, row);
    }
    add_arranged(
        &page,
        &help_label(mtm, "匹配规则留空表示默认档；default 不可删除。双击表格行可编辑按钮。"),
    );

    add_arranged(&page, &section_title(mtm, "按钮"));
    let button_host = v_stack(mtm);
    unsafe { button_host.setSpacing(6.0) };
    let button_scroll = unsafe {
        let s = objc2_app_kit::NSScrollView::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(560.0, 240.0)),
        );
        s.setHasVerticalScroller(true);
        s.setDocumentView(Some(&button_host));
        s.setBorderType(objc2_app_kit::NSBorderType::BezelBorder);
        s.setDrawsBackground(false);
        NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(&[
            s.heightAnchor().constraintEqualToConstant(240.0),
            s.widthAnchor().constraintEqualToConstant(560.0),
        ]));
        s
    };
    add_arranged(&page, &button_scroll);
    add_arranged(
        &page,
        &help_label(mtm, "按钮列表可滚动；点「编辑」打开动作/图标/可见范围编辑器。"),
    );

        (
        page,
        ProfileFields {
            popup,
            name,
            match_bundle,
            match_app,
            match_title,
            rows,
            cols,
            button_host,
        },
    )
}

fn build_perms_page(
    mtm: MainThreadMarker,
    target: &SettingsTarget,
) -> (Retained<NSStackView>, PermFields) {
    let page = v_stack(mtm);
    add_arranged(&page, &section_title(mtm, "系统权限"));
    add_arranged(
        &page,
        &help_label(
            mtm,
            "手势 / 热键 / 点击穿透需要「辅助功能」与「输入监控」。授权后可能需重启应用。",
        ),
    );
    let status_ax = unsafe { NSTextField::labelWithString(&NSString::from_str("辅助功能：…"), mtm) };
    let status_input =
        unsafe { NSTextField::labelWithString(&NSString::from_str("输入监控：…"), mtm) };
    let path_label = unsafe { NSTextField::labelWithString(&NSString::from_str("可执行文件：…"), mtm) };
    for t in [&status_ax, &status_input, &path_label] {
        unsafe {
            t.setMaximumNumberOfLines(0);
            t.setPreferredMaxLayoutWidth(520.0);
        }
        add_arranged(&page, t);
    }
    let row = h_stack(mtm);
    let ax = make_button(mtm, "打开辅助功能设置", 150.0);
    let input = make_button(mtm, "打开输入监控设置", 150.0);
    let refresh = make_button(mtm, "刷新状态", 100.0);
    let relaunch = make_button(mtm, "重新检测并重启", 140.0);
    unsafe {
        ax.setTarget(Some(target));
        ax.setAction(Some(objc2::sel!(openAx:)));
        input.setTarget(Some(target));
        input.setAction(Some(objc2::sel!(openInput:)));
        refresh.setTarget(Some(target));
        refresh.setAction(Some(objc2::sel!(refreshPerms:)));
        relaunch.setTarget(Some(target));
        relaunch.setAction(Some(objc2::sel!(relaunchPerms:)));
    }
    add_arranged(&row, &ax);
    add_arranged(&row, &input);
    add_arranged(&page, &row);
    let row2 = h_stack(mtm);
    add_arranged(&row2, &refresh);
    add_arranged(&row2, &relaunch);
    add_arranged(&page, &row2);

    (
        page,
        PermFields {
            status_ax,
            status_input,
            path_label,
        },
    )
}

/// 整数刻度滑条（行/列）：右侧显示当前值。
fn labeled_int_slider(
    mtm: MainThreadMarker,
    target: &SettingsTarget,
    label: &str,
    min: f64,
    max: f64,
    initial: &str,
) -> (Retained<NSStackView>, Retained<NSSlider>, Retained<NSTextField>) {
    let row = h_stack(mtm);
    let lab = unsafe { NSTextField::labelWithString(&NSString::from_str(label), mtm) };
    unsafe {
        NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(&[lab
            .widthAnchor()
            .constraintEqualToConstant(LABEL_W)]));
    }
    let ticks = ((max - min).round() as isize) + 1;
    let slider = unsafe {
        let s = NSSlider::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(220.0, 24.0)),
        );
        s.setMinValue(min);
        s.setMaxValue(max);
        s.setNumberOfTickMarks(ticks);
        s.setAllowsTickMarkValuesOnly(true);
        s.setContinuous(true);
        s.setTarget(Some(target));
        s.setAction(Some(objc2::sel!(fieldChanged:)));
        s
    };
    let value = unsafe { NSTextField::labelWithString(&NSString::from_str(initial), mtm) };
    unsafe {
        NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(&[value
            .widthAnchor()
            .constraintEqualToConstant(36.0)]));
    }
    add_arranged(&row, &lab);
    add_arranged(&row, &slider);
    add_arranged(&row, &value);
    (row, slider, value)
}

/// 浮点滑条（透明度等）。
fn labeled_float_slider(
    mtm: MainThreadMarker,
    target: &SettingsTarget,
    label: &str,
    min: f64,
    max: f64,
    initial: &str,
) -> (Retained<NSStackView>, Retained<NSSlider>, Retained<NSTextField>) {
    let row = h_stack(mtm);
    let lab = unsafe { NSTextField::labelWithString(&NSString::from_str(label), mtm) };
    unsafe {
        NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(&[lab
            .widthAnchor()
            .constraintEqualToConstant(LABEL_W)]));
    }
    let slider = unsafe {
        let s = NSSlider::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(220.0, 24.0)),
        );
        s.setMinValue(min);
        s.setMaxValue(max);
        s.setContinuous(true);
        s.setTarget(Some(target));
        s.setAction(Some(objc2::sel!(fieldChanged:)));
        s
    };
    let value = unsafe { NSTextField::labelWithString(&NSString::from_str(initial), mtm) };
    unsafe {
        NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(&[value
            .widthAnchor()
            .constraintEqualToConstant(48.0)]));
    }
    add_arranged(&row, &lab);
    add_arranged(&row, &slider);
    add_arranged(&row, &value);
    (row, slider, value)
}

/// 按钮尺寸滑条（取整显示）。
fn labeled_size_slider(
    mtm: MainThreadMarker,
    target: &SettingsTarget,
    label: &str,
    min: f64,
    max: f64,
    initial: &str,
) -> (Retained<NSStackView>, Retained<NSSlider>, Retained<NSTextField>) {
    labeled_float_slider(mtm, target, label, min, max, initial)
}

fn slider_int(slider: &NSSlider, min: u8, max: u8) -> u8 {
    let v = unsafe { slider.doubleValue() }.round() as i32;
    v.clamp(min as i32, max as i32) as u8
}

fn slider_size(slider: &NSSlider) -> f32 {
    let v = unsafe { slider.doubleValue() }.round() as f32;
    v.clamp(BTN_SIZE_MIN as f32, BTN_SIZE_MAX as f32)
}

fn color_row(
    mtm: MainThreadMarker,
    label: &str,
    target: &SettingsTarget,
) -> (Retained<NSStackView>, Retained<NSColorWell>) {
    let row = h_stack(mtm);
    let lab = unsafe { NSTextField::labelWithString(&NSString::from_str(label), mtm) };
    unsafe {
        NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(&[lab
            .widthAnchor()
            .constraintEqualToConstant(LABEL_W)]));
    }
    let well = unsafe {
        let w = NSColorWell::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(44.0, 24.0)),
        );
        w.setTarget(Some(target));
        w.setAction(Some(objc2::sel!(fieldChanged:)));
        w
    };
    add_arranged(&row, &lab);
    add_arranged(&row, &well);
    (row, well)
}

fn load_theme_into_wells(fields: &AppearanceFields, theme: &ThemeSettings, _global: bool) {
    set_well_color(&fields.bg, theme.background_color.as_deref());
    set_well_color(&fields.button_bg, theme.button_color.as_deref());
    set_well_color(&fields.letter_bg, theme.letter_background_color.as_deref());
    set_well_color(&fields.letter_color, theme.letter_color.as_deref());
    set_field(
        &fields.letter_size,
        &theme
            .letter_size
            .map(|v| format!("{v:.0}"))
            .unwrap_or_else(|| "11".into()),
    );
    set_field(
        &fields.letter_font,
        theme.letter_font.as_deref().unwrap_or(""),
    );
    set_switch(
        &fields.win_border_on,
        theme.window_border_enabled.unwrap_or(false),
    );
    set_field(
        &fields.win_border_w,
        &theme
            .window_border_width
            .map(|v| format!("{v:.1}"))
            .unwrap_or_else(|| "1".into()),
    );
    set_well_color(
        &fields.win_border_color,
        theme.window_border_color.as_deref(),
    );
    set_switch(
        &fields.btn_border_on,
        theme.button_border_enabled.unwrap_or(true),
    );
    set_field(
        &fields.btn_border_w,
        &theme
            .button_border_width
            .map(|v| format!("{v:.1}"))
            .unwrap_or_else(|| "1".into()),
    );
    set_well_color(
        &fields.btn_border_color,
        theme.button_border_color.as_deref(),
    );
}

fn store_theme_from_wells(fields: &AppearanceFields, theme: &mut ThemeSettings, _global: bool) {
    theme.background_color = Some(well_to_string(&fields.bg));
    theme.button_color = Some(well_to_string(&fields.button_bg));
    theme.letter_background_color = Some(well_to_string(&fields.letter_bg));
    theme.letter_color = Some(well_to_string(&fields.letter_color));
    theme.letter_size = field_string(&fields.letter_size).parse().ok();
    let font = field_string(&fields.letter_font);
    theme.letter_font = if font.trim().is_empty() {
        None
    } else {
        Some(font)
    };
    theme.window_border_enabled = Some(switch_on(&fields.win_border_on));
    theme.window_border_width = field_string(&fields.win_border_w).parse().ok();
    theme.window_border_color = Some(well_to_string(&fields.win_border_color));
    theme.button_border_enabled = Some(switch_on(&fields.btn_border_on));
    theme.button_border_width = field_string(&fields.btn_border_w).parse().ok();
    theme.button_border_color = Some(well_to_string(&fields.btn_border_color));
}

fn set_well_color(well: &NSColorWell, value: Option<&str>) {
    let rgb = value.and_then(parse_color).unwrap_or(crate::config::RgbColor::TRANSPARENT);
    unsafe { well.setColor(&ns_color_from_rgb(rgb)) };
}

fn well_to_string(well: &NSColorWell) -> String {
    let color = unsafe { well.color() };
    let srgb = unsafe { color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace()) };
    let Some(c) = srgb else {
        return "transparent".into();
    };
    let r = (unsafe { c.redComponent() } * 255.0).round().clamp(0.0, 255.0) as u8;
    let g = (unsafe { c.greenComponent() } * 255.0).round().clamp(0.0, 255.0) as u8;
    let b = (unsafe { c.blueComponent() } * 255.0).round().clamp(0.0, 255.0) as u8;
    let a = (unsafe { c.alphaComponent() } * 255.0).round().clamp(0.0, 255.0) as u8;
    color_to_string(crate::config::RgbColor::rgba(r, g, b, a))
}

fn ns_color_from_rgb(c: crate::config::RgbColor) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        c.r as f64 / 255.0,
        c.g as f64 / 255.0,
        c.b as f64 / 255.0,
        c.a as f64 / 255.0,
    )
}

fn action_summary(action: &Action) -> String {
    match action {
        Action::None => "无".into(),
        Action::Hotkey(keys) => format!(
            "热键 {}",
            keys.iter()
                .map(|k| format!("{k:?}").replace("Char(", "").replace(')', ""))
                .collect::<Vec<_>>()
                .join("+")
        ),
        Action::OpenApp { target } => format!("应用 {target}"),
        Action::Shell { .. } => "Shell".into(),
        Action::OpenUrl { url } => format!("URL {url}"),
        Action::OpenFile { path } => format!("文件 {path}"),
        Action::PasteText { .. } => "粘贴文本".into(),
        Action::MouseMacro(m) => format!("宏 {}步", m.steps.len()),
        Action::Script(s) => {
            if let Some(p) = &s.program {
                format!("脚本 {}步", p.steps.len())
            } else {
                "动作脚本".into()
            }
        },
    }
}

fn scope_label(scope: &ButtonScope) -> String {
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

fn nonempty(s: String) -> Option<String> {
    let t = s.trim().to_string();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}


fn button_is_configured(button: &ButtonConfig) -> bool {
    if button.enabled {
        return true;
    }
    if !button.label.trim().is_empty() {
        return true;
    }
    if button.icon.as_ref().is_some_and(|s| !s.trim().is_empty()) {
        return true;
    }
    !matches!(button.action, Action::None)
}

/// 当前焦点是否在文本编辑（含 NSTextField 的 field editor）。
fn text_field_is_editing() -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    let Some(win) = app.keyWindow() else {
        return false;
    };
    let Some(fr) = win.firstResponder() else {
        return false;
    };
    fr.isKindOfClass(NSTextView::class())
}


fn event_to_hotkey_spec(event: &NSEvent) -> Option<String> {
    let flags = event.modifierFlags();
    let chars = event
        .charactersIgnoringModifiers()
        .map(|s| s.to_string())
        .unwrap_or_default();
    let keycode = event.keyCode();

    // Ignore pure modifiers
    if chars.is_empty() && matches!(keycode, 54 | 55 | 56 | 57 | 58 | 59 | 60 | 61 | 62 | 63) {
        return None;
    }

    let mut parts = Vec::new();
    if flags.contains(NSEventModifierFlags::Command) {
        parts.push("Cmd");
    }
    if flags.contains(NSEventModifierFlags::Control) {
        parts.push("Ctrl");
    }
    if flags.contains(NSEventModifierFlags::Option) {
        parts.push("Alt");
    }
    if flags.contains(NSEventModifierFlags::Shift) {
        parts.push("Shift");
    }

    let key = keycode_to_name(keycode, &chars)?;
    if key.eq_ignore_ascii_case("Escape") {
        return None;
    }
    parts.push(key);
    Some(parts.join("+"))
}

fn keycode_to_name(keycode: u16, chars: &str) -> Option<&'static str> {
    match keycode {
        36 => Some("Enter"),
        48 => Some("Tab"),
        49 => Some("Space"),
        51 => Some("Backspace"),
        53 => Some("Escape"),
        117 => Some("Delete"),
        122 => Some("F1"),
        120 => Some("F2"),
        99 => Some("F3"),
        118 => Some("F4"),
        96 => Some("F5"),
        97 => Some("F6"),
        98 => Some("F7"),
        100 => Some("F8"),
        101 => Some("F9"),
        109 => Some("F10"),
        103 => Some("F11"),
        111 => Some("F12"),
        29 => Some("0"),
        18 => Some("1"),
        19 => Some("2"),
        20 => Some("3"),
        21 => Some("4"),
        23 => Some("5"),
        22 => Some("6"),
        26 => Some("7"),
        28 => Some("8"),
        25 => Some("9"),
        0 => Some("A"),
        11 => Some("B"),
        8 => Some("C"),
        2 => Some("D"),
        14 => Some("E"),
        3 => Some("F"),
        5 => Some("G"),
        4 => Some("H"),
        34 => Some("I"),
        38 => Some("J"),
        40 => Some("K"),
        37 => Some("L"),
        46 => Some("M"),
        45 => Some("N"),
        31 => Some("O"),
        35 => Some("P"),
        12 => Some("Q"),
        15 => Some("R"),
        1 => Some("S"),
        17 => Some("T"),
        32 => Some("U"),
        9 => Some("V"),
        13 => Some("W"),
        7 => Some("X"),
        16 => Some("Y"),
        6 => Some("Z"),
        43 => Some(","),
        47 => Some("."),
        44 => Some("/"),
        41 => Some(";"),
        39 => Some("'"),
        33 => Some("["),
        30 => Some("]"),
        42 => Some("\\"),
        27 => Some("-"),
        24 => Some("="),
        50 => Some("`"),
        _ => {
            let c = chars.chars().next()?;
            match c.to_ascii_uppercase() {
                'A'..='Z' | '0'..='9' => {
                    // Fallback via static map is hard; use common chars from keycode above.
                    None
                }
                ',' => Some(","),
                '.' => Some("."),
                '/' => Some("/"),
                ';' => Some(";"),
                '\'' => Some("'"),
                '[' => Some("["),
                ']' => Some("]"),
                '\\' => Some("\\"),
                '-' => Some("-"),
                '=' => Some("="),
                '`' => Some("`"),
                _ => None,
            }
        }
    }
}
