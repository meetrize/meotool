//! 边框浮动 NSPanel + 按钮网格（阶段 1 MVP）。

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSBezelStyle, NSButton, NSCellImagePosition, NSColor, NSEvent,
    NSImageScaling, NSMenu, NSMenuItem, NSPanel, NSStatusWindowLevel, NSTextField, NSView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};

use crate::app::NativeMemoryCounters;
use crate::config::{parse_color, ButtonConfig, DisplayMode, PanelConfig, RgbColor};
use crate::platform::resolve_native_icon_tint;

use super::events::{decode_menu_tag, emit, encode_menu_tag, ButtonMenuAction, NativeUiEvent};
use super::icon_cache::NativeIconCache;

/// 与 egui Theme 对齐的圆角与图标边长
const BUTTON_CORNER_RADIUS: f64 = 8.0;
const ICON_DISPLAY_SIZE: f64 = 32.0;

static BUTTON_CLICK_TX: OnceLock<Mutex<Option<Sender<usize>>>> = OnceLock::new();

fn button_click_tx() -> &'static Mutex<Option<Sender<usize>>> {
    BUTTON_CLICK_TX.get_or_init(|| Mutex::new(None))
}

pub fn drain_button_clicks(rx: &Receiver<usize>) -> Vec<usize> {
    let mut out = Vec::new();
    while let Ok(i) = rx.try_recv() {
        out.push(i);
    }
    out
}

/// `sync` 回传：用户拖拽后窗口位置相对配置发生了变化。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DraggedPosition {
    pub x: f32,
    pub y: f32,
}

#[derive(Default)]
struct ButtonTargetIvars;

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "MeoToolNativeButtonTarget"]
    #[ivars = ButtonTargetIvars]
    struct ButtonTarget;

    unsafe impl NSObjectProtocol for ButtonTarget {}

    impl ButtonTarget {
        #[unsafe(method(buttonClicked:))]
        fn button_clicked(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            if tag < 0 {
                return;
            }
            if let Ok(guard) = button_click_tx().lock() {
                if let Some(tx) = guard.as_ref() {
                    let _ = tx.send(tag as usize);
                }
            }
        }

        #[unsafe(method(menuItemClicked:))]
        fn menu_item_clicked(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            if let Some((index, action)) = decode_menu_tag(tag) {
                emit(NativeUiEvent::ButtonMenu { index, action });
            }
        }

        #[unsafe(method(headerCollapseClicked:))]
        fn header_collapse_clicked(&self, _sender: &AnyObject) {
            emit(NativeUiEvent::EnterCollapsedMode);
        }

        #[unsafe(method(launcherClicked:))]
        fn launcher_clicked(&self, _sender: &AnyObject) {
            emit(NativeUiEvent::LauncherInteract);
            // 双击恢复完整模式
            let mtm = MainThreadMarker::new().expect("main");
            let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
            if let Some(ev) = unsafe { app.currentEvent() } {
                if unsafe { ev.clickCount() } >= 2 {
                    emit(NativeUiEvent::ExitCollapsedMode);
                }
            }
        }

        #[unsafe(method(launcherMenuClicked:))]
        fn launcher_menu_clicked(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            if let Some(action) = super::events::LauncherMenuAction::from_code(tag) {
                emit(NativeUiEvent::LauncherMenu(action));
            }
        }
    }
);

impl ButtonTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ButtonTargetIvars);
        unsafe { msg_send![super(this), init] }
    }
}

/// 标题栏：整条区域按下即可拖动窗口（含文字空隙）。
#[derive(Default)]
struct HeaderBarIvars;

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "MeoToolHeaderBar"]
    #[ivars = HeaderBarIvars]
    struct HeaderBar;

    unsafe impl NSObjectProtocol for HeaderBar {}

    impl HeaderBar {
        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool {
            true
        }
    }
);

impl HeaderBar {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(HeaderBarIvars);
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

/// 标题文字也允许拖窗口，避免点到字上反而不能拖。
#[derive(Default)]
struct HeaderLabelIvars;

define_class!(
    #[unsafe(super = NSTextField)]
    #[thread_kind = MainThreadOnly]
    #[name = "MeoToolHeaderLabel"]
    #[ivars = HeaderLabelIvars]
    struct HeaderLabel;

    unsafe impl NSObjectProtocol for HeaderLabel {}

    impl HeaderLabel {
        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool {
            true
        }
    }
);

impl HeaderLabel {
    fn new(mtm: MainThreadMarker, text: &str) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(HeaderLabelIvars);
        let field: Retained<Self> = unsafe { msg_send![super(this), init] };
        unsafe {
            field.setStringValue(&NSString::from_str(text));
            field.setBezeled(false);
            field.setBordered(false);
            field.setDrawsBackground(false);
            field.setEditable(false);
            field.setSelectable(false);
        }
        field
    }
}

