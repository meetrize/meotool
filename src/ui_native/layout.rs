//! 原生偏好窗布局：左侧栏导航 + 右侧滚动表单（无粘性底栏）。
//!
//! Chrome 一律用手动 frame（`layout_pref_sidebar_chrome`），不在 contentView 上挂 Auto Layout，
//! 避免拖拽缩放时约束冲突导致崩溃。

use objc2::rc::Retained;
use objc2::{define_class, msg_send, MainThreadOnly};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSButton, NSColor, NSLayoutConstraint, NSLayoutConstraintOrientation,
    NSLayoutPriorityDefaultHigh, NSPopUpButton, NSScrollView, NSStackView, NSStackViewDistribution,
    NSSwitch, NSTextField, NSUserInterfaceLayoutOrientation, NSView,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSEdgeInsets, NSPoint, NSRect, NSSize, NSString};

#[derive(Default)]
struct FlippedPageHostIvars;

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "MeoToolFlippedPageHost"]
    #[ivars = FlippedPageHostIvars]
    struct FlippedPageHost;

    impl FlippedPageHost {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

impl FlippedPageHost {
    fn with_frame(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(FlippedPageHostIvars);
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

pub const SIDEBAR_W: f64 = 168.0;
pub const SIDEBAR_IO_H: f64 = 148.0;
pub const LABEL_W: f64 = 140.0;
pub const ROW_H: f64 = 28.0;
pub const CONTENT_PAD: f64 = 16.0;
pub const SIDEBAR_NAV_H: f64 = 32.0;

pub struct PrefSidebarChrome {
    pub sidebar: Retained<NSView>,
    /// 侧栏上方导航按钮纵向 stack
    pub nav_stack: Retained<NSStackView>,
    /// 侧栏下方配置 IO 区域
    pub io_stack: Retained<NSStackView>,
    pub scroll: Retained<NSScrollView>,
    pub page_host: Retained<NSView>,
    /// 右下角「已保存」提示（叠在内容区上）
    pub saved_label: Retained<NSTextField>,
}

/// 按 contentView.bounds 摆放侧栏 / 滚动区（可在 windowDidResize 中反复调用）。
pub fn layout_pref_sidebar_chrome(
    content: &NSView,
    sidebar: &NSView,
    nav_stack: &NSView,
    io_stack: &NSView,
    scroll: &NSView,
    saved_label: &NSView,
) {
    unsafe {
        let b = content.bounds().size;
        let w = b.width.max(480.0);
        let h = b.height.max(320.0);
        let side_w = SIDEBAR_W.min(w * 0.4).max(140.0);
        let body_w = (w - side_w).max(200.0);

        sidebar.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(side_w, h),
        ));
        scroll.setFrame(NSRect::new(
            NSPoint::new(side_w, 0.0),
            NSSize::new(body_w, h),
        ));

        let io_h = SIDEBAR_IO_H.min(h * 0.45).max(120.0);
        let nav_h = (h - io_h - 16.0).max(80.0);
        nav_stack.setFrame(NSRect::new(
            NSPoint::new(8.0, io_h + 8.0),
            NSSize::new((side_w - 16.0).max(1.0), nav_h),
        ));
        io_stack.setFrame(NSRect::new(
            NSPoint::new(8.0, 8.0),
            NSSize::new((side_w - 16.0).max(1.0), io_h),
        ));

        let toast_w = 72.0;
        let toast_h = 22.0;
        saved_label.setFrame(NSRect::new(
            NSPoint::new(w - toast_w - 16.0, 12.0),
            NSSize::new(toast_w, toast_h),
        ));
    }
}

/// 在 contentView 上铺左侧栏 + 右侧滚动区（纯 frame，无 Auto Layout）。
pub fn install_pref_sidebar_chrome(
    mtm: MainThreadMarker,
    content: &NSView,
    size: NSSize,
) -> PrefSidebarChrome {
    let sidebar = unsafe {
        let v = NSView::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(SIDEBAR_W, size.height)),
        );
        v.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);
        v.setAutoresizesSubviews(false);
        v.setTranslatesAutoresizingMaskIntoConstraints(true);
        v.setWantsLayer(true);
        if let Some(layer) = v.layer() {
            layer.setBackgroundColor(Some(&NSColor::controlBackgroundColor().CGColor()));
        }
        v
    };

    let nav_stack = v_stack_tight(mtm);
    let io_stack = v_stack_tight(mtm);
    unsafe {
        nav_stack.setSpacing(4.0);
        io_stack.setSpacing(6.0);
        sidebar.addSubview(&nav_stack);
        sidebar.addSubview(&io_stack);
    }

    let scroll = unsafe {
        let s = NSScrollView::initWithFrame(
            mtm.alloc(),
            NSRect::new(
                NSPoint::new(SIDEBAR_W, 0.0),
                NSSize::new((size.width - SIDEBAR_W).max(200.0), size.height),
            ),
        );
        s.setHasVerticalScroller(true);
        s.setHasHorizontalScroller(false);
        s.setAutohidesScrollers(true);
        s.setDrawsBackground(false);
        s.setBorderType(objc2_app_kit::NSBorderType::NoBorder);
        s.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);
        s.setTranslatesAutoresizingMaskIntoConstraints(true);
        s
    };
    let page_host = {
        let v = FlippedPageHost::with_frame(
            mtm,
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(400.0, 400.0)),
        );
        unsafe {
            v.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);
            scroll.setDocumentView(Some(&v));
        }
        Retained::into_super(v)
    };

    let saved_label = unsafe {
        let t = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        t.setFont(Some(&objc2_app_kit::NSFont::systemFontOfSize(11.0)));
        t.setTextColor(Some(&NSColor::secondaryLabelColor()));
        t.setHidden(true);
        t
    };

    unsafe {
        content.setAutoresizesSubviews(false);
        content.addSubview(&sidebar);
        content.addSubview(&scroll);
        content.addSubview(&saved_label);
        layout_pref_sidebar_chrome(
            content,
            &sidebar,
            &nav_stack,
            &io_stack,
            &scroll,
            &saved_label,
        );
    }

    PrefSidebarChrome {
        sidebar,
        nav_stack,
        io_stack,
        scroll,
        page_host,
        saved_label,
    }
}

