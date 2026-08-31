//! 脚本编辑窗工具栏：快捷插入 / 流程模板为图标按钮组，其余为文字按钮。

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSButton, NSImage, NSScrollView, NSSegmentedControl, NSSegmentSwitchTracking, NSSegmentStyle,
    NSView,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSPoint, NSRect, NSSize, NSString};

use crate::config::script_presets::{ScriptPreset, FLOW_PRESETS, QUICK_PRESETS};

pub const SCRIPT_TOOLBAR_H: f64 = 40.0;

const BTN_H: f64 = 26.0;
const BTN_Y: f64 = 7.0;
const BTN_GAP: f64 = 6.0;
const GROUP_GAP: f64 = 12.0;
const PAD_X: f64 = 10.0;
const SEGMENT_W: f64 = 28.0;

pub struct ScriptToolbarStrip {
    pub view: Retained<NSView>,
    pub capture_finish_btn: Retained<NSButton>,
}

/// SF Symbol 名（按 preset.id）。
fn sf_symbol_for_preset(id: &str) -> &'static str {
    match id {
        "hotkey" => "keyboard",
        "open_app" => "macwindow",
        "open_url" => "link",
        "open_file" => "doc",
        "paste" => "doc.on.clipboard",
        "shell" => "terminal",
        "click_once" => "hand.point.up.left",
        "record_clicks" => "record.circle",
        "type_enter" => "return",
        "paste_enter" => "arrow.turn.down.left",
        "click_hotkey" => "cursorarrow.click",
        "open_app_type" => "app.badge.plus",
        _ => "plus",
    }
}

fn system_symbol(name: &str, description: &str) -> Option<Retained<NSImage>> {
    let img = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(name),
        Some(&NSString::from_str(description)),
    )?;
    unsafe {
        img.setTemplate(true);
    }
    Some(img)
}

fn icon_action_group(
    mtm: MainThreadMarker,
    parent: &NSView,
    target: &AnyObject,
    x: f64,
    items: &[(&str, &str, &str)],
    action: objc2::runtime::Sel,
) -> (Retained<NSSegmentedControl>, f64) {
    let images: Vec<Retained<NSImage>> = items
        .iter()
        .map(|(symbol, label, _)| {
            system_symbol(symbol, label).unwrap_or_else(|| {
                NSImage::imageNamed(&NSString::from_str("NSActionTemplate"))
                    .unwrap_or_else(|| NSImage::new())
            })
        })
        .collect();
    let image_refs: Vec<&NSImage> = images.iter().map(|i| i.as_ref()).collect();
    let arr = NSArray::from_slice(&image_refs);

    let control = unsafe {
        NSSegmentedControl::segmentedControlWithImages_trackingMode_target_action(
            &arr,
            NSSegmentSwitchTracking::Momentary,
            Some(target),
            Some(action),
            mtm,
        )
    };
    unsafe {
        control.setSegmentStyle(NSSegmentStyle::Rounded);
        for (i, (_, label, tip)) in items.iter().enumerate() {
            let i = i as isize;
            control.setWidth_forSegment(SEGMENT_W, i);
            let tip = format!("{label} — {tip}");
            control.setToolTip_forSegment(Some(&NSString::from_str(&tip)), i);
        }
        control.sizeToFit();
        let w = control.frame().size.width.max(items.len() as f64 * SEGMENT_W);
        control.setFrame(NSRect::new(
            NSPoint::new(x, BTN_Y),
            NSSize::new(w, BTN_H),
        ));
        parent.addSubview(&control);
        (control, x + w + GROUP_GAP)
    }
}

fn icon_button_group(
    mtm: MainThreadMarker,
    parent: &NSView,
    target: &AnyObject,
    x: f64,
    presets: &[ScriptPreset],
    action: objc2::runtime::Sel,
) -> (Retained<NSSegmentedControl>, f64) {
    let items: Vec<(&str, &str, &str)> = presets
        .iter()
        .map(|p| (sf_symbol_for_preset(p.id), p.label, p.tooltip))
        .collect();
    icon_action_group(mtm, parent, target, x, &items, action)
}

