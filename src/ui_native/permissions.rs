//! 权限引导窗口。

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, MainThreadOnly};
use objc2_app_kit::{NSBackingStoreType, NSButton, NSTextField, NSWindow, NSWindowStyleMask};
use objc2_foundation::{
    MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};

use crate::platform::{
    check_permissions, executable_path, open_accessibility_settings, open_input_monitoring_settings,
};

use super::events::{emit, NativeUiEvent};

#[derive(Default)]
struct PermTargetIvars;

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "MeoToolPermTarget"]
    #[ivars = PermTargetIvars]
    struct PermTarget;

    unsafe impl NSObjectProtocol for PermTarget {}

    impl PermTarget {
        #[unsafe(method(refreshClicked:))]
        fn refresh_clicked(&self, _s: &AnyObject) {
            emit_perm(PermAction::Refresh);
        }
        #[unsafe(method(openAxClicked:))]
        fn open_ax(&self, _s: &AnyObject) {
            open_accessibility_settings();
        }
        #[unsafe(method(openInputClicked:))]
        fn open_input(&self, _s: &AnyObject) {
            open_input_monitoring_settings();
        }
        #[unsafe(method(relaunchClicked:))]
        fn relaunch_clicked(&self, _s: &AnyObject) {
            emit(NativeUiEvent::RelaunchForPermissions);
        }
        #[unsafe(method(closeClicked:))]
        fn close_clicked(&self, _s: &AnyObject) {
            emit_perm(PermAction::Close);
        }
    }
);

impl PermTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PermTargetIvars);
        unsafe { msg_send![super(this), init] }
    }
}

#[derive(Debug, Clone)]
enum PermAction {
    Refresh,
    Close,
}

static PERM_TX: OnceLock<Mutex<Option<Sender<PermAction>>>> = OnceLock::new();

fn emit_perm(a: PermAction) {
    if let Ok(g) = PERM_TX.get_or_init(|| Mutex::new(None)).lock() {
        if let Some(tx) = g.as_ref() {
            let _ = tx.send(a);
        }
    }
}

pub struct PermissionsWindow {
    window: Retained<NSWindow>,
    status_ax: Retained<NSTextField>,
    status_input: Retained<NSTextField>,
    path_label: Retained<NSTextField>,
    action_rx: Receiver<PermAction>,
    mtm: MainThreadMarker,
    front_held: bool,
}

impl PermissionsWindow {
    pub fn new(mtm: MainThreadMarker) -> Self {
        let (tx, rx) = mpsc::channel();
        *PERM_TX.get_or_init(|| Mutex::new(None)).lock().unwrap() = Some(tx);
        let target = PermTarget::new(mtm);
        let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable;
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc(),
                NSRect::new(NSPoint::new(280.0, 240.0), NSSize::new(520.0, 320.0)),
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe {
            window.setReleasedWhenClosed(false);
            window.setTitle(&NSString::from_str("MeoTool 需要系统权限"));
        }
        let content = window.contentView().unwrap();
        let hint = label(
            mtm,
            &content,
            "手势 / 热键 / 点击穿透需要「辅助功能」与「输入监控」。",
            250.0,
        );
        let _ = hint;
        let status_ax = label(mtm, &content, "辅助功能：…", 210.0);
        let status_input = label(mtm, &content, "输入监控：…", 178.0);
        let path_label = label(mtm, &content, "可执行文件：…", 146.0);

        add_btn(
            mtm,
            &content,
            &target,
            "打开辅助功能设置",
            objc2::sel!(openAxClicked:),
            16.0,
            90.0,
        );
        add_btn(
            mtm,
            &content,
            &target,
            "打开输入监控设置",
            objc2::sel!(openInputClicked:),
            200.0,
            90.0,
        );
        add_btn(
            mtm,
            &content,
            &target,
            "重新检测并重启",
            objc2::sel!(relaunchClicked:),
            16.0,
            50.0,
        );
        add_btn(
            mtm,
            &content,
            &target,
            "刷新状态",
            objc2::sel!(refreshClicked:),
            200.0,
            50.0,
        );
        add_btn(
            mtm,
            &content,
            &target,
            "关闭",
            objc2::sel!(closeClicked:),
            384.0,
            16.0,
        );
        std::mem::forget(target);

        let mut w = Self {
            window,
            status_ax,
            status_input,
            path_label,
            action_rx: rx,
            mtm,
            front_held: false,
        };
        w.refresh();
        w
    }

    pub fn show(&mut self) {
        self.refresh();
        super::activation::present_utility_window(self.mtm, &self.window, &mut self.front_held);
        let status = check_permissions();
        log::info!(
            "permissions dialog shown (ax={}, input={})",
            status.accessibility,
            status.input_monitoring
        );
    }

    pub fn hide(&mut self) {
        super::activation::dismiss_utility_window(self.mtm, &self.window, &mut self.front_held);
    }

    pub fn is_visible(&self) -> bool {
        unsafe { self.window.isVisible() }
    }

    pub fn poll(&mut self) {
        while let Ok(a) = self.action_rx.try_recv() {
            match a {
                PermAction::Refresh => {
                    self.refresh();
                    if check_permissions().all_granted() {
                        self.hide();
                    }
                }
                PermAction::Close => self.hide(),
            }
        }
    }

    fn refresh(&mut self) {
        let status = check_permissions();
        let ax = if status.accessibility {
            "辅助功能：已授权 ✓"
        } else {
            "辅助功能：未授权 — 请点击下方打开系统设置并勾选 MeoTool"
        };
        let input = if status.input_monitoring {
            "输入监控：已授权 ✓"
        } else {
            "输入监控：未授权 — 请点击下方打开系统设置并勾选 MeoTool"
        };
        let path = executable_path()
            .map(|p| format!("可执行文件：{}", p.display()))
            .unwrap_or_else(|| "可执行文件：未知".into());
        unsafe {
            self.status_ax.setStringValue(&NSString::from_str(ax));
            self.status_input.setStringValue(&NSString::from_str(input));
            self.path_label.setStringValue(&NSString::from_str(&path));
        }
    }
}

fn label(
    mtm: MainThreadMarker,
    parent: &objc2_app_kit::NSView,
    text: &str,
    y: f64,
) -> Retained<NSTextField> {
    unsafe {
        let t = NSTextField::labelWithString(&NSString::from_str(text), mtm);
        t.setFrame(NSRect::new(NSPoint::new(16.0, y), NSSize::new(488.0, 28.0)));
        parent.addSubview(&t);
        t
    }
}

fn add_btn(
    mtm: MainThreadMarker,
    parent: &objc2_app_kit::NSView,
    target: &PermTarget,
    title: &str,
    action: objc2::runtime::Sel,
    x: f64,
    y: f64,
) {
    unsafe {
        let b = NSButton::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(x, y), NSSize::new(170.0, 28.0)),
        );
        b.setTitle(&NSString::from_str(title));
        b.setTarget(Some(target));
        b.setAction(Some(action));
        parent.addSubview(&b);
    }
}
