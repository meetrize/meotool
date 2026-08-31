//! 脚本编辑区：行号栏 + MeoFlow 着色文本视图。

use std::cell::RefCell;
use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSBackgroundColorAttributeName, NSColor, NSFont, NSFontAttributeName, NSForegroundColorAttributeName,
    NSScrollView, NSTextDidChangeNotification, NSTextView, NSView, NSViewBoundsDidChangeNotification,
};
use objc2_foundation::{MainThreadMarker, NSNotification, NSNotificationCenter, NSPoint, NSRange, NSRect, NSSize, NSString};

use crate::config::script_presets::DEFAULT_SCRIPT_SKELETON;

use super::meoflow_highlight::{
    byte_range_to_utf16, gutter_text, highlight_meoflow, line_count, utf16_len, TokenKind,
};

pub const GUTTER_WIDTH: f64 = 44.0;

pub struct ScriptEditorPane {
    pub row: Retained<NSView>,
    pub script_scroll: Retained<NSScrollView>,
    pub script_view: Retained<NSTextView>,
    gutter_scroll: Retained<NSScrollView>,
    gutter_view: Retained<NSTextView>,
    text_font: Retained<NSFont>,
    error_lines: RefCell<Vec<usize>>,
    bounds_observer: RefCell<Option<Retained<ProtocolObject<dyn objc2_foundation::NSObjectProtocol>>>>,
    text_observer: RefCell<Option<Retained<ProtocolObject<dyn objc2_foundation::NSObjectProtocol>>>>,
}

impl ScriptEditorPane {
    pub fn install(
        mtm: MainThreadMarker,
        parent: &NSView,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        on_text_changed: &block2::RcBlock<dyn Fn(NonNull<NSNotification>)>,
    ) -> Self {
        let row = unsafe {
            let v = NSView::initWithFrame(
                mtm.alloc(),
                NSRect::new(NSPoint::new(x, y), NSSize::new(width, height)),
            );
            v.setAutoresizingMask(objc2_app_kit::NSAutoresizingMaskOptions::ViewNotSizable);
            parent.addSubview(&v);
            v
        };

        let text_font = objc2_app_kit::NSFont::userFixedPitchFontOfSize(12.0)
            .unwrap_or_else(|| objc2_app_kit::NSFont::systemFontOfSize(12.0));
        let gutter_font = objc2_app_kit::NSFont::userFixedPitchFontOfSize(11.0)
            .unwrap_or_else(|| text_font.clone());

        let gutter_scroll = gutter_scroll_view(mtm, &row);
        let gutter_view = gutter_text_view(mtm, &gutter_scroll, &gutter_font);
        let (script_scroll, script_view) =
            script_text_view(mtm, &row, &text_font, width - GUTTER_WIDTH, height);

        let pane = Self {
            row,
            script_scroll: script_scroll.clone(),
            script_view: script_view.clone(),
            gutter_scroll,
            gutter_view,
            text_font,
            error_lines: RefCell::new(Vec::new()),
            bounds_observer: RefCell::new(None),
            text_observer: RefCell::new(None),
        };
        pane.layout(width, height);
        pane.update_gutter_numbers(DEFAULT_SCRIPT_SKELETON);
        pane.install_observers(on_text_changed);
        pane.apply_highlight();
        pane
    }

    pub fn layout(&self, width: f64, height: f64) {
        layout_editor_row(
            &self.row,
            &self.gutter_scroll,
            &self.script_scroll,
            &self.script_view,
            &self.gutter_view,
            width,
            height,
        );
    }

    pub fn gutter_scroll(&self) -> Retained<NSScrollView> {
        self.gutter_scroll.clone()
    }

    pub fn gutter_view(&self) -> Retained<NSTextView> {
        self.gutter_view.clone()
    }
}