/// 折叠桌宠：拖拽移动窗口；悬停展开；双击恢复完整模式；右键菜单。
#[derive(Default)]
struct PetLauncherIvars;

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "MeoToolPetLauncher"]
    #[ivars = PetLauncherIvars]
    struct PetLauncher;

    unsafe impl NSObjectProtocol for PetLauncher {}

    impl PetLauncher {
        /// 整块区域可命中（含 PNG 透明缝），否则透明窗点不到拖不动。
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> *const NSView {
            let hit: *const NSView = unsafe { msg_send![super(self), hitTest: point] };
            if hit.is_null() {
                std::ptr::null()
            } else {
                self as *const Self as *const NSView
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            // 双击退出折叠；拖动由 runtime 按桌宠矩形几何处理（透明像素也能拖）
            if unsafe { event.clickCount() } >= 2 {
                emit(NativeUiEvent::ExitCollapsedMode);
            }
        }
    }
);

impl PetLauncher {
    fn new(
        mtm: MainThreadMarker,
        frame: NSRect,
        icon_dirs: &[std::path::PathBuf],
        pet_name: &str,
        target: &ButtonTarget,
        pinned: bool,
        counters: &mut NativeMemoryCounters,
        icon_cache: &mut NativeIconCache,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PetLauncherIvars);
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        unsafe {
            let img_view = objc2_app_kit::NSImageView::initWithFrame(
                mtm.alloc(),
                NSRect::new(
                    NSPoint::new(0.0, 0.0),
                    NSSize::new(frame.size.width, frame.size.height),
                ),
            );
            img_view.setImageScaling(NSImageScaling::ScaleProportionallyDown);
            if let Some(img) = icon_cache.load(mtm, icon_dirs, pet_name, counters) {
                let avail_w = PanelConfig::COLLAPSED_LAUNCHER_WIDTH as f64;
                let avail_h = PanelConfig::COLLAPSED_LAUNCHER_HEIGHT as f64;
                let size = img.size();
                let scale = (avail_w / size.width).min(avail_h / size.height);
                img.setSize(NSSize::new(size.width * scale, size.height * scale));
                img_view.setImage(Some(&img));
            }
            view.addSubview(&img_view);
        }
        attach_launcher_context_menu(mtm, view.as_ref(), target, pinned);
        view
    }
}

#[derive(Clone, Copy, PartialEq)]
struct AppliedFrame {
    x: f64,
    y_top: f64,
    w: f64,
    h: f64,
}

/// 折叠模式内容壳：悬停展开时只切换可见性，不整树重建。
struct CollapsedShell {
    content: Retained<NSView>,
    grid_container: Retained<NSView>,
}

pub struct NativePanel {
    panel: Retained<NSPanel>,
    target: Retained<ButtonTarget>,
    click_rx: Receiver<usize>,
    last_fingerprint: String,
    last_applied: Option<AppliedFrame>,
    last_visible: Option<bool>,
    last_ignores_mouse: Option<bool>,
    /// 折叠桌宠在窗口内的矩形（左下原点，Cocoa）
    launcher_local: Option<NSRect>,
    /// 折叠态内容总矩形（含展开网格）
    content_local: Option<NSRect>,
    last_collapsed_expanded: Option<bool>,
    collapsed_shell: Option<CollapsedShell>,
    counters: NativeMemoryCounters,
    icon_cache: NativeIconCache,
}

impl NativePanel {
    pub fn new(mtm: MainThreadMarker) -> Self {
        let (tx, rx) = mpsc::channel();
        *button_click_tx().lock().unwrap() = Some(tx);

        let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
        let panel = unsafe {
            NSPanel::initWithContentRect_styleMask_backing_defer(
                mtm.alloc(),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(200.0, 200.0)),
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe {
            panel.setReleasedWhenClosed(false);
            panel.setOpaque(false);
            panel.setHasShadow(true);
            panel.setMovableByWindowBackground(true);
            panel.setFloatingPanel(true);
            panel.setBecomesKeyOnlyIfNeeded(true);
            panel.setLevel(NSStatusWindowLevel);
            panel.setBackgroundColor(Some(&NSColor::clearColor()));
            panel.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::FullScreenAuxiliary,
            );
        }

        let target = ButtonTarget::new(mtm);
        Self {
            panel,
            target,
            click_rx: rx,
            last_fingerprint: String::new(),
            last_applied: None,
            last_visible: None,
            last_ignores_mouse: None,
            launcher_local: None,
            content_local: None,
            last_collapsed_expanded: None,
            collapsed_shell: None,
            counters: NativeMemoryCounters::default(),
            icon_cache: NativeIconCache::default(),
        }
    }

    pub fn click_receiver(&self) -> &Receiver<usize> {
        &self.click_rx
    }

