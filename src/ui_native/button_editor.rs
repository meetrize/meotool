//! 原生按钮脚本编辑窗口。

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, ClassType, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSButton, NSEvent, NSEventMask, NSEventModifierFlags,
    NSScrollView, NSTextField, NSTextView, NSView, NSWindow, NSWindowDelegate,
    NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize,
    NSString,
};

use crate::action::{
    action_attached_macro, action_needs_legacy_upgrade, action_to_meoflow, compile,
    macro_to_meoflow, step_preview_label,
};
use crate::config::script_presets::{
    is_comments_only, preset_by_id, render_template, PresetBuildParams, PresetParamKind,
    DEFAULT_SCRIPT_SKELETON, FLOW_PRESETS, QUICK_PRESETS, ScriptPreset,
};
use crate::config::{Action, ActionScript, MouseMacro};

use super::button_edit_session::ButtonEditSession;
use super::button_editor_common::{footer_btn, show_alert};
use super::events::{emit, NativeUiEvent};
use super::layout::{
    install_script_editor_chrome, layout_script_editor_chrome, scroll_document_to_top,
    CONTENT_PAD, SCRIPT_EDITOR_DEFAULT_H, SCRIPT_EDITOR_DEFAULT_W, SCRIPT_EDITOR_MIN_H,
    SCRIPT_EDITOR_MIN_W,
};
use super::meoflow_highlight::error_line_numbers;
use super::script_editor_pane::{layout_editor_row, ScriptEditorPane};
use super::script_editor_toolbar::{install_toolbar_strip, SCRIPT_TOOLBAR_H};
use super::script_wizard::{
    pick_app_path, pick_open_file_path, pick_script_export_path, pick_script_import,
    prompt_flow_insert_choice, prompt_hotkey, prompt_open_app_choice, prompt_script_import_choice,
    prompt_shell, prompt_text, prompt_url, prompt_wait_ms, show_syntax_help, write_script_file,
    FlowInsertChoice, OpenAppChoice,
};

#[derive(Default)]
struct EditorTargetIvars;

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "MeoToolScriptEditorTarget"]
    #[ivars = EditorTargetIvars]
    struct EditorTarget;

    unsafe impl NSObjectProtocol for EditorTarget {}

    unsafe impl NSWindowDelegate for EditorTarget {
        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, _notification: &NSNotification) {
            layout_script_editor_live_now();
        }
    }

    impl EditorTarget {
        #[unsafe(method(saveClicked:))]
        fn save_clicked(&self, _s: &AnyObject) {
            emit_editor(EditorAction::Save);
        }
        #[unsafe(method(cancelClicked:))]
        fn cancel_clicked(&self, _s: &AnyObject) {
            emit_editor(EditorAction::Cancel);
        }
        #[unsafe(method(quickPresetClicked:))]
        fn quick_preset_clicked(&self, sender: &AnyObject) {
            let selected: isize = unsafe { msg_send![sender, selectedSegment] };
            if selected < 0 {
                return;
            }
            emit_editor(EditorAction::QuickPreset(selected as usize));
        }
        #[unsafe(method(flowPresetClicked:))]
        fn flow_preset_clicked(&self, sender: &AnyObject) {
            let selected: isize = unsafe { msg_send![sender, selectedSegment] };
            if selected < 0 {
                return;
            }
            emit_editor(EditorAction::FlowPreset(selected as usize));
        }
        #[unsafe(method(scriptTryRunClicked:))]
        fn script_try_run_clicked(&self, _s: &AnyObject) {
            emit_editor(EditorAction::ScriptTryRun);
        }
        #[unsafe(method(scriptCaptureClicked:))]
        fn script_capture_clicked(&self, _s: &AnyObject) {
            emit_editor(EditorAction::ScriptCaptureClick);
        }
        #[unsafe(method(scriptCaptureAppClicked:))]
        fn script_capture_app_clicked(&self, _s: &AnyObject) {
            emit_editor(EditorAction::ScriptCaptureApp);
        }
        #[unsafe(method(scriptFinishRecordClicked:))]
        fn script_finish_record_clicked(&self, _s: &AnyObject) {
            emit_editor(EditorAction::ScriptFinishRecord);
        }
        #[unsafe(method(syntaxHelpClicked:))]
        fn syntax_help_clicked(&self, _s: &AnyObject) {
            emit_editor(EditorAction::SyntaxHelp);
        }
        #[unsafe(method(scriptIoClicked:))]
        fn script_io_clicked(&self, sender: &AnyObject) {
            let selected: isize = unsafe { msg_send![sender, selectedSegment] };
            match selected {
                0 => emit_editor(EditorAction::ImportScript),
                1 => emit_editor(EditorAction::ExportScript),
                _ => {}
            }
        }
    }
);

impl EditorTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(EditorTargetIvars::default());
        unsafe { msg_send![super(this), init] }
    }
}

#[derive(Debug, Clone)]
enum EditorAction {
    Save,
    Cancel,
    QuickPreset(usize),
    FlowPreset(usize),
    ScriptTryRun,
    ScriptCaptureClick,
    ScriptCaptureApp,
    ScriptFinishRecord,
    SyntaxHelp,
    ImportScript,
    ExportScript,
    RefreshHighlight,
}

