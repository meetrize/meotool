//! 按钮设置窗口（标签、图标、配色、快捷键、可见范围、启用）。

use std::cell::RefCell;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSButton, NSControlStateValueOff, NSControlStateValueOn, NSImageScaling,
    NSImageView, NSPopUpButton, NSScrollView, NSSwitch, NSTextField, NSView, NSWindow,
    NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize,
    NSString,
};

use crate::config::color::parse_color;
use crate::config::button_presets::{
    appearance_hint_text, apply_preset, preset_index_for, BUTTON_COLOR_PRESETS,
};
use crate::config::{ButtonConfig, ButtonScope, RgbColor};
use crate::platform::resolve_native_icon_tint;

use super::button_edit_session::ButtonEditSession;
use super::button_editor_common::{
    appearance_section, footer_btn, hotkey_row, icon_row, labeled, labeled_pair, load_icon_image,
    load_template_icon_image, ns_color_from_rgb, paint_view_background, popup, scope_bundle_row,
    scope_from_ui, set_well_color, style_preset_selection, switch_row, well_to_string,
    AppearanceUi, BUTTON_CORNER_RADIUS, CONTENT_PAD, FIELD_X,
};
use super::script_wizard::prompt_hotkey;
use super::events::{emit, IconPickTarget, NativeUiEvent};
use super::layout::{
    install_button_editor_chrome, layout_button_editor_chrome, scroll_document_to_top,
    BTN_EDITOR_FOOTER_H,
};

const APPEARANCE_DEFAULT_W: f64 = 560.0;
const APPEARANCE_MIN_W: f64 = 520.0;
const APPEARANCE_MIN_H: f64 = 480.0;
const FIELD_W: f64 = APPEARANCE_DEFAULT_W - FIELD_X - CONTENT_PAD;

#[derive(Default)]
struct AppearanceEditorTargetIvars;

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "MeoToolAppearanceEditorTarget"]
    #[ivars = AppearanceEditorTargetIvars]
    struct AppearanceEditorTarget;

    unsafe impl NSObjectProtocol for AppearanceEditorTarget {}

    unsafe impl NSWindowDelegate for AppearanceEditorTarget {
        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, _notification: &NSNotification) {
            layout_appearance_editor_live_now();
        }
    }

    impl AppearanceEditorTarget {
        #[unsafe(method(saveClicked:))]
        fn save_clicked(&self, _s: &AnyObject) {
            emit_appearance_editor(AppearanceEditorAction::Save);
        }
        #[unsafe(method(cancelClicked:))]
        fn cancel_clicked(&self, _s: &AnyObject) {
            emit_appearance_editor(AppearanceEditorAction::Cancel);
        }
        #[unsafe(method(pickIconClicked:))]
        fn pick_icon_clicked(&self, _s: &AnyObject) {
            emit_appearance_editor(AppearanceEditorAction::PickIcon);
        }
        #[unsafe(method(scopeModeChanged:))]
        fn scope_mode_changed(&self, _s: &AnyObject) {
            emit_appearance_editor(AppearanceEditorAction::ScopeModeChanged);
        }
        #[unsafe(method(pickAppClicked:))]
        fn pick_app_clicked(&self, _s: &AnyObject) {
            emit_appearance_editor(AppearanceEditorAction::PickApp);
        }
        #[unsafe(method(clearIconClicked:))]
        fn clear_icon_clicked(&self, _s: &AnyObject) {
            emit_appearance_editor(AppearanceEditorAction::ClearIcon);
        }
        #[unsafe(method(iconEdited:))]
        fn icon_edited(&self, _s: &AnyObject) {
            emit_appearance_editor(AppearanceEditorAction::IconEdited);
        }
        #[unsafe(method(presetClicked:))]
        fn preset_clicked(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            emit_appearance_editor(AppearanceEditorAction::AppearancePreset(tag as usize));
        }
        #[unsafe(method(bgColorChanged:))]
        fn bg_color_changed(&self, _s: &AnyObject) {
            emit_appearance_editor(AppearanceEditorAction::BgColorChanged);
        }
        #[unsafe(method(iconColorChanged:))]
        fn icon_color_changed(&self, _s: &AnyObject) {
            emit_appearance_editor(AppearanceEditorAction::IconColorChanged);
        }
        #[unsafe(method(inheritBgClicked:))]
        fn inherit_bg_clicked(&self, _s: &AnyObject) {
            emit_appearance_editor(AppearanceEditorAction::InheritBg);
        }
        #[unsafe(method(inheritIconClicked:))]
        fn inherit_icon_clicked(&self, _s: &AnyObject) {
            emit_appearance_editor(AppearanceEditorAction::InheritIcon);
        }
        #[unsafe(method(recordHotkeyClicked:))]
        fn record_hotkey_clicked(&self, _s: &AnyObject) {
            emit_appearance_editor(AppearanceEditorAction::RecordHotkey);
        }
        #[unsafe(method(clearHotkeyClicked:))]
        fn clear_hotkey_clicked(&self, _s: &AnyObject) {
            emit_appearance_editor(AppearanceEditorAction::ClearHotkey);
        }
    }
);