    pub fn memory_counters(&self) -> &NativeMemoryCounters {
        &self.counters
    }

    pub fn icon_cache_resident_count(&self) -> usize {
        self.icon_cache.resident_count()
    }

    pub fn clear_icon_cache(&mut self) {
        self.icon_cache.clear();
    }

    pub fn is_last_visible(&self) -> Option<bool> {
        self.last_visible
    }

    fn set_visible(&mut self, visible: bool) {
        if self.last_visible == Some(visible) {
            return;
        }
        self.last_visible = Some(visible);
        unsafe {
            if visible {
                self.panel.orderFrontRegardless();
            } else {
                self.panel.orderOut(None);
            }
        }
    }

    /// `x` / `y_top`：与现有 egui OuterPosition 一致（原点在屏幕左上）。
    fn set_frame_top_left(&self, mtm: MainThreadMarker, frame: AppliedFrame) {
        let screen_h = cocoa_screen_height(mtm, &self.panel);
        let origin_y = screen_h - frame.y_top - frame.h;
        unsafe {
            // display=false：拖拽/程序改位时避免每帧强制重绘闪烁
            self.panel.setFrame_display(
                NSRect::new(
                    NSPoint::new(frame.x, origin_y),
                    NSSize::new(frame.w, frame.h),
                ),
                false,
            );
        }
    }

    fn read_top_left(&self, mtm: MainThreadMarker) -> AppliedFrame {
        let frame = unsafe { self.panel.frame() };
        let screen_h = cocoa_screen_height(mtm, &self.panel);
        let y_top = screen_h - frame.origin.y - frame.size.height;
        AppliedFrame {
            x: frame.origin.x,
            y_top,
            w: frame.size.width,
            h: frame.size.height,
        }
    }

    /// 当前窗口左上角（屏幕坐标，原点左上）。
    pub fn top_left(&self, mtm: MainThreadMarker) -> (f64, f64) {
        let f = self.read_top_left(mtm);
        (f.x, f.y_top)
    }

    /// 拖动折叠桌宠时改位置（保持宽高，并更新 last_applied 以免 sync 拉回）。
    pub fn move_top_left(&mut self, mtm: MainThreadMarker, x: f64, y_top: f64) {
        let mut frame = self.read_top_left(mtm);
        frame.x = x;
        frame.y_top = y_top;
        self.set_frame_top_left(mtm, frame);
        self.last_applied = Some(frame);
    }

    pub fn set_ignores_mouse_events(&mut self, ignores: bool) {
        if self.last_ignores_mouse == Some(ignores) {
            return;
        }
        self.last_ignores_mouse = Some(ignores);
        unsafe {
            self.panel.setIgnoresMouseEvents(ignores);
        }
    }

    /// 屏幕坐标（左上原点）下，指针是否在折叠桌宠上。
    pub fn pointer_over_launcher(&self, mtm: MainThreadMarker, mx: f32, my: f32) -> bool {
        self.pointer_over_local_rect(mtm, self.launcher_local, mx, my)
    }

    /// 屏幕坐标下，指针是否在折叠态整块内容（宠+网格）上。
    pub fn pointer_over_collapsed_content(&self, mtm: MainThreadMarker, mx: f32, my: f32) -> bool {
        self.pointer_over_local_rect(mtm, self.content_local, mx, my)
    }

    /// 整窗命中区 `[min_x, min_y, max_x, max_y]`（左上原点，与 `global_mouse_pos` / 手势坐标一致）。
    pub fn panel_hit_rect(&self, mtm: MainThreadMarker) -> Option<[f32; 4]> {
        let frame = unsafe { self.panel.frame() };
        let screen_h = cocoa_screen_height(mtm, &self.panel);
        let left = frame.origin.x;
        let bottom = frame.origin.y;
        let right = left + frame.size.width;
        let top = bottom + frame.size.height;
        let y_top = screen_h - top;
        let y_bottom = screen_h - bottom;
        Some([left as f32, y_top as f32, right as f32, y_bottom as f32])
    }

    /// 指针是否在面板窗口上（完整 / 折叠模式通用）。
    pub fn pointer_over_panel(&self, mtm: MainThreadMarker, mx: f32, my: f32) -> bool {
        let Some([min_x, min_y, max_x, max_y]) = self.panel_hit_rect(mtm) else {
            return false;
        };
        mx >= min_x && mx < max_x && my >= min_y && my < max_y
    }