fn v_stack_tight(mtm: MainThreadMarker) -> Retained<NSStackView> {
    let empty: Retained<NSArray<NSView>> = NSArray::from_retained_slice(&[]);
    let stack = NSStackView::stackViewWithViews(&empty, mtm);
    unsafe {
        stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
        stack.setAlignment(objc2_app_kit::NSLayoutAttribute::Leading);
        stack.setDistribution(NSStackViewDistribution::GravityAreas);
        stack.setSpacing(4.0);
        stack.setEdgeInsets(NSEdgeInsets {
            top: 0.0,
            left: 0.0,
            bottom: 0.0,
            right: 0.0,
        });
        stack.setTranslatesAutoresizingMaskIntoConstraints(true);
        stack.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);
    }
    stack
}

pub fn v_stack(mtm: MainThreadMarker) -> Retained<NSStackView> {
    let empty: Retained<NSArray<NSView>> = NSArray::from_retained_slice(&[]);
    let stack = NSStackView::stackViewWithViews(&empty, mtm);
    unsafe {
        stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
        stack.setAlignment(objc2_app_kit::NSLayoutAttribute::Leading);
        stack.setDistribution(NSStackViewDistribution::GravityAreas);
        stack.setSpacing(10.0);
        stack.setHuggingPriority_forOrientation(
            NSLayoutPriorityDefaultHigh,
            NSLayoutConstraintOrientation::Vertical,
        );
        stack.setEdgeInsets(NSEdgeInsets {
            top: CONTENT_PAD,
            left: CONTENT_PAD,
            bottom: CONTENT_PAD,
            right: CONTENT_PAD,
        });
        stack.setTranslatesAutoresizingMaskIntoConstraints(true);
        stack.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);
    }
    stack
}