impl AppearanceEditorTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(AppearanceEditorTargetIvars);
        unsafe { msg_send![super(this), init] }
    }
}

#[derive(Debug, Clone)]
enum AppearanceEditorAction {
    Save,
    Cancel,
    PickIcon,
    ClearIcon,
    IconEdited,
    ScopeModeChanged,
    PickApp,
    AppearancePreset(usize),
    BgColorChanged,
    IconColorChanged,
    InheritBg,
    InheritIcon,
    RecordHotkey,
    ClearHotkey,
}

static APPEARANCE_EDITOR_TX: OnceLock<Mutex<Option<Sender<AppearanceEditorAction>>>> =
    OnceLock::new();

struct AppearanceEditorLiveLayout {
    window: Retained<NSWindow>,
    scroll: Retained<NSScrollView>,
    document: Retained<NSView>,
    footer: Retained<NSView>,
    save: Retained<NSButton>,
    cancel: Retained<NSButton>,
    content_h: f64,
}

thread_local! {
    static LIVE_APPEARANCE_LAYOUT: RefCell<Option<AppearanceEditorLiveLayout>> =
        const { RefCell::new(None) };
}

fn set_appearance_editor_live_layout(layout: Option<AppearanceEditorLiveLayout>) {
    LIVE_APPEARANCE_LAYOUT.with(|slot| *slot.borrow_mut() = layout);
}

fn layout_appearance_editor_live_now() {
    LIVE_APPEARANCE_LAYOUT.with(|slot| {
        let Ok(guard) = slot.try_borrow() else {
            return;
        };
        let Some(live) = guard.as_ref() else {
            return;
        };
        let Some(content) = live.window.contentView() else {
            return;
        };
        layout_button_editor_chrome(
            &content,
            &live.scroll,
            &live.document,
            &live.footer,
            Some(&live.save),
            Some(&live.cancel),
            None,
            0.0,
            0.0,
            live.content_h,
        );
    });
}

fn emit_appearance_editor(action: AppearanceEditorAction) {
    if let Ok(g) = APPEARANCE_EDITOR_TX.get_or_init(|| Mutex::new(None)).lock() {
        if let Some(tx) = g.as_ref() {
            let _ = tx.send(action);
        }
    }
}

pub struct AppearanceEditorWindow {
    window: Retained<NSWindow>,
    #[allow(dead_code)]
    target: Retained<AppearanceEditorTarget>,
    scroll: Retained<NSScrollView>,
    enabled: Retained<NSSwitch>,
    label: Retained<NSTextField>,
    icon: Retained<NSTextField>,
    icon_preview: Retained<NSImageView>,
    hotkey_field: Retained<NSTextField>,
    #[allow(dead_code)]
    hotkey_record_btn: Retained<NSButton>,
    #[allow(dead_code)]
    hotkey_clear_btn: Retained<NSButton>,
    scope_mode: Retained<NSPopUpButton>,
    scope_bundle_label: Retained<NSTextField>,
    scope_bundle: Retained<NSTextField>,
    scope_pick_btn: Retained<NSButton>,
    scope_app_label: Retained<NSTextField>,
    scope_app: Retained<NSTextField>,
    appearance: AppearanceUi,
    action_rx: Receiver<AppearanceEditorAction>,
    profile: usize,
    button: usize,
    draft: ButtonConfig,
    icon_dirs: Vec<std::path::PathBuf>,
    theme_button_bg: RgbColor,
    theme_icon_color: RgbColor,
    mtm: MainThreadMarker,
    front_held: bool,
    app_picking: bool,
}