    fn pointer_over_local_rect(
        &self,
        mtm: MainThreadMarker,
        local: Option<NSRect>,
        mx: f32,
        my: f32,
    ) -> bool {
        let Some(local) = local else {
            return false;
        };
        let frame = unsafe { self.panel.frame() };
        let screen_h = cocoa_screen_height(mtm, &self.panel);
        // local 为窗口内容坐标（左下原点）
        let left = frame.origin.x + local.origin.x;
        let bottom = frame.origin.y + local.origin.y;
        let right = left + local.size.width;
        let top_from_bottom = bottom + local.size.height;
        let y_top = screen_h - top_from_bottom;
        let y_bottom_screen = screen_h - bottom;
        mx as f64 >= left
            && (mx as f64) < right
            && (my as f64) >= y_top
            && (my as f64) < y_bottom_screen
    }

    /// 同步面板。`collapsed_expanded`：折叠模式下网格是否展开。
    pub fn sync(
        &mut self,
        mtm: MainThreadMarker,
        panel_cfg: &PanelConfig,
        buttons: &[ButtonConfig],
        icon_dirs: &[std::path::PathBuf],
        visible: bool,
        collapsed_expanded: bool,
        pinned: bool,
    ) -> Option<DraggedPosition> {
        let fp = fingerprint(panel_cfg, buttons, pinned);
        let rebuilt = if fp != self.last_fingerprint {
            self.counters.record_fingerprint_change();
            self.rebuild(
                mtm,
                panel_cfg,
                buttons,
                icon_dirs,
                collapsed_expanded,
                pinned,
            );
            self.last_fingerprint = fp;
            self.last_collapsed_expanded = Some(collapsed_expanded);
            self.counters.record_panel_rebuild();
            true
        } else if panel_cfg.collapsed && self.last_collapsed_expanded != Some(collapsed_expanded) {
            self.apply_collapsed_expanded(mtm, panel_cfg, collapsed_expanded);
            self.last_collapsed_expanded = Some(collapsed_expanded);
            self.counters.record_collapsed_toggle();
            false
        } else {
            false
        };

        let (w, h) = if panel_cfg.collapsed {
            panel_cfg.viewport_size(collapsed_expanded)
        } else {
            panel_cfg.viewport_size(false)
        };
        let desired = AppliedFrame {
            x: panel_cfg.position.x as f64,
            y_top: panel_cfg.position.y as f64,
            w: w as f64,
            h: h as f64,
        };

        let mut dragged = None;

        if visible && self.last_visible == Some(true) && !rebuilt {
            let actual = self.read_top_left(mtm);
            if let Some(applied) = self.last_applied {
                let moved_by_user = (actual.x - applied.x).abs() > 0.5
                    || (actual.y_top - applied.y_top).abs() > 0.5;
                if moved_by_user {
                    self.last_applied = Some(AppliedFrame {
                        x: actual.x,
                        y_top: actual.y_top,
                        w: applied.w,
                        h: applied.h,
                    });
                    if (actual.x - desired.x).abs() > 0.5
                        || (actual.y_top - desired.y_top).abs() > 0.5
                    {
                        dragged = Some(DraggedPosition {
                            x: actual.x as f32,
                            y: actual.y_top as f32,
                        });
                    }
                }
            }
        }

        let need_apply = dragged.is_none()
            && (rebuilt
                || self.last_applied != Some(desired)
                || (visible && self.last_visible != Some(true)));

        if need_apply {
            self.set_frame_top_left(mtm, desired);
            self.last_applied = Some(desired);
        }

        if panel_cfg.collapsed && !collapsed_expanded {
            unsafe {
                self.panel.setBackgroundColor(Some(&NSColor::clearColor()));
                self.panel.setOpaque(false);
                self.panel.setHasShadow(false);
            }
        } else {
            unsafe {
                self.panel
                    .setBackgroundColor(Some(&ns_color(panel_cfg.ui_style.background)));
                self.panel.setOpaque(panel_cfg.ui_style.background.a > 200);
                self.panel.setHasShadow(true);
            }
        }
        self.set_visible(visible);
        dragged
    }

    /// 折叠悬停展开/收起：仅切换网格与背景，避免整面板重建与重复解码图标。
    fn apply_collapsed_expanded(
        &mut self,
        mtm: MainThreadMarker,
        panel_cfg: &PanelConfig,
        expanded: bool,
    ) {
        let Some(shell) = self.collapsed_shell.as_ref() else {
            return;
        };
        let bg = scale_alpha(
            panel_cfg.ui_style.background,
            panel_cfg.opacity.clamp(0.05, 1.0),
        );
        unsafe {
            shell.grid_container.setHidden(!expanded);
            if expanded {
                paint_view_background(&shell.content, bg, 0.0);
                let (vw, vh) = panel_cfg.viewport_size(true);
                self.content_local = Some(NSRect::new(
                    NSPoint::new(0.0, 0.0),
                    NSSize::new(vw as f64, vh as f64),
                ));
            } else {
                clear_view_background(&shell.content);
            }
        }
        let _ = mtm;
    }