pub fn h_stack(mtm: MainThreadMarker) -> Retained<NSStackView> {
    let empty: Retained<NSArray<NSView>> = NSArray::from_retained_slice(&[]);
    let stack = NSStackView::stackViewWithViews(&empty, mtm);
    unsafe {
        stack.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
        stack.setAlignment(objc2_app_kit::NSLayoutAttribute::CenterY);
        stack.setDistribution(NSStackViewDistribution::Fill);
        stack.setSpacing(8.0);
        stack.setTranslatesAutoresizingMaskIntoConstraints(true);
    }
    stack
}

pub fn add_arranged(stack: &NSStackView, view: &NSView) {
    unsafe { stack.addArrangedSubview(view) };
}

pub fn section_title(mtm: MainThreadMarker, title: &str) -> Retained<NSTextField> {
    let tf = unsafe { NSTextField::labelWithString(&NSString::from_str(title), mtm) };
    unsafe {
        tf.setFont(Some(&objc2_app_kit::NSFont::boldSystemFontOfSize(13.0)));
        tf.setContentHuggingPriority_forOrientation(
            NSLayoutPriorityDefaultHigh,
            NSLayoutConstraintOrientation::Vertical,
        );
    }
    tf
}

pub fn help_label(mtm: MainThreadMarker, text: &str) -> Retained<NSTextField> {
    let tf = unsafe { NSTextField::labelWithString(&NSString::from_str(text), mtm) };
    unsafe {
        tf.setFont(Some(&objc2_app_kit::NSFont::systemFontOfSize(11.0)));
        tf.setTextColor(Some(&objc2_app_kit::NSColor::secondaryLabelColor()));
        tf.setMaximumNumberOfLines(0);
        tf.setPreferredMaxLayoutWidth(520.0);
    }
    tf
}

pub fn labeled_field(
    mtm: MainThreadMarker,
    label: &str,
    width: f64,
) -> (Retained<NSStackView>, Retained<NSTextField>) {
    let row = h_stack(mtm);
    let lab = unsafe { NSTextField::labelWithString(&NSString::from_str(label), mtm) };
    unsafe {
        lab.setFrameSize(NSSize::new(LABEL_W, ROW_H));
        NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(&[lab
            .widthAnchor()
            .constraintEqualToConstant(LABEL_W)]));
    }
    let field = unsafe {
        let t = NSTextField::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, 24.0)),
        );
        t.setEditable(true);
        t.setBezeled(true);
        t.setFrameSize(NSSize::new(width, 24.0));
        t
    };
    add_arranged(&row, &lab);
    add_arranged(&row, &field);
    (row, field)
}

pub fn labeled_switch(
    mtm: MainThreadMarker,
    label: &str,
) -> (Retained<NSStackView>, Retained<NSSwitch>) {
    let row = h_stack(mtm);
    let lab = unsafe { NSTextField::labelWithString(&NSString::from_str(label), mtm) };
    unsafe {
        NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(&[lab
            .widthAnchor()
            .constraintEqualToConstant(LABEL_W)]));
    }
    let sw = unsafe {
        NSSwitch::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(42.0, 24.0)),
        )
    };
    add_arranged(&row, &lab);
    add_arranged(&row, &sw);
    (row, sw)
}

pub fn labeled_popup(
    mtm: MainThreadMarker,
    label: &str,
    items: &[&str],
    width: f64,
) -> (Retained<NSStackView>, Retained<NSPopUpButton>) {
    let row = h_stack(mtm);
    let lab = unsafe { NSTextField::labelWithString(&NSString::from_str(label), mtm) };
    unsafe {
        NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(&[lab
            .widthAnchor()
            .constraintEqualToConstant(LABEL_W)]));
    }
    let popup = unsafe {
        let p = NSPopUpButton::initWithFrame_pullsDown(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, 26.0)),
            false,
        );
        let titles: Vec<_> = items.iter().map(|s| NSString::from_str(s)).collect();
        p.addItemsWithTitles(&NSArray::from_retained_slice(&titles));
        p
    };
    add_arranged(&row, &lab);
    add_arranged(&row, &popup);
    (row, popup)
}