impl AppearanceEditorWindow {
    pub fn new(mtm: MainThreadMarker) -> Self {
        let (tx, rx) = mpsc::channel();
        *APPEARANCE_EDITOR_TX
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap() = Some(tx);
        let target = AppearanceEditorTarget::new(mtm);
        let tg = &*target;
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;
        // 初始高度仅为占位；表单排完后按 content_h 收紧，避免竖向滚动条与底部空洞。
        let provisional_h = 640.0;
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc(),
                NSRect::new(
                    NSPoint::new(240.0, 120.0),
                    NSSize::new(APPEARANCE_DEFAULT_W, provisional_h),
                ),
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe {
            window.setReleasedWhenClosed(false);
            window.setTitle(&NSString::from_str("按钮设置"));
            window.setContentMinSize(NSSize::new(APPEARANCE_MIN_W, APPEARANCE_MIN_H));
            window.setDelegate(Some(ProtocolObject::from_ref(&*target)));
        }

        let content = window.contentView().unwrap();
        let chrome = install_button_editor_chrome(
            mtm,
            &content,
            NSSize::new(APPEARANCE_DEFAULT_W, provisional_h),
        );
        let document = chrome.document.clone();
        let scroll = chrome.scroll.clone();
        let footer = chrome.footer.clone();

        let save_btn = footer_btn(mtm, tg, "保存", objc2::sel!(saveClicked:));
        let cancel_btn = footer_btn(mtm, tg, "取消", objc2::sel!(cancelClicked:));
        unsafe {
            footer.addSubview(&save_btn);
            footer.addSubview(&cancel_btn);
        }

        let mut y = CONTENT_PAD;
        let enabled = switch_row(mtm, &document, "启用", &mut y, tg);
        let label = labeled(mtm, &document, "标签", FIELD_W, &mut y);
        let (icon, icon_preview) = icon_row(mtm, &document, FIELD_W, &mut y, tg);
        let (hotkey_field, hotkey_record_btn, hotkey_clear_btn) =
            hotkey_row(mtm, &document, FIELD_W, &mut y, tg);
        let appearance = appearance_section(mtm, &document, FIELD_W, &mut y, tg);

        y += 8.0;
        let scope_mode = popup(
            mtm,
            &document,
            "可见范围",
            &["始终", "仅当前应用"],
            FIELD_W,
            &mut y,
            tg,
            objc2::sel!(scopeModeChanged:),
        );
        let (scope_bundle_label, scope_bundle, scope_pick_btn) =
            scope_bundle_row(mtm, &document, FIELD_W, &mut y, tg);
        let (scope_app_label, scope_app) =
            labeled_pair(mtm, &document, "Scope 应用名", FIELD_W, &mut y);

        let content_h = y + CONTENT_PAD;
        let fitted_h = content_h + BTN_EDITOR_FOOTER_H;
        unsafe {
            window.setContentSize(NSSize::new(APPEARANCE_DEFAULT_W, fitted_h));
            window.setContentMinSize(NSSize::new(
                APPEARANCE_MIN_W,
                APPEARANCE_MIN_H.min(fitted_h),
            ));
        }

        set_appearance_editor_live_layout(Some(AppearanceEditorLiveLayout {
            window: window.clone(),
            scroll: scroll.clone(),
            document: document.clone(),
            footer: footer.clone(),
            save: save_btn.clone(),
            cancel: cancel_btn.clone(),
            content_h,
        }));
        layout_appearance_editor_live_now();

        let editor = Self {
            window,
            target,
            scroll,
            enabled,
            label,
            icon,
            icon_preview,
            hotkey_field,
            hotkey_record_btn,
            hotkey_clear_btn,
            scope_mode,
            scope_bundle_label,
            scope_bundle,
            scope_pick_btn,
            scope_app_label,
            scope_app,
            appearance,
            action_rx: rx,
            profile: 0,
            button: 0,
            draft: ButtonConfig {
                label: String::new(),
                icon: None,
                action: crate::config::Action::None,
                enabled: true,
                background_color: None,
                border_color: None,
                icon_color: None,
                scope: ButtonScope::Always,
                hotkey: None,
            },
            icon_dirs: Vec::new(),
            theme_button_bg: RgbColor::rgba(40, 40, 40, 217),
            theme_icon_color: RgbColor::rgb(224, 224, 224),
            mtm,
            front_held: false,
            app_picking: false,
        };
        editor.sync_scope_fields_visible();
        editor
    }