/// 供窗口 resize 回调使用（不依赖 `ScriptEditorPane` 指针，避免悬空引用）。
pub fn layout_editor_row(
    row: &NSView,
    gutter_scroll: &NSScrollView,
    script_scroll: &NSScrollView,
    script_view: &NSTextView,
    gutter_view: &NSTextView,
    width: f64,
    height: f64,
) {
    unsafe {
        row.setFrameSize(NSSize::new(width.max(GUTTER_WIDTH + 80.0), height));
        gutter_scroll.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(GUTTER_WIDTH, height),
        ));
        script_scroll.setFrame(NSRect::new(
            NSPoint::new(GUTTER_WIDTH, 0.0),
            NSSize::new((width - GUTTER_WIDTH).max(80.0), height),
        ));
        let doc_h = height.max(120.0);
        script_view.setMinSize(NSSize::new(0.0, doc_h));
        gutter_view.setMinSize(NSSize::new(GUTTER_WIDTH - 8.0, doc_h));
    }
}

impl ScriptEditorPane {
    pub fn set_error_lines(&self, lines: Vec<usize>) {
        *self.error_lines.borrow_mut() = lines;
    }

    pub fn apply_highlight(&self) {
        let source = self.source_string();
        self.update_gutter_numbers(&source);
        let spans = highlight_meoflow(&source);
        let error_lines = self.error_lines.borrow().clone();
        unsafe {
            self.script_view.setRichText(true);
            let Some(storage) = self.script_view.textStorage() else {
                return;
            };
            let selection = self.script_view.selectedRange();
            let len = storage.length();
            if len == 0 {
                return;
            }
            let full = NSRange::new(0, len);
            let default_color = NSColor::textColor();
            storage.beginEditing();
            storage.addAttribute_value_range(
                &NSForegroundColorAttributeName,
                &default_color,
                full,
            );
            storage.addAttribute_value_range(&NSFontAttributeName, &self.text_font, full);
            storage.removeAttribute_range(&NSBackgroundColorAttributeName, full);

            for span in spans {
                let (loc, slen) = byte_range_to_utf16(&source, span.start, span.end);
                if slen == 0 {
                    continue;
                }
                let color = color_for_kind(span.kind);
                storage.addAttribute_value_range(
                    &NSForegroundColorAttributeName,
                    &color,
                    NSRange::new(loc as _, slen as _),
                );
            }

            for line_no in error_lines {
                if let Some((loc, slen)) = line_utf16_range(&source, line_no) {
                    if slen > 0 {
                        storage.addAttribute_value_range(
                            &NSBackgroundColorAttributeName,
                            &NSColor::systemRedColor().colorWithAlphaComponent(0.18),
                            NSRange::new(loc as _, slen as _),
                        );
                    }
                }
            }
            storage.endEditing();
            self.script_view.setSelectedRange(selection);
        }
    }

    pub fn update_gutter_numbers(&self, source: &str) {
        let text = gutter_text(line_count(source));
        unsafe {
            self.gutter_view.setString(&NSString::from_str(&text));
        }
    }

    pub fn remove_observers(&self) {
        let center = NSNotificationCenter::defaultCenter();
        if let Some(obs) = self.bounds_observer.borrow_mut().take() {
            unsafe { center.removeObserver(obs.as_ref()) };
        }
        if let Some(obs) = self.text_observer.borrow_mut().take() {
            unsafe { center.removeObserver(obs.as_ref()) };
        }
    }

    fn source_string(&self) -> String {
        unsafe { self.script_view.string().to_string() }
    }

    fn install_observers(
        &self,
        on_text_changed: &block2::RcBlock<dyn Fn(NonNull<NSNotification>)>,
    ) {
        unsafe {
            let clip = self.script_scroll.contentView();
            clip.setPostsBoundsChangedNotifications(true);
        }
        let pane_script = self.script_scroll.clone();
        let pane_gutter = self.gutter_scroll.clone();
        let bounds_block = block2::RcBlock::new(move |_notification: NonNull<NSNotification>| {
            let pane_script = pane_script.clone();
            let pane_gutter = pane_gutter.clone();
            unsafe {
                let script_clip = pane_script.contentView();
                let gutter_clip = pane_gutter.contentView();
                let origin = script_clip.bounds().origin;
                let mut bounds = gutter_clip.bounds();
                bounds.origin.y = origin.y;
                gutter_clip.scrollToPoint(bounds.origin);
                pane_gutter.reflectScrolledClipView(&gutter_clip);
            }
        });
        let center = NSNotificationCenter::defaultCenter();
        let bounds_obs = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSViewBoundsDidChangeNotification),
                Some(&self.script_scroll.contentView()),
                None,
                &bounds_block,
            )
        };
        *self.bounds_observer.borrow_mut() = Some(bounds_obs);

        let view = self.script_view.clone();
        let text_obs = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSTextDidChangeNotification),
                Some(&view),
                None,
                on_text_changed,
            )
        };
        *self.text_observer.borrow_mut() = Some(text_obs);
    }
}