    fn rebuild(
        &mut self,
        mtm: MainThreadMarker,
        panel_cfg: &PanelConfig,
        buttons: &[ButtonConfig],
        icon_dirs: &[std::path::PathBuf],
        collapsed_expanded: bool,
        pinned: bool,
    ) {
        self.launcher_local = None;
        self.content_local = None;
        self.collapsed_shell = None;
        if panel_cfg.collapsed {
            self.rebuild_collapsed(
                mtm,
                panel_cfg,
                buttons,
                icon_dirs,
                collapsed_expanded,
                pinned,
            );
        } else {
            self.rebuild_full(mtm, panel_cfg, buttons, icon_dirs);
        }
    }

    fn rebuild_full(
        &mut self,
        mtm: MainThreadMarker,
        panel_cfg: &PanelConfig,
        buttons: &[ButtonConfig],
        icon_dirs: &[std::path::PathBuf],
    ) {
        let (vw, vh) = panel_cfg.viewport_size(false);
        let content = unsafe {
            NSView::initWithFrame(
                mtm.alloc(),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(vw as f64, vh as f64)),
            )
        };
        paint_view_background(
            &content,
            scale_alpha(
                panel_cfg.ui_style.background,
                panel_cfg.opacity.clamp(0.05, 1.0),
            ),
            0.0,
        );

        let cols = panel_cfg.cols.max(1) as usize;
        let rows = panel_cfg.rows.max(1) as usize;
        let bw = panel_cfg.button_width as f64;
        let bh = panel_cfg.button_height as f64;
        let gap = panel_cfg.gap as f64;
        let pad = PanelConfig::PANEL_PADDING as f64;
        let header = PanelConfig::PANEL_HEADER_HEIGHT as f64;

        for (index, button) in buttons.iter().enumerate() {
            let col = index % cols;
            let row = index / cols;
            if row >= rows {
                break;
            }
            let x = pad + col as f64 * (bw + gap);
            let y_from_top = header + pad + row as f64 * (bh + gap);
            let y = vh as f64 - y_from_top - bh;

            let btn = unsafe {
                let b = NSButton::initWithFrame(
                    mtm.alloc(),
                    NSRect::new(NSPoint::new(x, y), NSSize::new(bw, bh)),
                );
                b.setTarget(Some(&self.target));
                b.setAction(Some(objc2::sel!(buttonClicked:)));
                b.setTag(index as isize);
                style_grid_button(
                    mtm,
                    &b,
                    panel_cfg,
                    button,
                    icon_dirs,
                    index,
                    &mut self.counters,
                    &mut self.icon_cache,
                );
                attach_button_context_menu(mtm, &b, &self.target, index, button);
                b
            };
            unsafe { content.addSubview(&btn) };
        }

        let header_bar = HeaderBar::new(
            mtm,
            NSRect::new(
                NSPoint::new(0.0, vh as f64 - header),
                NSSize::new(vw as f64, header),
            ),
        );
        paint_view_background(
            &header_bar,
            scale_alpha(
                panel_cfg.ui_style.background,
                panel_cfg.opacity.clamp(0.05, 1.0),
            ),
            0.0,
        );
        let collapse_w = 36.0_f64;
        let label = HeaderLabel::new(mtm, "MeoTool");
        unsafe {
            label.setFrame(NSRect::new(
                NSPoint::new(pad, 2.0),
                NSSize::new((vw as f64 - pad * 2.0 - collapse_w).max(40.0), header - 4.0),
            ));
            label.setTextColor(Some(&NSColor::whiteColor()));
            header_bar.addSubview(&label);
        }

        // 标题栏右侧折叠猫头
        let pet_name = panel_cfg
            .collapsed_icon
            .as_deref()
            .unwrap_or(PanelConfig::DEFAULT_COLLAPSED_ICON);
        let collapse_btn = unsafe {
            let b = NSButton::initWithFrame(
                mtm.alloc(),
                NSRect::new(
                    NSPoint::new(vw as f64 - collapse_w - 2.0, 1.0),
                    NSSize::new(collapse_w, header - 2.0),
                ),
            );
            b.setBordered(false);
            b.setTitle(&NSString::from_str(""));
            b.setTarget(Some(&self.target));
            b.setAction(Some(objc2::sel!(headerCollapseClicked:)));
            b.setToolTip(Some(&NSString::from_str("切换为折叠模式")));
            if let Some(img) = self.icon_cache.load(mtm, icon_dirs, pet_name, &mut self.counters) {
                img.setSize(NSSize::new(20.0, 14.0));
                b.setImage(Some(&img));
                b.setImagePosition(NSCellImagePosition::ImageOnly);
                b.setImageScaling(NSImageScaling::ScaleProportionallyDown);
            } else {
                b.setTitle(&NSString::from_str("🐱"));
            }
            header_bar.addSubview(&b);
            b
        };
        let _ = collapse_btn;