    pub fn open(&mut self, session: &ButtonEditSession) {
        self.profile = session.profile;
        self.button = session.button;
        self.draft = session.draft.clone();
        self.icon_dirs = session.icon_dirs.clone();
        self.theme_button_bg = session.theme_button_bg;
        self.theme_icon_color = session.theme_icon_color;
        self.set_app_picking(false);
        self.load();
        unsafe {
            self.window.setTitle(&NSString::from_str(&format!(
                "按钮设置 P{}#{}",
                session.profile, session.button
            )));
        }
        super::activation::present_utility_window(self.mtm, &self.window, &mut self.front_held);
        layout_appearance_editor_live_now();
        scroll_document_to_top(&self.scroll);
    }

    pub fn hide(&mut self) {
        self.set_app_picking(false);
        super::activation::dismiss_utility_window(self.mtm, &self.window, &mut self.front_held);
    }

    pub fn is_visible(&self) -> bool {
        unsafe { self.window.isVisible() }
    }

    pub fn is_app_picking(&self) -> bool {
        self.app_picking
    }

    pub fn set_app_picking(&mut self, picking: bool) {
        self.app_picking = picking;
        unsafe {
            self.scope_pick_btn.setTitle(&NSString::from_str(if picking {
                "取消"
            } else {
                "点选"
            }));
        }
    }

    pub fn apply_app_pick(&mut self, bundle_id: Option<String>, app_name: String) {
        unsafe {
            self.scope_mode.selectItemAtIndex(1);
            self.scope_bundle
                .setStringValue(&NSString::from_str(bundle_id.as_deref().unwrap_or("")));
            self.scope_app.setStringValue(&NSString::from_str(&app_name));
        }
        self.set_app_picking(false);
        self.sync_scope_fields_visible();
        super::activation::present_utility_window(self.mtm, &self.window, &mut self.front_held);
    }

    pub fn set_icon_name(&mut self, name: &str) {
        self.draft.icon = Some(name.to_string());
        unsafe {
            self.icon.setStringValue(&NSString::from_str(name));
        }
        self.refresh_appearance_preview();
    }

    /// 将表单字段写入 `session.draft`（保留 `draft.action` 不变）。
    pub fn collect(&self, session: &mut ButtonEditSession) {
        let enabled = unsafe { self.enabled.state() } == NSControlStateValueOn;
        let label = unsafe { self.label.stringValue().to_string() };
        let icon = {
            let s = unsafe { self.icon.stringValue().to_string() };
            let t = s.trim().to_string();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        };
        let when_active = unsafe { self.scope_mode.indexOfSelectedItem() } == 1;
        let bundle = unsafe { self.scope_bundle.stringValue().to_string() };
        let app = unsafe { self.scope_app.stringValue().to_string() };
        let scope = scope_from_ui(when_active, bundle, app);

        session.draft.label = label;
        session.draft.icon = icon;
        session.draft.enabled = enabled;
        session.draft.background_color = self.draft.background_color.clone();
        session.draft.border_color = self.draft.border_color.clone();
        session.draft.icon_color = self.draft.icon_color.clone();
        session.draft.scope = scope;
        session.draft.hotkey = self.draft.hotkey.clone();
    }