pub fn make_button(mtm: MainThreadMarker, title: &str, width: f64) -> Retained<NSButton> {
    unsafe {
        let b = NSButton::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, 28.0)),
        );
        b.setTitle(&NSString::from_str(title));
        b.setBezelStyle(objc2_app_kit::NSBezelStyle::Push);
        b
    }
}

/// 侧栏导航按钮（全宽、推挽选中）。
pub fn sidebar_nav_button(mtm: MainThreadMarker, title: &str, tag: isize) -> Retained<NSButton> {
    unsafe {
        let w = SIDEBAR_W - 16.0;
        let b = NSButton::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, SIDEBAR_NAV_H)),
        );
        b.setTitle(&NSString::from_str(title));
        b.setBezelStyle(objc2_app_kit::NSBezelStyle::FlexiblePush);
        b.setButtonType(objc2_app_kit::NSButtonType::PushOnPushOff);
        b.setTag(tag);
        NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(&[
            b.widthAnchor().constraintEqualToConstant(w),
            b.heightAnchor().constraintEqualToConstant(SIDEBAR_NAV_H),
        ]));
        b
    }
}

/// 侧栏底部 IO 小按钮（全宽）。
pub fn sidebar_io_button(mtm: MainThreadMarker, title: &str) -> Retained<NSButton> {
    unsafe {
        let w = SIDEBAR_W - 16.0;
        let b = NSButton::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, 26.0)),
        );
        b.setTitle(&NSString::from_str(title));
        b.setBezelStyle(objc2_app_kit::NSBezelStyle::FlexiblePush);
        NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(&[
            b.widthAnchor().constraintEqualToConstant(w),
            b.heightAnchor().constraintEqualToConstant(26.0),
        ]));
        b
    }
}

/// 滚动到文档顶部（page_host 为 flipped 坐标系）。
pub fn scroll_document_to_top(scroll: &NSScrollView) {
    unsafe {
        let clip = scroll.contentView();
        clip.scrollToPoint(NSPoint::new(0.0, 0.0));
        scroll.reflectScrolledClipView(&clip);
    }
}

/// 把 page stack 铺满 page_host，并按内容高度撑开以便滚动。
pub fn install_page(page_host: &NSView, page: &NSStackView) {
    unsafe {
        let subs = page_host.subviews();
        for view in subs.iter() {
            view.removeFromSuperview();
        }
        let width = document_width(page_host);
        page.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(width, 400.0),
        ));
        page.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);
        page_host.addSubview(page);
        sync_page_height(page_host, page);
    }
}

fn document_width(page_host: &NSView) -> f64 {
    unsafe {
        if let Some(sv) = page_host.superview() {
            let w = sv.bounds().size.width;
            if w > 1.0 {
                return w.max(200.0);
            }
        }
        page_host.bounds().size.width.max(200.0)
    }
}

pub fn sync_page_height(page_host: &NSView, page: &NSStackView) {
    unsafe {
        let width = document_width(page_host);
        if width < 2.0 {
            return;
        }
        page.setFrameSize(NSSize::new(width, page.frame().size.height.max(40.0)));
        page.layoutSubtreeIfNeeded();
        let fit = page.fittingSize();
        let h = fit.height.max(40.0);
        if !h.is_finite() || !width.is_finite() {
            return;
        }
        page.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(width, h),
        ));
        page_host.setFrameSize(NSSize::new(width, h));
    }
}

pub fn set_switch(sw: &NSSwitch, on: bool) {
    unsafe {
        sw.setState(if on {
            objc2_app_kit::NSControlStateValueOn
        } else {
            objc2_app_kit::NSControlStateValueOff
        });
    }
}