fn toolbar_button(
    mtm: MainThreadMarker,
    parent: &NSView,
    target: &AnyObject,
    x: f64,
    title: &str,
    tooltip: &str,
    tag: isize,
    action: objc2::runtime::Sel,
) -> (Retained<NSButton>, f64) {
    unsafe {
        let btn = NSButton::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(x, BTN_Y), NSSize::new(48.0, BTN_H)),
        );
        btn.setTitle(&NSString::from_str(title));
        btn.setBezelStyle(objc2_app_kit::NSBezelStyle::FlexiblePush);
        if !tooltip.is_empty() {
            btn.setToolTip(Some(&NSString::from_str(tooltip)));
        }
        btn.setTarget(Some(target));
        btn.setAction(Some(action));
        btn.setTag(tag);
        btn.sizeToFit();
        let w = btn.frame().size.width.max(48.0);
        btn.setFrame(NSRect::new(NSPoint::new(x, BTN_Y), NSSize::new(w, BTN_H)));
        parent.addSubview(&btn);
        (btn, x + w + BTN_GAP)
    }
}

pub fn install_toolbar_strip(
    mtm: MainThreadMarker,
    content: &NSView,
    width: f64,
    target: &AnyObject,
) -> ScriptToolbarStrip {
    let outer = unsafe {
        let v = NSView::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, SCRIPT_TOOLBAR_H)),
        );
        v.setAutoresizingMask(
            objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                | objc2_app_kit::NSAutoresizingMaskOptions::ViewMinYMargin,
        );
        v.setWantsLayer(true);
        if let Some(layer) = v.layer() {
            layer.setBackgroundColor(Some(
                &objc2_app_kit::NSColor::controlBackgroundColor().CGColor(),
            ));
        }
        content.addSubview(&v);
        v
    };

    let scroll = unsafe {
        let s = NSScrollView::initWithFrame(
            mtm.alloc(),
            NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(width, SCRIPT_TOOLBAR_H),
            ),
        );
        s.setHasVerticalScroller(false);
        s.setHasHorizontalScroller(true);
        s.setAutohidesScrollers(true);
        s.setBorderType(objc2_app_kit::NSBorderType::NoBorder);
        s.setAutoresizingMask(
            objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        outer.addSubview(&s);
        s
    };

    let doc = unsafe {
        let d = NSView::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, SCRIPT_TOOLBAR_H)),
        );
        scroll.setDocumentView(Some(&d));
        d
    };

    let mut x = PAD_X;
    // 快捷插入：图标按钮组
    let (_, next) = icon_button_group(
        mtm,
        &doc,
        target,
        x,
        QUICK_PRESETS,
        objc2::sel!(quickPresetClicked:),
    );
    x = next;
    // 流程模板：图标按钮组
    let (_, next) = icon_button_group(
        mtm,
        &doc,
        target,
        x,
        FLOW_PRESETS,
        objc2::sel!(flowPresetClicked:),
    );
    x = next;

    // 导入 / 导出
    let (_, next) = icon_action_group(
        mtm,
        &doc,
        target,
        x,
        &[
            (
                "square.and.arrow.down",
                "导入",
                "从 .meoflow / .txt 文件导入脚本",
            ),
            (
                "square.and.arrow.up",
                "导出",
                "将当前脚本导出为文件",
            ),
        ],
        objc2::sel!(scriptIoClicked:),
    );
    x = next;

    let mut x = x;
    (_, x) = toolbar_button(
        mtm,
        &doc,
        target,
        x,
        "捕获点击",
        "点击目标窗口一次，插入窗口相对坐标",
        0,
        objc2::sel!(scriptCaptureClicked:),
    );
    (_, x) = toolbar_button(
        mtm,
        &doc,
        target,
        x,
        "捕获应用",
        "点击目标应用窗口，插入激活应用行",
        0,
        objc2::sel!(scriptCaptureAppClicked:),
    );
    let (capture_finish_btn, next) = toolbar_button(
        mtm,
        &doc,
        target,
        x,
        "完成录制",
        "结束连续点击/按键录制并写入脚本",
        0,
        objc2::sel!(scriptFinishRecordClicked:),
    );
    x = next;
    unsafe {
        capture_finish_btn.setHidden(true);
    }

    x += GROUP_GAP - BTN_GAP;
    (_, x) = toolbar_button(
        mtm,
        &doc,
        target,
        x,
        "运行",
        "试运行脚本（请先点到目标窗口）",
        0,
        objc2::sel!(scriptTryRunClicked:),
    );
    (_, x) = toolbar_button(
        mtm,
        &doc,
        target,
        x,
        "帮助",
        "打开常用 MeoFlow 命令速查",
        0,
        objc2::sel!(syntaxHelpClicked:),
    );

    let content_w = x + PAD_X - BTN_GAP;
    unsafe {
        doc.setFrameSize(NSSize::new(content_w.max(width), SCRIPT_TOOLBAR_H));
    }

    ScriptToolbarStrip {
        view: outer,
        capture_finish_btn,
    }
}