    fn merge_ui_into_draft(&mut self) {
        let enabled = unsafe { self.enabled.state() } == NSControlStateValueOn;
        self.draft.enabled = enabled;
        self.draft.label = unsafe { self.label.stringValue().to_string() };
        let icon_s = unsafe { self.icon.stringValue().to_string() };
        let icon_t = icon_s.trim().to_string();
        self.draft.icon = if icon_t.is_empty() {
            None
        } else {
            Some(icon_t)
        };
        let when_active = unsafe { self.scope_mode.indexOfSelectedItem() } == 1;
        let bundle = unsafe { self.scope_bundle.stringValue().to_string() };
        let app = unsafe { self.scope_app.stringValue().to_string() };
        self.draft.scope = scope_from_ui(when_active, bundle, app);
    }

    pub fn poll(&mut self, session: &mut ButtonEditSession) {
        while let Ok(action) = self.action_rx.try_recv() {
            match action {
                AppearanceEditorAction::Save => {
                    self.merge_ui_into_draft();
                    self.collect(session);
                    emit(NativeUiEvent::SaveButton {
                        profile: session.profile,
                        button: session.button,
                        config: session.draft.clone(),
                    });
                    self.hide();
                }
                AppearanceEditorAction::Cancel => {
                    emit(NativeUiEvent::CancelButtonEditor);
                    self.hide();
                }
                AppearanceEditorAction::PickIcon => {
                    emit(NativeUiEvent::OpenIconPicker(IconPickTarget::Button {
                        profile: self.profile,
                        button: self.button,
                    }));
                }
                AppearanceEditorAction::ClearIcon => self.clear_icon(),
                AppearanceEditorAction::IconEdited => self.refresh_appearance_preview(),
                AppearanceEditorAction::ScopeModeChanged => {
                    if unsafe { self.scope_mode.indexOfSelectedItem() } != 1 && self.app_picking {
                        emit(NativeUiEvent::CancelAppPick);
                    }
                    self.sync_scope_fields_visible();
                }
                AppearanceEditorAction::PickApp => {
                    if self.app_picking {
                        emit(NativeUiEvent::CancelAppPick);
                    } else {
                        emit(NativeUiEvent::StartAppPick);
                    }
                }
                AppearanceEditorAction::AppearancePreset(index) => self.apply_preset(index),
                AppearanceEditorAction::BgColorChanged => self.sync_bg_from_well(),
                AppearanceEditorAction::IconColorChanged => self.sync_icon_from_well(),
                AppearanceEditorAction::InheritBg => self.inherit_background(),
                AppearanceEditorAction::InheritIcon => self.inherit_icon_color(),
                AppearanceEditorAction::RecordHotkey => self.record_hotkey(),
                AppearanceEditorAction::ClearHotkey => self.clear_hotkey(),
            }
        }
    }