pub fn switch_on(sw: &NSSwitch) -> bool {
    unsafe { sw.state() == objc2_app_kit::NSControlStateValueOn }
}

pub fn set_field(field: &NSTextField, value: &str) {
    unsafe { field.setStringValue(&NSString::from_str(value)) };
}

pub fn field_string(field: &NSTextField) -> String {
    unsafe { field.stringValue().to_string() }
}

// ── 按钮编辑窗：滚动主内容 + 固定底栏 ─────────────────────────────

pub const BTN_EDITOR_FOOTER_H: f64 = 52.0;

pub struct ButtonEditorChrome {
    pub scroll: Retained<NSScrollView>,
    pub document: Retained<NSView>,
    pub footer: Retained<NSView>,
}

/// 创建 flipped 文档容器（y 向下增长，便于自上而下排布）。
pub fn flipped_document(mtm: MainThreadMarker, width: f64, height: f64) -> Retained<NSView> {
    let v = FlippedPageHost::with_frame(
        mtm,
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height)),
    );
    unsafe {
        v.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);
    }
    Retained::into_super(v)
}

/// 在 contentView 上安装滚动区 + 底栏（纯 frame，resize 时同步调用 [`layout_button_editor_chrome`]）。
pub fn install_button_editor_chrome(
    mtm: MainThreadMarker,
    content: &NSView,
    size: NSSize,
) -> ButtonEditorChrome {
    let footer = unsafe {
        let v = NSView::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(size.width, BTN_EDITOR_FOOTER_H)),
        );
        v.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);
        v.setWantsLayer(true);
        if let Some(layer) = v.layer() {
            layer.setBackgroundColor(Some(&NSColor::windowBackgroundColor().CGColor()));
        }
        v
    };

    let scroll = unsafe {
        let s = NSScrollView::initWithFrame(
            mtm.alloc(),
            NSRect::new(
                NSPoint::new(0.0, BTN_EDITOR_FOOTER_H),
                NSSize::new(size.width, (size.height - BTN_EDITOR_FOOTER_H).max(1.0)),
            ),
        );
        s.setHasVerticalScroller(true);
        s.setHasHorizontalScroller(false);
        s.setAutohidesScrollers(true);
        s.setDrawsBackground(true);
        s.setBorderType(objc2_app_kit::NSBorderType::NoBorder);
        s.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);
        s
    };

    let document = flipped_document(mtm, size.width, size.height);
    unsafe {
        scroll.setDocumentView(Some(&document));
        content.setAutoresizesSubviews(false);
        content.addSubview(&scroll);
        content.addSubview(&footer);
        layout_button_editor_chrome(
            content,
            &scroll,
            &document,
            &footer,
            None,
            None,
            None,
            0.0,
            0.0,
            400.0,
        );
    }

    ButtonEditorChrome {
        scroll,
        document,
        footer,
    }
}

/// 同步按钮编辑窗 chrome；`script_scroll` / 高度参数用于弹性脚本区。
pub fn layout_button_editor_chrome(
    content: &NSView,
    scroll: &NSScrollView,
    document: &NSView,
    footer: &NSView,
    save: Option<&NSButton>,
    cancel: Option<&NSButton>,
    script_scroll: Option<&NSScrollView>,
    script_y: f64,
    script_min_h: f64,
    content_h: f64,
) {
    unsafe {
        let b = content.bounds();
        let w = b.size.width.max(1.0);
        let h = b.size.height.max(1.0);
        let body_h = (h - BTN_EDITOR_FOOTER_H).max(1.0);

        footer.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(w, BTN_EDITOR_FOOTER_H),
        ));
        if let Some(save) = save {
            save.setFrame(NSRect::new(
                NSPoint::new(w - 116.0, 12.0),
                NSSize::new(100.0, 28.0),
            ));
        }
        if let Some(cancel) = cancel {
            cancel.setFrame(NSRect::new(
                NSPoint::new(w - 224.0, 12.0),
                NSSize::new(100.0, 28.0),
            ));
        }

        scroll.setFrame(NSRect::new(
            NSPoint::new(0.0, BTN_EDITOR_FOOTER_H),
            NSSize::new(w, body_h),
        ));

        let field_w = (w - 108.0 - CONTENT_PAD).max(200.0);

        if let Some(ss) = script_scroll {
            // 脚本区：窗口变高时把多余高度给编辑器，避免底部空洞。
            let bonus = (body_h - content_h).max(0.0).min(220.0);
            let script_h = script_min_h + bonus;
            let doc_h = content_h + bonus;
            ss.setFrame(NSRect::new(
                NSPoint::new(108.0, script_y),
                NSSize::new(field_w, script_h),
            ));
            document.setFrameSize(NSSize::new(w, doc_h.max(body_h)));
        } else {
            // 外观窗：文档高度贴合内容，窗口默认高度也应等于 content_h + footer。
            document.setFrameSize(NSSize::new(w, content_h.max(1.0)));
        }

        let clip = scroll.contentView();
        scroll.reflectScrolledClipView(&clip);
    }
}