        unsafe {
            content.addSubview(&header_bar);
            self.panel.setContentView(Some(&content));
        }
        self.last_collapsed_expanded = None;
    }

    fn rebuild_collapsed(
        &mut self,
        mtm: MainThreadMarker,
        panel_cfg: &PanelConfig,
        buttons: &[ButtonConfig],
        icon_dirs: &[std::path::PathBuf],
        expanded: bool,
        pinned: bool,
    ) {
        let (vw, vh) = panel_cfg.viewport_size(true);
        let content = unsafe {
            NSView::initWithFrame(
                mtm.alloc(),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(vw as f64, vh as f64)),
            )
        };
        let bg = scale_alpha(
            panel_cfg.ui_style.background,
            panel_cfg.opacity.clamp(0.05, 1.0),
        );
        if expanded {
            paint_view_background(&content, bg, 0.0);
        }

        let (lw, lh) = PanelConfig::collapsed_launcher_alloc();
        let gap = panel_cfg.gap as f64;
        let (full_w, _) = panel_cfg.panel_size();
        let launcher_x = ((vw as f64 - lw as f64) * 0.5).max(0.0);
        let launcher_y = vh as f64 - lh as f64; // top of content in cocoa y
        let launcher_rect = NSRect::new(
            NSPoint::new(launcher_x, launcher_y),
            NSSize::new(lw as f64, lh as f64),
        );
        self.launcher_local = Some(launcher_rect);
        self.content_local = Some(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(vw as f64, vh as f64),
        ));

        let pet_name = panel_cfg
            .collapsed_icon
            .as_deref()
            .unwrap_or(PanelConfig::DEFAULT_COLLAPSED_ICON);
        let launcher = PetLauncher::new(
            mtm,
            launcher_rect,
            icon_dirs,
            pet_name,
            &self.target,
            pinned,
            &mut self.counters,
            &mut self.icon_cache,
        );
        let cols = panel_cfg.cols.max(1) as usize;
        let rows = panel_cfg.rows.max(1) as usize;
        let bw = panel_cfg.button_width as f64;
        let bh = panel_cfg.button_height as f64;
        let btn_gap = panel_cfg.gap as f64;
        let grid_x = ((vw as f64 - full_w as f64) * 0.5).max(0.0);
        let grid_top = lh as f64 + gap;
        let grid_height = rows as f64 * bh + (rows.saturating_sub(1)) as f64 * btn_gap;
        let grid_width = cols as f64 * bw + (cols.saturating_sub(1)) as f64 * btn_gap;
        let grid_container = unsafe {
            NSView::initWithFrame(
                mtm.alloc(),
                NSRect::new(
                    NSPoint::new(grid_x, vh as f64 - grid_top - grid_height),
                    NSSize::new(grid_width, grid_height),
                ),
            )
        };
        for (index, button) in buttons.iter().enumerate() {
            let col = index % cols;
            let row = index / cols;
            if row >= rows {
                break;
            }
            let x = col as f64 * (bw + btn_gap);
            let y = grid_height - (row as f64 + 1.0) * bh - row as f64 * btn_gap;
            let btn = unsafe {
                let b = NSButton::initWithFrame(
                    mtm.alloc(),
                    NSRect::new(NSPoint::new(x, y), NSSize::new(bw, bh)),
                );
                b.setTarget(Some(&self.target));
                b.setAction(Some(objc2::sel!(buttonClicked:)));
                b.setTag(index as isize);
                style_grid_button(
                    mtm,
                    &b,
                    panel_cfg,
                    button,
                    icon_dirs,
                    index,
                    &mut self.counters,
                    &mut self.icon_cache,
                );
                attach_button_context_menu(mtm, &b, &self.target, index, button);
                grid_container.addSubview(&b);
                b
            };
            let _ = btn;
        }
        unsafe {
            grid_container.setHidden(!expanded);
            content.addSubview(&grid_container);
            content.addSubview(&launcher);
            self.panel.setContentView(Some(&content));
        }
        self.collapsed_shell = Some(CollapsedShell {
            content,
            grid_container,
        });
        self.last_collapsed_expanded = Some(expanded);
    }
}