    fn load(&mut self) {
        unsafe {
            self.enabled.setState(if self.draft.enabled {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            self.label
                .setStringValue(&NSString::from_str(&self.draft.label));
            self.icon.setStringValue(&NSString::from_str(
                self.draft.icon.as_deref().unwrap_or(""),
            ));
            match &self.draft.scope {
                ButtonScope::Always => {
                    self.scope_mode.selectItemAtIndex(0);
                    self.scope_bundle.setStringValue(&NSString::from_str(""));
                    self.scope_app.setStringValue(&NSString::from_str(""));
                }
                ButtonScope::WhenActive(rule) => {
                    self.scope_mode.selectItemAtIndex(1);
                    self.scope_bundle.setStringValue(&NSString::from_str(
                        rule.bundle_id.as_deref().unwrap_or(""),
                    ));
                    self.scope_app.setStringValue(&NSString::from_str(
                        rule.app_name.as_deref().unwrap_or(""),
                    ));
                }
            }
        }
        self.sync_hotkey_field();
        self.refresh_appearance_preview();
        self.sync_scope_fields_visible();
    }

    fn sync_hotkey_field(&self) {
        let text = self
            .draft
            .hotkey
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("未设置");
        unsafe {
            self.hotkey_field
                .setStringValue(&NSString::from_str(text));
        }
    }

    fn record_hotkey(&mut self) {
        if let Some(spec) = prompt_hotkey(self.mtm) {
            let trimmed = spec.trim().to_string();
            if trimmed.is_empty() {
                self.draft.hotkey = None;
            } else {
                self.draft.hotkey = Some(trimmed);
            }
            self.sync_hotkey_field();
        }
        super::activation::present_utility_window(self.mtm, &self.window, &mut self.front_held);
    }

    fn clear_hotkey(&mut self) {
        self.draft.hotkey = None;
        self.sync_hotkey_field();
    }

    fn clear_icon(&mut self) {
        self.draft.icon = None;
        unsafe {
            self.icon.setStringValue(&NSString::from_str(""));
        }
        self.refresh_appearance_preview();
    }

    fn refresh_appearance_preview(&self) {
        let name = unsafe { self.icon.stringValue().to_string() };
        let thumb = load_icon_image(self.mtm, &self.icon_dirs, name.trim());
        unsafe {
            self.icon_preview.setImage(thumb.as_deref());
        }

        let bg = self
            .draft
            .background_color
            .as_deref()
            .and_then(parse_color)
            .unwrap_or(self.theme_button_bg);
        paint_view_background(&self.appearance.preview, bg, BUTTON_CORNER_RADIUS);

        let enabled = self.draft.enabled;
        let icon_tint = resolve_native_icon_tint(
            self.draft.icon_color.as_deref(),
            self.theme_icon_color,
            enabled,
        );
        unsafe {
            self.appearance.preview_button.setBordered(false);
            self.appearance
                .preview_button
                .setContentTintColor(Some(&ns_color_from_rgb(icon_tint)));
            if let Some(img) = load_template_icon_image(self.mtm, &self.icon_dirs, name.trim()) {
                img.setSize(NSSize::new(28.0, 28.0));
                self.appearance.preview_button.setImage(Some(&img));
                self.appearance
                    .preview_button
                    .setImagePosition(objc2_app_kit::NSCellImagePosition::ImageOnly);
                self.appearance
                    .preview_button
                    .setImageScaling(NSImageScaling::ScaleProportionallyDown);
            } else {
                self.appearance.preview_button.setImage(None);
            }
        }

        let hint = appearance_hint_text(
            self.draft.background_color.is_none(),
            self.draft.icon_color.is_none(),
        );
        unsafe {
            self.appearance
                .appearance_hint
                .setStringValue(&NSString::from_str(hint));
        }
        self.sync_preset_selection();
        self.sync_color_wells();
    }

    fn sync_color_wells(&self) {
        let bg = self
            .draft
            .background_color
            .as_deref()
            .and_then(parse_color)
            .unwrap_or(self.theme_button_bg);
        let icon = self
            .draft
            .icon_color
            .as_deref()
            .and_then(parse_color)
            .unwrap_or(self.theme_icon_color);
        set_well_color(&self.appearance.bg_well, bg);
        set_well_color(&self.appearance.icon_well, icon);
    }

    fn sync_preset_selection(&self) {
        let selected = preset_index_for(
            self.draft.background_color.as_deref(),
            self.draft.icon_color.as_deref(),
        );
        for (i, btn) in self.appearance.preset_buttons.iter().enumerate() {
            style_preset_selection(btn, selected == Some(i));
        }
    }

    fn apply_preset(&mut self, index: usize) {
        if index >= BUTTON_COLOR_PRESETS.len() {
            return;
        }
        apply_preset(&mut self.draft, index);
        self.refresh_appearance_preview();
    }

    fn inherit_background(&mut self) {
        self.draft.background_color = None;
        self.refresh_appearance_preview();
    }

    fn inherit_icon_color(&mut self) {
        self.draft.icon_color = None;
        self.refresh_appearance_preview();
    }

    fn sync_bg_from_well(&mut self) {
        self.draft.background_color = Some(well_to_string(&self.appearance.bg_well));
        self.refresh_appearance_preview();
    }

    fn sync_icon_from_well(&mut self) {
        self.draft.icon_color = Some(well_to_string(&self.appearance.icon_well));
        self.refresh_appearance_preview();
    }

    fn sync_scope_fields_visible(&self) {
        let when_active = unsafe { self.scope_mode.indexOfSelectedItem() } == 1;
        let hidden = !when_active;
        unsafe {
            self.scope_bundle_label.setHidden(hidden);
            self.scope_bundle.setHidden(hidden);
            self.scope_pick_btn.setHidden(hidden);
            self.scope_app_label.setHidden(hidden);
            self.scope_app.setHidden(hidden);
        }
    }
}