/// 流程模板多步 wizard 状态。
#[derive(Debug, Clone)]
enum PendingFlowWizard {
    ClickHotkey {
        params: PresetBuildParams,
    },
    OpenAppType {
        params: PresetBuildParams,
    },
}

const SCRIPT_VIEW_MIN_H: f64 = 200.0;

static EDITOR_TX: OnceLock<Mutex<Option<Sender<EditorAction>>>> = OnceLock::new();

struct ScriptEditorLiveLayout {
    window: Retained<NSWindow>,
    toolbar: Retained<NSView>,
    scroll: Retained<NSScrollView>,
    document: Retained<NSView>,
    status_bar: Retained<NSView>,
    footer: Retained<NSView>,
    save: Retained<NSButton>,
    cancel: Retained<NSButton>,
    cursor: Retained<NSTextField>,
    status: Retained<NSTextField>,
    preview: Retained<NSTextField>,
    editor_row: Retained<NSView>,
    gutter_scroll: Retained<NSScrollView>,
    script_scroll: Retained<NSScrollView>,
    script_view: Retained<NSTextView>,
    gutter_view: Retained<NSTextView>,
    script_y: f64,
    script_min_h: f64,
    content_h: f64,
}

thread_local! {
    static LIVE_SCRIPT_LAYOUT: RefCell<Option<ScriptEditorLiveLayout>> = const { RefCell::new(None) };
}

fn set_script_editor_live_layout(layout: Option<ScriptEditorLiveLayout>) {
    LIVE_SCRIPT_LAYOUT.with(|slot| *slot.borrow_mut() = layout);
}

fn layout_script_editor_live_now() {
    LIVE_SCRIPT_LAYOUT.with(|slot| {
        let Ok(guard) = slot.try_borrow() else {
            return;
        };
        let Some(live) = guard.as_ref() else {
            return;
        };
        let Some(content) = live.window.contentView() else {
            return;
        };
        let field_w = {
            let b = content.bounds();
            (b.size.width.max(SCRIPT_EDITOR_MIN_W) - CONTENT_PAD * 2.0).max(200.0)
        };
        let script_h = {
            let b = content.bounds();
            let h = b.size.height.max(SCRIPT_EDITOR_MIN_H);
            let bottom_chrome = super::layout::BTN_EDITOR_FOOTER_H
                + super::layout::SCRIPT_EDITOR_STATUS_H
                + SCRIPT_TOOLBAR_H;
            let body_h = (h - bottom_chrome).max(1.0);
            let bonus = (body_h - live.content_h).max(0.0);
            live.script_min_h + bonus
        };
        layout_script_editor_chrome(
            &content,
            Some(&live.toolbar),
            &live.scroll,
            &live.document,
            &live.status_bar,
            &live.footer,
            Some(&live.save),
            Some(&live.cancel),
            Some(&live.cursor),
            Some(&live.status),
            Some(&live.preview),
            live.script_y,
            live.script_min_h,
            live.content_h,
            None,
            Some(&live.editor_row),
            SCRIPT_TOOLBAR_H,
        );
        layout_editor_row(
            &live.editor_row,
            &live.gutter_scroll,
            &live.script_scroll,
            &live.script_view,
            &live.gutter_view,
            field_w,
            script_h,
        );
    });
}

fn emit_editor(action: EditorAction) {
    if let Ok(g) = EDITOR_TX.get_or_init(|| Mutex::new(None)).lock() {
        if let Some(tx) = g.as_ref() {
            let _ = tx.send(action);
        }
    }
}

pub struct ScriptEditorWindow {
    window: Retained<NSWindow>,
    #[allow(dead_code)]
    target: Retained<EditorTarget>,
    scroll: Retained<NSScrollView>,
    pane: ScriptEditorPane,
    cursor_pos: Retained<NSTextField>,
    script_status: Retained<NSTextField>,
    script_preview: Retained<NSTextField>,
    capture_finish_btn: Retained<NSButton>,
    action_rx: Receiver<EditorAction>,
    profile: usize,
    button: usize,
    mtm: MainThreadMarker,
    front_held: bool,
    script_app_picking: bool,
    script_open_app_picking: bool,
    capturing_click: bool,
    macro_recording: bool,
    pending_flow: Option<PendingFlowWizard>,
    flow_replace_all: bool,
    key_monitor: Option<Retained<AnyObject>>,
    /// 文本框为 firstResponder 时记住的光标/选区；点工具栏后仍用于插入。
    last_caret: Cell<NSRange>,
}