/// 对齐 egui 按钮右键菜单项。
fn attach_button_context_menu(
    mtm: MainThreadMarker,
    button: &NSButton,
    target: &ButtonTarget,
    index: usize,
    cfg: &ButtonConfig,
) {
    let configured = is_button_configured(cfg);
    let has_action = cfg.action.is_configured();
    let menu = NSMenu::new(mtm);

    add_menu_item(mtm, &menu, target, "编辑脚本…", index, ButtonMenuAction::Edit);
    add_menu_item(
        mtm,
        &menu,
        target,
        "编辑外观…",
        index,
        ButtonMenuAction::EditAppearance,
    );

    if configured {
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        if cfg.enabled {
            add_menu_item(mtm, &menu, target, "禁用", index, ButtonMenuAction::Disable);
        } else if has_action {
            add_menu_item(mtm, &menu, target, "启用", index, ButtonMenuAction::Enable);
        }
        if has_action {
            add_menu_item(
                mtm,
                &menu,
                target,
                "清空动作",
                index,
                ButtonMenuAction::ClearAction,
            );
        }
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        add_menu_item(mtm, &menu, target, "删除", index, ButtonMenuAction::Delete);
    }

    menu.addItem(&NSMenuItem::separatorItem(mtm));
    add_menu_item(
        mtm,
        &menu,
        target,
        "设置",
        index,
        ButtonMenuAction::Settings,
    );
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    add_menu_item(mtm, &menu, target, "退出", index, ButtonMenuAction::Quit);

    unsafe {
        let _: () = msg_send![button, setMenu: &*menu];
    }
}

fn add_menu_item(
    mtm: MainThreadMarker,
    menu: &NSMenu,
    target: &ButtonTarget,
    title: &str,
    index: usize,
    action: ButtonMenuAction,
) {
    unsafe {
        let item = NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str(title),
            Some(objc2::sel!(menuItemClicked:)),
            &NSString::from_str(""),
        );
        item.setTarget(Some(target));
        item.setTag(encode_menu_tag(index, action));
        menu.addItem(&item);
    }
}

fn is_button_configured(button: &ButtonConfig) -> bool {
    !button.label.is_empty() || button.icon.is_some() || button.action.is_configured()
}

fn paint_view_background(view: &NSView, color: RgbColor, corner_radius: f64) {
    unsafe {
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            let _: () = msg_send![&*layer, setCornerRadius: corner_radius];
            let _: () = msg_send![&*layer, setMasksToBounds: true];
            let ns = ns_color(color);
            let _: () = msg_send![&*layer, setBackgroundColor: &*ns.CGColor()];
        }
    }
}

fn clear_view_background(view: &NSView) {
    unsafe {
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            let clear = NSColor::clearColor();
            let _: () = msg_send![&*layer, setBackgroundColor: &*clear.CGColor()];
        }
    }
}

fn cocoa_screen_height(mtm: MainThreadMarker, _panel: &NSPanel) -> f64 {
    // Cocoa 全局坐标以主屏底部为原点；读写必须用同一参照，避免拖拽回读跳变
    objc2_app_kit::NSScreen::mainScreen(mtm)
        .map(|s| s.frame().size.height)
        .unwrap_or(900.0)
}

/// 去掉系统胶囊 bezel，画成 egui 同款圆角正方形，图标居中不撑破边框。
fn style_grid_button(
    mtm: MainThreadMarker,
    button: &NSButton,
    panel: &PanelConfig,
    cfg: &ButtonConfig,
    icon_dirs: &[std::path::PathBuf],
    index: usize,
    counters: &mut NativeMemoryCounters,
    icon_cache: &mut NativeIconCache,
) {
    let style = &panel.ui_style;
    let opacity = panel.opacity.clamp(0.05, 1.0);
    let enabled = cfg.enabled && cfg.action.is_configured();

    let bg = cfg
        .background_color
        .as_deref()
        .and_then(parse_color)
        .unwrap_or(style.button_bg);
    let border = cfg
        .border_color
        .as_deref()
        .and_then(parse_color)
        .unwrap_or(style.button_border_color);
    let border_on = style.button_border_enabled || cfg.border_color.is_some();
    let border_w = if cfg.border_color.is_some() && style.button_border_width <= 0.0 {
        1.0
    } else {
        style.button_border_width
    };

    unsafe {
        // 关闭系统圆角胶囊边框（否则 64×64 也会画成扁长圆角条）
        button.setBordered(false);
        button.setBezelStyle(NSBezelStyle::FlexiblePush);
        button.setEnabled(enabled);

        button.setWantsLayer(true);
        if let Some(layer) = button.layer() {
            // 部分 CALayer 方法受 crate feature 门控；用 msg_send 保证可用
            let _: () = msg_send![&*layer, setCornerRadius: BUTTON_CORNER_RADIUS];
            let _: () = msg_send![&*layer, setMasksToBounds: true];
            let bg_a = if enabled { opacity } else { opacity * 0.45 };
            let bg_color = ns_color(scale_alpha(bg, bg_a));
            let _: () = msg_send![&*layer, setBackgroundColor: &*bg_color.CGColor()];
            if border_on && border_w > 0.0 {
                let _: () = msg_send![&*layer, setBorderWidth: border_w as f64];
                let border_color = ns_color(scale_alpha(border, opacity));
                let _: () = msg_send![&*layer, setBorderColor: &*border_color.CGColor()];
            } else {
                let _: () = msg_send![&*layer, setBorderWidth: 0.0_f64];
            }
        }

        match panel.display_mode {
            DisplayMode::Text => {
                button.setImage(None);
                button.setImagePosition(NSCellImagePosition::NoImage);
                button.setTitle(&NSString::from_str(&text_label(cfg, index)));
                button.setFont(Some(&objc2_app_kit::NSFont::systemFontOfSize(
                    style.letter_size as f64,
                )));
                button.setContentTintColor(None);
            }
            DisplayMode::Icon | DisplayMode::IndexIcon => {
                button.setTitle(&NSString::from_str(""));
                button.setImagePosition(NSCellImagePosition::ImageOnly);
                button.setImageScaling(NSImageScaling::ScaleProportionallyDown);
                let icon_tint = scale_alpha(
                    resolve_native_icon_tint(cfg.icon_color.as_deref(), style.letter_color, enabled),
                    opacity,
                );
                if let Some(icon_name) = cfg.icon.as_deref() {
                    if let Some(img) =
                        icon_cache.load_template(mtm, icon_dirs, icon_name, counters)
                    {
                        let side = ICON_DISPLAY_SIZE
                            .min(panel.button_width as f64 - 16.0)
                            .min(panel.button_height as f64 - 16.0)
                            .max(12.0);
                        img.setSize(NSSize::new(side, side));
                        button.setImage(Some(&img));
                        button.setContentTintColor(Some(&ns_color(icon_tint)));
                    } else {
                        button.setImage(None);
                        button.setContentTintColor(None);
                    }
                } else {
                    button.setImage(None);
                    button.setContentTintColor(None);
                }
                if !cfg.label.trim().is_empty() {
                    button.setToolTip(Some(&NSString::from_str(&cfg.label)));
                }
                let _ = index; // IndexIcon 角标后续可加；先保证外框与图标正确
            }
        }
    }
}