fn color_for_kind(kind: TokenKind) -> Retained<NSColor> {
    match kind {
        TokenKind::Comment => NSColor::secondaryLabelColor(),
        TokenKind::Keyword => NSColor::systemIndigoColor(),
        TokenKind::String => NSColor::systemBrownColor(),
        TokenKind::Number => NSColor::systemOrangeColor(),
    }
}

fn line_utf16_range(source: &str, line_no: usize) -> Option<(usize, usize)> {
    if line_no == 0 {
        return None;
    }
    let mut current = 1usize;
    let mut utf16_start = 0usize;
    for line in source.split_inclusive('\n') {
        if current == line_no {
            return Some((utf16_start, utf16_len(line)));
        }
        utf16_start += utf16_len(line);
        current += 1;
    }
    None
}

fn gutter_scroll_view(mtm: MainThreadMarker, row: &NSView) -> Retained<NSScrollView> {
    unsafe {
        let scroll = NSScrollView::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(GUTTER_WIDTH, 200.0)),
        );
        scroll.setHasVerticalScroller(false);
        scroll.setHasHorizontalScroller(false);
        scroll.setBorderType(objc2_app_kit::NSBorderType::NoBorder);
        scroll.setDrawsBackground(true);
        scroll.setAutohidesScrollers(true);
        row.addSubview(&scroll);
        scroll
    }
}

fn gutter_text_view(
    mtm: MainThreadMarker,
    scroll: &NSScrollView,
    font: &NSFont,
) -> Retained<NSTextView> {
    unsafe {
        let view = NSTextView::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(GUTTER_WIDTH - 4.0, 200.0)),
        );
        view.setRichText(false);
        view.setEditable(false);
        view.setSelectable(false);
        view.setDrawsBackground(true);
        view.setBackgroundColor(&NSColor::controlBackgroundColor());
        view.setTextColor(Some(&NSColor::secondaryLabelColor()));
        view.setFont(Some(font));
        view.setVerticallyResizable(true);
        view.setHorizontallyResizable(false);
        view.setAutoresizingMask(objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable);
        scroll.setDocumentView(Some(&view));
        view
    }
}

fn script_text_view(
    mtm: MainThreadMarker,
    row: &NSView,
    font: &NSFont,
    width: f64,
    height: f64,
) -> (Retained<NSScrollView>, Retained<NSTextView>) {
    unsafe {
        let scroll = NSScrollView::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(GUTTER_WIDTH, 0.0), NSSize::new(width, height)),
        );
        scroll.setHasVerticalScroller(true);
        scroll.setBorderType(objc2_app_kit::NSBorderType::BezelBorder);
        scroll.setAutohidesScrollers(true);

        let view = NSTextView::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height)),
        );
        view.setMinSize(NSSize::new(0.0, height));
        view.setMaxSize(NSSize::new(f64::MAX, f64::MAX));
        view.setVerticallyResizable(true);
        view.setHorizontallyResizable(false);
        view.setAutoresizingMask(objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable);
        view.setRichText(true);
        view.setImportsGraphics(false);
        view.setEditable(true);
        view.setSelectable(true);
        view.setAllowsUndo(true);
        view.setAutomaticQuoteSubstitutionEnabled(false);
        view.setAutomaticDashSubstitutionEnabled(false);
        view.setAutomaticTextReplacementEnabled(false);
        view.setAutomaticSpellingCorrectionEnabled(false);
        view.setSmartInsertDeleteEnabled(false);
        view.setFont(Some(font));
        view.setString(&NSString::from_str(DEFAULT_SCRIPT_SKELETON));
        scroll.setDocumentView(Some(&view));
        row.addSubview(&scroll);
        (scroll, view)
    }
}