/// 兼容旧名。
impl ScriptEditorWindow {
    pub fn new(mtm: MainThreadMarker) -> Self {
        let (tx, rx) = mpsc::channel();
        *EDITOR_TX.get_or_init(|| Mutex::new(None)).lock().unwrap() = Some(tx);
        let target = EditorTarget::new(mtm);
        let tg = &*target;
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc(),
                NSRect::new(
                    NSPoint::new(200.0, 80.0),
                    NSSize::new(SCRIPT_EDITOR_DEFAULT_W, SCRIPT_EDITOR_DEFAULT_H),
                ),
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe {
            window.setReleasedWhenClosed(false);
            window.setTitle(&NSString::from_str("编辑脚本"));
            window.setContentMinSize(NSSize::new(SCRIPT_EDITOR_MIN_W, SCRIPT_EDITOR_MIN_H));
            window.setDelegate(Some(ProtocolObject::from_ref(&*target)));
        }

        let content = window.contentView().unwrap();
        let chrome = install_script_editor_chrome(
            mtm,
            &content,
            NSSize::new(SCRIPT_EDITOR_DEFAULT_W, SCRIPT_EDITOR_DEFAULT_H),
        );
        let document = chrome.document.clone();
        let scroll = chrome.scroll.clone();
        let status_bar = chrome.status_bar.clone();
        let footer = chrome.footer.clone();

        let toolbar_strip =
            install_toolbar_strip(mtm, &content, SCRIPT_EDITOR_DEFAULT_W, tg as &AnyObject);

        let save_btn = footer_btn(mtm, tg, "完成", objc2::sel!(saveClicked:));
        let cancel_btn = footer_btn(mtm, tg, "取消", objc2::sel!(cancelClicked:));
        unsafe {
            footer.addSubview(&save_btn);
            footer.addSubview(&cancel_btn);
        }

        let status_font = objc2_app_kit::NSFont::userFixedPitchFontOfSize(11.0)
            .unwrap_or_else(|| objc2_app_kit::NSFont::systemFontOfSize(11.0));
        let cursor_pos = status_label(mtm, &status_bar, "Ln 1, Col 1", &status_font);
        let script_status = status_label(mtm, &status_bar, "—", &status_font);
        let script_preview = status_label(mtm, &status_bar, "—", &status_font);
        unsafe {
            script_preview.setMaximumNumberOfLines(1);
        }

        let script_y = CONTENT_PAD;
        let field_w = SCRIPT_EDITOR_DEFAULT_W - CONTENT_PAD * 2.0;
        let on_text_changed =
            block2::RcBlock::new(move |_n: NonNull<NSNotification>| {
                emit_editor(EditorAction::RefreshHighlight);
            });
        let pane = ScriptEditorPane::install(
            mtm,
            &document,
            CONTENT_PAD,
            script_y,
            field_w,
            SCRIPT_VIEW_MIN_H,
            &on_text_changed,
        );
        let content_h = script_y + SCRIPT_VIEW_MIN_H + CONTENT_PAD;

        set_script_editor_live_layout(Some(ScriptEditorLiveLayout {
            window: window.clone(),
            toolbar: toolbar_strip.view.clone(),
            scroll: scroll.clone(),
            document: document.clone(),
            status_bar: status_bar.clone(),
            footer: footer.clone(),
            save: save_btn.clone(),
            cancel: cancel_btn.clone(),
            cursor: cursor_pos.clone(),
            status: script_status.clone(),
            preview: script_preview.clone(),
            editor_row: pane.row.clone(),
            gutter_scroll: pane.gutter_scroll(),
            script_scroll: pane.script_scroll.clone(),
            script_view: pane.script_view.clone(),
            gutter_view: pane.gutter_view(),
            script_y,
            script_min_h: SCRIPT_VIEW_MIN_H,
            content_h,
        }));

        let editor = Self {
            window,
            target,
            scroll,
            pane,
            cursor_pos,
            script_status,
            script_preview,
            capture_finish_btn: toolbar_strip.capture_finish_btn,
            action_rx: rx,
            profile: 0,
            button: 0,
            mtm,
            front_held: false,
            script_app_picking: false,
            script_open_app_picking: false,
            capturing_click: false,
            macro_recording: false,
            pending_flow: None,
            flow_replace_all: false,
            key_monitor: None,
            last_caret: Cell::new(NSRange::new(0, 0)),
        };
        layout_script_editor_live_now();
        editor
    }

    pub fn open(&mut self, session: &mut ButtonEditSession) {
        self.profile = session.profile;
        self.button = session.button;
        self.set_script_app_picking(false);
        self.set_script_open_app_picking(false);
        self.macro_recording = false;
        self.pending_flow = None;
        self.sync_macro_record_ui();
        self.load(session);
        unsafe {
            self.window.setTitle(&NSString::from_str(&format!(
                "编辑脚本 P{}#{}",
                session.profile, session.button
            )));
        }
        super::activation::present_utility_window(self.mtm, &self.window, &mut self.front_held);
        layout_script_editor_live_now();
        scroll_document_to_top(&self.scroll);
        self.install_key_monitor();
    }

    pub fn hide(&mut self) {
        self.set_script_app_picking(false);
        self.set_script_open_app_picking(false);
        self.remove_key_monitor();
        self.pane.remove_observers();
        super::activation::dismiss_utility_window(self.mtm, &self.window, &mut self.front_held);
    }

    /// 试运行时临时收起编辑窗（保留内容与监视器），避免挡住目标应用。
    pub fn withdraw_for_try_run(&mut self) {
        super::activation::dismiss_utility_window(self.mtm, &self.window, &mut self.front_held);
    }