// ── 脚本编辑窗：滚动区 + 状态栏 + 底栏 ─────────────────────────────

pub const SCRIPT_EDITOR_STATUS_H: f64 = 28.0;
pub const SCRIPT_EDITOR_DEFAULT_W: f64 = 960.0;
pub const SCRIPT_EDITOR_DEFAULT_H: f64 = 680.0;
pub const SCRIPT_EDITOR_MIN_W: f64 = 720.0;
pub const SCRIPT_EDITOR_MIN_H: f64 = 480.0;

pub struct ScriptEditorChrome {
    pub scroll: Retained<NSScrollView>,
    pub document: Retained<NSView>,
    pub status_bar: Retained<NSView>,
    pub footer: Retained<NSView>,
}

/// 脚本编辑窗 chrome：主内容滚动 + 固定状态栏 + 固定底栏。
pub fn install_script_editor_chrome(
    mtm: MainThreadMarker,
    content: &NSView,
    size: NSSize,
) -> ScriptEditorChrome {
    let footer = unsafe {
        let v = NSView::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(size.width, BTN_EDITOR_FOOTER_H)),
        );
        v.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);
        v.setWantsLayer(true);
        if let Some(layer) = v.layer() {
            layer.setBackgroundColor(Some(&NSColor::windowBackgroundColor().CGColor()));
        }
        v
    };

    let status_bar = unsafe {
        let v = NSView::initWithFrame(
            mtm.alloc(),
            NSRect::new(
                NSPoint::new(0.0, BTN_EDITOR_FOOTER_H),
                NSSize::new(size.width, SCRIPT_EDITOR_STATUS_H),
            ),
        );
        v.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);
        v.setWantsLayer(true);
        if let Some(layer) = v.layer() {
            layer.setBackgroundColor(Some(&NSColor::controlBackgroundColor().CGColor()));
        }
        v
    };

    let scroll = unsafe {
        let bottom = BTN_EDITOR_FOOTER_H + SCRIPT_EDITOR_STATUS_H;
        let s = NSScrollView::initWithFrame(
            mtm.alloc(),
            NSRect::new(
                NSPoint::new(0.0, bottom),
                NSSize::new(size.width, (size.height - bottom - 40.0).max(1.0)),
            ),
        );
        s.setHasVerticalScroller(true);
        s.setHasHorizontalScroller(false);
        s.setAutohidesScrollers(true);
        s.setDrawsBackground(true);
        s.setBorderType(objc2_app_kit::NSBorderType::NoBorder);
        s.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);
        s
    };

    let document = flipped_document(mtm, size.width, size.height);
    unsafe {
        scroll.setDocumentView(Some(&document));
        content.setAutoresizesSubviews(false);
        content.addSubview(&scroll);
        content.addSubview(&status_bar);
        content.addSubview(&footer);
        layout_script_editor_chrome(
            content,
            None,
            &scroll,
            &document,
            &status_bar,
            &footer,
            None,
            None,
            None,
            None,
            None,
            0.0,
            0.0,
            200.0,
            None,
            None,
            0.0,
        );
    }

    ScriptEditorChrome {
        scroll,
        document,
        status_bar,
        footer,
    }
}