fn text_label(button: &ButtonConfig, index: usize) -> String {
    if !button.label.trim().is_empty() {
        return button.label.clone();
    }
    if let Some(icon) = button.icon.as_deref() {
        return icon.trim_end_matches(".png").chars().take(8).collect();
    }
    format!("{}", index + 1)
}

fn scale_alpha(c: RgbColor, opacity: f32) -> RgbColor {
    RgbColor {
        r: c.r,
        g: c.g,
        b: c.b,
        a: ((c.a as f32) * opacity).clamp(0.0, 255.0) as u8,
    }
}

/// 对齐 egui 折叠桌宠右键菜单。
fn attach_launcher_context_menu(
    mtm: MainThreadMarker,
    view: &NSView,
    target: &ButtonTarget,
    pinned: bool,
) {
    use super::events::LauncherMenuAction;
    let menu = NSMenu::new(mtm);
    let pin_title = if pinned {
        "取消钉住"
    } else {
        "钉住面板"
    };
    for (title, action, sep_before) in [
        (pin_title, LauncherMenuAction::TogglePin, false),
        ("恢复完整模式", LauncherMenuAction::ExpandToFull, false),
        ("设置", LauncherMenuAction::Settings, true),
        ("退出", LauncherMenuAction::Quit, true),
    ] {
        if sep_before {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
        }
        unsafe {
            let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc(),
                &NSString::from_str(title),
                Some(objc2::sel!(launcherMenuClicked:)),
                &NSString::from_str(""),
            );
            item.setTarget(Some(target));
            item.setTag(action as isize);
            menu.addItem(&item);
        }
    }
    unsafe {
        let _: () = msg_send![view, setMenu: &*menu];
    }
}

fn fingerprint(panel: &PanelConfig, buttons: &[ButtonConfig], pinned: bool) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let _ = write!(
        &mut s,
        "{}x{}:{:.0}x{:.0}:{:.0}:{:?}:{:.2}:{}:{}:{}",
        panel.cols,
        panel.rows,
        panel.button_width,
        panel.button_height,
        panel.gap,
        panel.display_mode,
        panel.opacity,
        panel.collapsed,
        pinned,
        buttons.len()
    );
    for (i, b) in buttons.iter().enumerate() {
        let _ = write!(
            &mut s,
            "|{i}:{}:{}:{:?}:{}:{:?}:{:?}:{:?}",
            b.enabled,
            b.label,
            b.icon,
            b.action.is_configured(),
            b.background_color,
            b.border_color,
            b.icon_color
        );
    }
    s
}

fn ns_color(c: RgbColor) -> Retained<NSColor> {
    unsafe {
        NSColor::colorWithSRGBRed_green_blue_alpha(
            c.r as f64 / 255.0,
            c.g as f64 / 255.0,
            c.b as f64 / 255.0,
            c.a as f64 / 255.0,
        )
    }
}