    pub fn restore_after_try_run(&mut self) {
        super::activation::present_utility_window(self.mtm, &self.window, &mut self.front_held);
        layout_script_editor_live_now();
    }

    pub fn set_try_run_status(&self, message: &str) {
        unsafe {
            self.script_status
                .setStringValue(&NSString::from_str(message));
        }
    }

    fn install_key_monitor(&mut self) {
        if self.key_monitor.is_some() {
            return;
        }
        let block = block2::RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
            let event = unsafe { event.as_ref() };
            let Some(mtm) = MainThreadMarker::new() else {
                return event as *const NSEvent as *mut NSEvent;
            };
            let flags = event.modifierFlags();
            let independent = flags & NSEventModifierFlags::DeviceIndependentFlagsMask;
            if !independent.contains(NSEventModifierFlags::Command)
                || independent.contains(NSEventModifierFlags::Option)
                || independent.contains(NSEventModifierFlags::Control)
            {
                return event as *const NSEvent as *mut NSEvent;
            }
            let shift = independent.contains(NSEventModifierFlags::Shift);
            let chars = event
                .charactersIgnoringModifiers()
                .map(|s| s.to_string())
                .unwrap_or_default();
            let key = chars.to_ascii_lowercase();
            let action = match (key.as_str(), shift) {
                ("c", false) => Some(objc2::sel!(copy:)),
                ("x", false) => Some(objc2::sel!(cut:)),
                ("v", false) => Some(objc2::sel!(paste:)),
                ("a", false) => Some(objc2::sel!(selectAll:)),
                ("z", false) => Some(objc2::sel!(undo:)),
                ("z", true) => Some(objc2::sel!(redo:)),
                _ => None,
            };
            let Some(action) = action else {
                return event as *const NSEvent as *mut NSEvent;
            };

            let app = NSApplication::sharedApplication(mtm);
            let Some(win) = app.keyWindow() else {
                return event as *const NSEvent as *mut NSEvent;
            };
            let Some(fr) = win.firstResponder() else {
                return event as *const NSEvent as *mut NSEvent;
            };
            if !fr.isKindOfClass(NSTextView::class()) {
                return event as *const NSEvent as *mut NSEvent;
            }

            let handled = unsafe { app.sendAction_to_from(action, None, None) };
            if handled {
                std::ptr::null_mut()
            } else {
                event as *const NSEvent as *mut NSEvent
            }
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

    pub fn is_visible(&self) -> bool {
        unsafe { self.window.isVisible() }
    }

    pub fn set_script_app_picking(&mut self, picking: bool) {
        self.script_app_picking = picking;
        if picking {
            self.script_open_app_picking = false;
            self.capturing_click = false;
            unsafe {
                self.script_status.setStringValue(&NSString::from_str(
                    "捕获中… 请点击目标应用窗口（Esc 取消）",
                ));
            }
        } else if !self.capturing_click && !self.macro_recording {
            self.refresh_script_status(None);
        }
    }

    pub fn set_script_open_app_picking(&mut self, picking: bool) {
        self.script_open_app_picking = picking;
        if picking {
            self.script_app_picking = false;
            self.capturing_click = false;
            unsafe {
                self.script_status.setStringValue(&NSString::from_str(
                    "请选择目标应用窗口（Esc 取消）",
                ));
            }
        } else if !self.capturing_click && !self.macro_recording {
            self.refresh_script_status(None);
        }
    }

    pub fn is_script_open_app_picking(&self) -> bool {
        self.script_open_app_picking
    }

    fn sync_macro_record_ui(&self) {
        unsafe {
            self.capture_finish_btn
                .setHidden(!self.macro_recording);
        }
    }

    pub fn apply_app_pick(&mut self, bundle_id: Option<String>, app_name: String) {
        if self.script_open_app_picking {
            let target = bundle_id
                .filter(|s| !s.trim().is_empty())
                .or_else(|| {
                    let t = app_name.trim();
                    (!t.is_empty()).then(|| t.to_string())
                });
            if self.pending_flow.is_some() {
                self.continue_open_app_type_wizard(target);
            } else if let Some(app) = target {
                let fragment = render_template(
                    "打开应用 \"{app}\"\n",
                    &PresetBuildParams {
                        app: Some(app),
                        ..Default::default()
                    },
                );
                self.insert_script_fragment(&fragment, false);
            } else {
                unsafe {
                    self.script_status.setStringValue(&NSString::from_str(
                        "状态：未能识别应用，请重试",
                    ));
                }
            }
            self.set_script_open_app_picking(false);
            super::activation::present_utility_window(
                self.mtm,
                &self.window,
                &mut self.front_held,
            );
            return;
        }
        if self.script_app_picking {
            match bundle_id.filter(|s| !s.trim().is_empty()) {
                Some(id) => self.insert_script_activate(&id),
                None => {
                    self.set_script_app_picking(false);
                    unsafe {
                        self.script_status.setStringValue(&NSString::from_str(
                            "状态：未能识别 Bundle ID，请重试",
                        ));
                    }
                }
            }
            super::activation::present_utility_window(
                self.mtm,
                &self.window,
                &mut self.front_held,
            );
        }
    }

    fn script_view(&self) -> &NSTextView {
        &self.pane.script_view
    }

    pub fn poll(&mut self, session: &mut ButtonEditSession) {
        while let Ok(action) = self.action_rx.try_recv() {
            match action {
                EditorAction::Save => match self.collect(session) {
                    Ok(()) => {
                        emit(NativeUiEvent::SaveButton {
                            profile: session.profile,
                            button: session.button,
                            config: session.draft.clone(),
                        });
                        self.hide();
                    }
                    Err(msg) => show_alert(self.mtm, "无法保存", &msg),
                },
                EditorAction::Cancel => {
                    emit(NativeUiEvent::CancelButtonEditor);
                    self.hide();
                }
                EditorAction::QuickPreset(index) => self.run_quick_preset(index),
                EditorAction::FlowPreset(index) => self.run_flow_preset(index),
                EditorAction::ScriptTryRun => self.try_run_script(session),
                EditorAction::ScriptCaptureClick => self.start_click_capture(false),
                EditorAction::ScriptCaptureApp => {
                    if self.script_app_picking {
                        emit(NativeUiEvent::CancelAppPick);
                    } else {
                        if self.capturing_click {
                            emit(NativeUiEvent::CancelScriptClickCapture);
                        }
                        self.set_script_app_picking(true);
                        emit(NativeUiEvent::StartAppPick);
                    }
                }
                EditorAction::ScriptFinishRecord => {
                    emit(NativeUiEvent::FinishScriptMacroRecord);
                }
                EditorAction::SyntaxHelp => show_syntax_help(self.mtm),
                EditorAction::ImportScript => self.import_script(),
                EditorAction::ExportScript => self.export_script(),
                EditorAction::RefreshHighlight => self.refresh_highlight_only(session),
            }
        }
    }

    fn refresh_highlight_only(&mut self, session: &mut ButtonEditSession) {
        self.refresh_cursor_position();
        let source = self.script_source();
        let attached = session.attached_macro.clone();
        match compile(&source, attached.as_ref()) {
            Ok(_) => self.pane.set_error_lines(vec![]),
            Err(err) => self.pane.set_error_lines(error_line_numbers(&err)),
        }
        self.pane.apply_highlight();
    }

    fn run_quick_preset(&mut self, index: usize) {
        let Some(preset) = QUICK_PRESETS.get(index) else {
            return;
        };
        self.run_preset(preset);
    }

    fn run_flow_preset(&mut self, index: usize) {
        let Some(preset) = FLOW_PRESETS.get(index) else {
            return;
        };
        let source = self.script_source();
        let meaningful = !source.trim().is_empty() && !is_comments_only(&source);
        self.flow_replace_all = if meaningful {
            match prompt_flow_insert_choice(self.mtm) {
                Some(FlowInsertChoice::ReplaceAll) => true,
                Some(FlowInsertChoice::Append) => false,
                None => return,
            }
        } else {
            false
        };

        match preset.id {
            "type_enter" | "paste_enter" => {
                let title = if preset.id == "type_enter" {
                    "输入文字"
                } else {
                    "粘贴文字"
                };
                // 输入后回车：短文本用单行；粘贴后回车：允许多行
                let multiline = preset.id == "paste_enter";
                let Some(text) = prompt_text(
                    self.mtm,
                    title,
                    "将写入脚本并可在编辑器中继续修改",
                    multiline,
                    "",
                ) else {
                    return;
                };
                let fragment = render_template(
                    preset.template,
                    &PresetBuildParams {
                        text: Some(text),
                        ..Default::default()
                    },
                );
                self.insert_script_fragment(&fragment, self.flow_replace_all);
            }
            "click_hotkey" => {
                self.pending_flow = Some(PendingFlowWizard::ClickHotkey {
                    params: PresetBuildParams::default(),
                });
                self.start_click_capture(true);
            }
            "open_app_type" => {
                self.pending_flow = Some(PendingFlowWizard::OpenAppType {
                    params: PresetBuildParams::default(),
                });
                self.begin_open_app_for_flow();
            }
            _ => {}
        }
    }

    fn begin_open_app_for_flow(&mut self) {
        match prompt_open_app_choice(self.mtm) {
            Some(OpenAppChoice::BrowseApp) => {
                if let Some(path) = pick_app_path(self.mtm) {
                    self.continue_open_app_type_wizard(Some(path));
                } else {
                    self.pending_flow = None;
                }
            }
            Some(OpenAppChoice::PickWindow) => {
                self.set_script_open_app_picking(true);
                emit(NativeUiEvent::StartAppPick);
            }
            None => self.pending_flow = None,
        }
    }

    fn continue_open_app_type_wizard(&mut self, app: Option<String>) {
        let Some(app) = app.filter(|s| !s.trim().is_empty()) else {
            self.pending_flow = None;
            return;
        };
        let Some(PendingFlowWizard::OpenAppType { mut params }) = self.pending_flow.take() else {
            return;
        };
        params.app = Some(app);
        let Some(text) = prompt_text(
            self.mtm,
            "输入文字",
            "应用打开后将输入这段文字",
            true,
            "",
        ) else {
            return;
        };
        params.text = Some(text);
        let preset = preset_by_id("open_app_type").unwrap();
        let fragment = render_template(preset.template, &params);
        self.insert_script_fragment(&fragment, self.flow_replace_all);
    }

    fn run_preset(&mut self, preset: &ScriptPreset) {
        match preset.param {
            PresetParamKind::HotkeyCapture => {
                if let Some(hotkey) = prompt_hotkey(self.mtm) {
                    let fragment = render_template(
                        preset.template,
                        &PresetBuildParams {
                            hotkey: Some(hotkey),
                            ..Default::default()
                        },
                    );
                    self.insert_script_fragment(&fragment, false);
                }
            }
            PresetParamKind::OpenAppPanel => match prompt_open_app_choice(self.mtm) {
                Some(OpenAppChoice::BrowseApp) => {
                    if let Some(path) = pick_app_path(self.mtm) {
                        let fragment = render_template(
                            preset.template,
                            &PresetBuildParams {
                                app: Some(path),
                                ..Default::default()
                            },
                        );
                        self.insert_script_fragment(&fragment, false);
                    }
                }
                Some(OpenAppChoice::PickWindow) => {
                    self.set_script_open_app_picking(true);
                    emit(NativeUiEvent::StartAppPick);
                }
                None => {}
            },
            PresetParamKind::Url => {
                if let Some(url) = prompt_url(self.mtm) {
                    let fragment = render_template(
                        preset.template,
                        &PresetBuildParams {
                            url: Some(url),
                            ..Default::default()
                        },
                    );
                    self.insert_script_fragment(&fragment, false);
                }
            }
            PresetParamKind::OpenFilePanel => {
                if let Some(path) = pick_open_file_path(self.mtm, "") {
                    let fragment = render_template(
                        preset.template,
                        &PresetBuildParams {
                            path: Some(path),
                            ..Default::default()
                        },
                    );
                    self.insert_script_fragment(&fragment, false);
                }
            }
            PresetParamKind::Text { multiline } => {
                if let Some(text) = prompt_text(
                    self.mtm,
                    preset.label,
                    "将写入脚本片段",
                    multiline,
                    "",
                ) {
                    let fragment = render_template(
                        preset.template,
                        &PresetBuildParams {
                            text: Some(text),
                            ..Default::default()
                        },
                    );
                    self.insert_script_fragment(&fragment, false);
                }
            }
            PresetParamKind::WaitMs => {
                if let Some(ms) = prompt_wait_ms(self.mtm) {
                    let fragment = render_template(
                        preset.template,
                        &PresetBuildParams {
                            ms: Some(ms.to_string()),
                            ..Default::default()
                        },
                    );
                    self.insert_script_fragment(&fragment, false);
                }
            }
            PresetParamKind::ShellCommand => {
                if let Some(cmd) = prompt_shell(self.mtm) {
                    let fragment = render_template(
                        preset.template,
                        &PresetBuildParams {
                            text: Some(cmd),
                            ..Default::default()
                        },
                    );
                    self.insert_script_fragment(&fragment, false);
                }
            }
            PresetParamKind::ClickCapture => self.start_click_capture(false),
            PresetParamKind::MacroRecord => {
                self.macro_recording = true;
                self.sync_macro_record_ui();
                unsafe {
                    self.script_status.setStringValue(&NSString::from_str(
                        "录制中… 在目标窗口操作，完成后点「完成录制」",
                    ));
                }
                emit(NativeUiEvent::StartScriptMacroRecord);
            }
            PresetParamKind::FlowWizard => {}
        }
    }

    fn start_click_capture(&mut self, for_flow: bool) {
        if !for_flow {
            self.pending_flow = None;
        }
        if self.script_app_picking || self.script_open_app_picking {
            emit(NativeUiEvent::CancelAppPick);
        }
        self.capturing_click = true;
        unsafe {
            self.script_status
                .setStringValue(&NSString::from_str("捕获中… 请到目标窗口点击一次"));
        }
        emit(NativeUiEvent::StartScriptClickCapture);
    }

    fn script_source(&self) -> String {
        unsafe { self.script_view().string().to_string() }
    }

    fn set_script_source(&self, source: &str) {
        unsafe {
            self.script_view()
                .setString(&NSString::from_str(source));
        }
        self.pane.apply_highlight();
        self.refresh_script_status(None);
    }

    fn import_script(&mut self) {
        let text = match pick_script_import(self.mtm) {
            Ok(Some((_path, text))) => text,
            Ok(None) => return,
            Err(err) => {
                show_alert(self.mtm, "导入失败", &err);
                return;
            }
        };
        let source = self.script_source();
        let meaningful = !source.trim().is_empty() && !is_comments_only(&source);
        let replace = if meaningful {
            match prompt_script_import_choice(self.mtm) {
                Some(FlowInsertChoice::ReplaceAll) => true,
                Some(FlowInsertChoice::Append) => false,
                None => return,
            }
        } else {
            true
        };
        if replace {
            self.set_script_source(&text);
        } else {
            self.insert_script_fragment(&text, false);
        }
        unsafe {
            self.script_status
                .setStringValue(&NSString::from_str(if replace {
                    "已导入脚本（替换）"
                } else {
                    "已导入脚本（插入到光标处）"
                }));
        }
        super::activation::present_utility_window(self.mtm, &self.window, &mut self.front_held);
    }

    fn export_script(&mut self) {
        let source = self.script_source();
        let default_name = format!("script-P{}#{}.meoflow", self.profile, self.button);
        let Some(path) = pick_script_export_path(self.mtm, &default_name) else {
            return;
        };
        match write_script_file(&path, &source) {
            Ok(()) => unsafe {
                self.script_status.setStringValue(&NSString::from_str(&format!(
                    "已导出：{}",
                    path.display()
                )));
            },
            Err(err) => show_alert(self.mtm, "导出失败", &err),
        }
        super::activation::present_utility_window(self.mtm, &self.window, &mut self.front_held);
    }

    fn refresh_cursor_position(&self) {
        let view = self.script_view();
        unsafe {
            let range = view.selectedRange();
            if self.is_script_view_first_responder() {
                self.last_caret.set(range);
            }
        }
        let (line, col) = cursor_line_col(&view);
        unsafe {
            self.cursor_pos.setStringValue(&NSString::from_str(&format!(
                "Ln {line}, Col {col}"
            )));
        }
    }

    fn is_script_view_first_responder(&self) -> bool {
        let view = self.script_view();
        unsafe {
            let Some(fr) = self.window.firstResponder() else {
                return false;
            };
            let fr_ptr = objc2::rc::Retained::as_ptr(&fr) as *const NSTextView;
            std::ptr::eq(fr_ptr, view as *const NSTextView)
        }
    }

    fn refresh_script_status(&self, session: Option<&ButtonEditSession>) {
        self.refresh_cursor_position();
        let source = self.script_source();
        let attached = session.and_then(|s| s.attached_macro.clone());
        match compile(&source, attached.as_ref()) {
            Ok(prog) => {
                self.pane.set_error_lines(vec![]);
                let secs = prog.estimated_ms as f64 / 1000.0;
                let status = format!("{} 步 · 约 {secs:.1}s · 无错误", prog.steps.len());
                let preview: String = prog
                    .steps
                    .iter()
                    .enumerate()
                    .take(4)
                    .map(|(i, s)| format!("{}. {}", i + 1, step_preview_label(s)))
                    .collect::<Vec<_>>()
                    .join("  ");
                let preview = if prog.steps.len() > 4 {
                    format!("{preview} …")
                } else if preview.is_empty() {
                    "—".into()
                } else {
                    preview
                };
                unsafe {
                    self.script_status
                        .setStringValue(&NSString::from_str(&status));
                    self.script_preview
                        .setStringValue(&NSString::from_str(&preview));
                }
            }
            Err(err) => {
                self.pane.set_error_lines(error_line_numbers(&err));
                let msg = err.primary_display().replace('\n', " ");
                unsafe {
                    self.script_status
                        .setStringValue(&NSString::from_str(&format!("错误 — {msg}")));
                    self.script_preview.setStringValue(&NSString::from_str(
                        "修复错误后可保存与试运行",
                    ));
                }
            }
        }
        self.pane.apply_highlight();
    }

    fn try_run_script(&self, session: &ButtonEditSession) {
        let source = self.script_source();
        let attached = session.attached_macro.clone();
        if let Err(err) = compile(&source, attached.as_ref()) {
            show_alert(self.mtm, "无法试运行", &err.primary_display());
            return;
        }
        emit(NativeUiEvent::TryRunScript {
            source,
            attached_macro: attached,
        });
        unsafe {
            self.script_status
                .setStringValue(&NSString::from_str("状态：试运行中…"));
        }
    }

    pub fn insert_script_click(&mut self, x: f32, y: f32) {
        self.capturing_click = false;
        let x_s = format!("{x:.4}");
        let y_s = format!("{y:.4}");

        if let Some(PendingFlowWizard::ClickHotkey { mut params }) = self.pending_flow.take() {
            params.x = Some(x_s);
            params.y = Some(y_s);
            if let Some(hotkey) = prompt_hotkey(self.mtm) {
                params.hotkey = Some(hotkey);
                let preset = preset_by_id("click_hotkey").unwrap();
                let fragment = render_template(preset.template, &params);
                self.insert_script_fragment(&fragment, self.flow_replace_all);
            }
            return;
        }

        let line = format!("点击 {x:.4} {y:.4}\n");
        self.insert_script_fragment(&line, false);
    }

    pub fn insert_script_activate(&mut self, bundle_id: &str) {
        self.set_script_app_picking(false);
        let escaped = bundle_id.replace('\\', "\\\\").replace('"', "\\\"");
        let line = format!("激活应用 \"{escaped}\"\n");
        self.insert_script_fragment(&line, false);
    }

    fn insert_script_fragment(&mut self, fragment: &str, replace_all: bool) {
        if replace_all {
            self.set_script_source(fragment);
            return;
        }
        // 在光标/选区处插入。用 replaceCharactersInRange:withString:，
        // 避免已弃用的 insertText: 在弹窗后 firstResponder 仍是 NSTextView 时崩溃。
        let mut insert = fragment.to_string();
        if !insert.ends_with('\n') {
            insert.push('\n');
        }
        let view = self.script_view();
        unsafe {
            // 点工具栏后 text view 会失焦，但 last_caret 仍是编辑时的位置
            let range = if self.is_script_view_first_responder() {
                view.selectedRange()
            } else {
                self.last_caret.get()
            };
            let full_len = view.string().length();
            let loc = range.location.min(full_len);
            let max_len = full_len.saturating_sub(loc);
            let len = range.length.min(max_len);
            let range = NSRange::new(loc, len);

            let ns = NSString::from_str(&insert);
            let inserted_len = ns.length();
            view.replaceCharactersInRange_withString(range, &ns);
            let new_loc = loc.saturating_add(inserted_len);
            view.setSelectedRange(NSRange::new(new_loc, 0));
            self.last_caret.set(NSRange::new(new_loc, 0));
            let _ = self.window.makeFirstResponder(Some(&view));
        }
        self.pane.apply_highlight();
        self.refresh_script_status(None);
    }

    pub fn append_macro_script(&mut self, session: &mut ButtonEditSession, macro_: &MouseMacro) {
        let fragment = macro_to_meoflow(macro_);
        self.insert_script_fragment(&format!("{fragment}\n"), false);
        session.attached_macro = Some(macro_.clone());
        session.draft.action = Action::Script(ActionScript {
            source: self.script_source(),
            program: None,
            attached_macro: session.attached_macro.clone(),
        });
        self.macro_recording = false;
        self.sync_macro_record_ui();
        self.refresh_script_status(Some(session));
    }

    pub fn cancel_macro_recording(&mut self) {
        self.macro_recording = false;
        self.sync_macro_record_ui();
        self.refresh_script_status(None);
    }

    pub fn is_macro_recording(&self) -> bool {
        self.macro_recording
    }

    pub fn is_script_app_picking(&self) -> bool {
        self.script_app_picking
    }

    pub fn cancel_script_capture(&mut self) {
        if self.capturing_click {
            self.capturing_click = false;
            self.pending_flow = None;
            self.refresh_script_status(None);
        }
    }

    fn load(&mut self, session: &mut ButtonEditSession) {
        let (source, legacy) = match &session.draft.action {
            Action::Script(s) => (s.source.clone(), false),
            Action::None => (DEFAULT_SCRIPT_SKELETON.to_string(), false),
            other => (action_to_meoflow(other), action_needs_legacy_upgrade(other)),
        };
        if legacy {
            session.draft_legacy = true;
        }
        session.attached_macro = action_attached_macro(&session.draft.action);

        unsafe {
            self.script_view()
                .setString(&NSString::from_str(&source));
        }
        if session.draft_legacy {
            unsafe {
                self.script_status.setStringValue(&NSString::from_str(
                    "已由旧动作类型转换为脚本，保存后将升级格式",
                ));
            }
        }
        self.refresh_script_status(Some(session));
    }

    /// 编译脚本写入 `session.draft.action`，保留 session 中其它 draft 字段。
    pub fn collect(&mut self, session: &mut ButtonEditSession) -> Result<(), String> {
        let source = self.script_source();
        let action = if source.trim().is_empty() || is_comments_only(&source) {
            Action::None
        } else {
            let attached = session.attached_macro.clone();
            let program = compile(&source, attached.as_ref()).map_err(|e| e.primary_display())?;
            Action::Script(ActionScript {
                source,
                program: Some(program),
                attached_macro: attached,
            })
        };
        session.draft.action = action;
        session.draft_legacy = false;
        session.attached_macro = action_attached_macro(&session.draft.action);
        Ok(())
    }
}

fn status_label(
    mtm: MainThreadMarker,
    parent: &NSView,
    text: &str,
    font: &objc2_app_kit::NSFont,
) -> Retained<NSTextField> {
    unsafe {
        let f = NSTextField::labelWithString(&NSString::from_str(text), mtm);
        f.setFont(Some(font));
        f.setTextColor(Some(&objc2_app_kit::NSColor::secondaryLabelColor()));
        parent.addSubview(&f);
        f
    }
}

fn cursor_line_col(view: &NSTextView) -> (usize, usize) {
    unsafe {
        let range = view.selectedRange();
        let full = view.string();
        let loc = range.location;
        let prefix = if loc == 0 {
            NSString::from_str("")
        } else {
            full.substringToIndex(loc)
        };
        let prefix_str = prefix.to_string();
        let line = prefix_str.matches('\n').count() + 1;
        let col = prefix_str
            .rfind('\n')
            .map(|i| prefix_str.len() - i)
            .unwrap_or(prefix_str.len() + 1);
        (line, col)
    }
}

#[cfg(test)]
mod tests {
    use crate::ui_native::script_wizard::path_for_config;

    #[test]
    fn home_path_uses_tilde() {
        let home = dirs::home_dir().expect("home dir");
        assert_eq!(path_for_config(&home.to_string_lossy()), "~");
        assert_eq!(
            path_for_config(&home.join("Documents/a.pdf").to_string_lossy()),
            "~/Documents/a.pdf"
        );
        assert_eq!(path_for_config("/tmp/notes.md"), "/tmp/notes.md");
    }
}