/// 同步脚本编辑窗布局；`script_scroll` 在文档内弹性伸缩。
#[allow(clippy::too_many_arguments)]
pub fn layout_script_editor_chrome(
    content: &NSView,
    toolbar: Option<&NSView>,
    scroll: &NSScrollView,
    document: &NSView,
    status_bar: &NSView,
    footer: &NSView,
    save: Option<&NSButton>,
    cancel: Option<&NSButton>,
    cursor: Option<&NSTextField>,
    status: Option<&NSTextField>,
    preview: Option<&NSTextField>,
    script_y: f64,
    script_min_h: f64,
    content_h: f64,
    script_scroll: Option<&NSScrollView>,
    editor_row: Option<&NSView>,
    toolbar_h: f64,
) {
    unsafe {
        let b = content.bounds();
        let w = b.size.width.max(SCRIPT_EDITOR_MIN_W);
        let h = b.size.height.max(SCRIPT_EDITOR_MIN_H);
        let bottom_chrome = BTN_EDITOR_FOOTER_H + SCRIPT_EDITOR_STATUS_H;
        let body_h = (h - bottom_chrome - toolbar_h).max(1.0);

        footer.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(w, BTN_EDITOR_FOOTER_H),
        ));
        if let Some(save) = save {
            save.setFrame(NSRect::new(
                NSPoint::new(w - 116.0, 12.0),
                NSSize::new(100.0, 28.0),
            ));
        }
        if let Some(cancel) = cancel {
            cancel.setFrame(NSRect::new(
                NSPoint::new(w - 224.0, 12.0),
                NSSize::new(100.0, 28.0),
            ));
        }

        status_bar.setFrame(NSRect::new(
            NSPoint::new(0.0, BTN_EDITOR_FOOTER_H),
            NSSize::new(w, SCRIPT_EDITOR_STATUS_H),
        ));
        if let Some(cursor) = cursor {
            cursor.setFrame(NSRect::new(
                NSPoint::new(CONTENT_PAD, 4.0),
                NSSize::new(108.0, 20.0),
            ));
        }
        if let Some(status) = status {
            status.setFrame(NSRect::new(
                NSPoint::new(124.0, 4.0),
                NSSize::new((w * 0.42).max(120.0), 20.0),
            ));
        }
        if let Some(preview) = preview {
            let preview_x = 124.0 + (w * 0.42).max(120.0) + 8.0;
            preview.setFrame(NSRect::new(
                NSPoint::new(preview_x, 4.0),
                NSSize::new((w - preview_x - CONTENT_PAD).max(80.0), 20.0),
            ));
        }

        if let Some(tb) = toolbar {
            tb.setFrame(NSRect::new(
                NSPoint::new(0.0, h - toolbar_h),
                NSSize::new(w, toolbar_h),
            ));
        }

        scroll.setFrame(NSRect::new(
            NSPoint::new(0.0, bottom_chrome),
            NSSize::new(w, body_h),
        ));

        let bonus = (body_h - content_h).max(0.0);
        let script_h = script_min_h + bonus;
        let doc_h = content_h + bonus;
        let field_w = (w - CONTENT_PAD * 2.0).max(200.0);

        if let Some(row) = editor_row {
            row.setFrame(NSRect::new(
                NSPoint::new(CONTENT_PAD, script_y),
                NSSize::new(field_w, script_h),
            ));
        } else if let Some(ss) = script_scroll {
            ss.setFrame(NSRect::new(
                NSPoint::new(CONTENT_PAD, script_y),
                NSSize::new(field_w, script_h),
            ));
        }

        document.setFrameSize(NSSize::new(w, doc_h.max(body_h)));

        let clip = scroll.contentView();
        scroll.reflectScrolledClipView(&clip);
    }
}
