//! 按钮编辑窗（外观 / 脚本）共享的 AppKit 表单辅助。

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{msg_send, Message};
use objc2_app_kit::{
    NSAlert, NSBox, NSBoxType, NSButton, NSColor, NSColorSpace, NSColorWell, NSControlStateValueOn,
    NSImage, NSImageScaling, NSImageView, NSPopUpButton, NSSwitch, NSTextField, NSView,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSPoint, NSRect, NSSize, NSString,
};

use crate::config::color::{color_to_string, parse_color, RgbColor};
use crate::config::button_presets::BUTTON_COLOR_PRESETS;
use crate::config::{ButtonScope, MatchRule};
use crate::platform::resolve_icon_path;

use super::icon_cache::decode_template_image;

pub(crate) const FIELD_X: f64 = 108.0;
pub(crate) const CONTENT_PAD: f64 = 16.0;
pub(crate) const PRESET_CELL: f64 = 52.0;
pub(crate) const PRESET_GAP: f64 = 8.0;
pub(crate) const PRESET_COLS: usize = 3;
pub(crate) const PREVIEW_SIZE: f64 = 64.0;
pub(crate) const BUTTON_CORNER_RADIUS: f64 = 8.0;

fn action_target<T: Message>(target: &T) -> &AnyObject {
    // SAFETY: `T` is an Objective-C object used as an action target.
    unsafe { &*(target as *const T as *const AnyObject) }
}

pub(crate) struct AppearanceUi {
    pub preview: Retained<NSView>,
    pub preview_button: Retained<NSButton>,
    pub appearance_hint: Retained<NSTextField>,
    pub preset_buttons: Vec<Retained<NSButton>>,
    pub bg_well: Retained<NSColorWell>,
    pub icon_well: Retained<NSColorWell>,
}

pub(crate) fn nonempty(s: String) -> Option<String> {
    let t = s.trim().to_string();
    (!t.is_empty()).then_some(t)
}

pub(crate) fn scope_from_ui(when_active: bool, bundle: String, app: String) -> ButtonScope {
    if when_active {
        ButtonScope::WhenActive(MatchRule {
            bundle_id: nonempty(bundle),
            app_name: nonempty(app),
            title_regex: None,
            title_regex_text: None,
        })
    } else {
        ButtonScope::Always
    }
}

pub(crate) fn labeled(
    mtm: MainThreadMarker,
    parent: &NSView,
    title: &str,
    field_width: f64,
    y: &mut f64,
) -> Retained<NSTextField> {
    labeled_pair(mtm, parent, title, field_width, y).1
}

pub(crate) fn labeled_pair(
    mtm: MainThreadMarker,
    parent: &NSView,
    title: &str,
    field_width: f64,
    y: &mut f64,
) -> (Retained<NSTextField>, Retained<NSTextField>) {
    unsafe {
        let lab = NSTextField::labelWithString(&NSString::from_str(title), mtm);
        lab.setFrame(NSRect::new(
            NSPoint::new(CONTENT_PAD, *y),
            NSSize::new(88.0, 22.0),
        ));
        let field = NSTextField::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(FIELD_X, *y - 2.0), NSSize::new(field_width, 24.0)),
        );
        field.setEditable(true);
        field.setBezeled(true);
        parent.addSubview(&lab);
        parent.addSubview(&field);
        *y += 36.0;
        (lab, field)
    }
}

pub(crate) fn scope_bundle_row<T: Message>(
    mtm: MainThreadMarker,
    parent: &NSView,
    y: &mut f64,
    target: &T,
) -> (
    Retained<NSTextField>,
    Retained<NSTextField>,
    Retained<NSButton>,
) {
    unsafe {
        let lab = NSTextField::labelWithString(&NSString::from_str("Scope Bundle"), mtm);
        lab.setFrame(NSRect::new(
            NSPoint::new(CONTENT_PAD, *y),
            NSSize::new(88.0, 22.0),
        ));
        let field = NSTextField::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(FIELD_X, *y - 2.0), NSSize::new(226.0, 24.0)),
        );
        field.setEditable(true);
        field.setBezeled(true);
        let btn = NSButton::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(344.0, *y - 3.0), NSSize::new(88.0, 26.0)),
        );
        btn.setTitle(&NSString::from_str("点选"));
        btn.setTarget(Some(action_target(target)));
        btn.setAction(Some(objc2::sel!(pickAppClicked:)));
        parent.addSubview(&lab);
        parent.addSubview(&field);
        parent.addSubview(&btn);
        *y += 36.0;
        (lab, field, btn)
    }
}

pub(crate) fn switch_row<T: Message>(
    mtm: MainThreadMarker,
    parent: &NSView,
    title: &str,
    y: &mut f64,
    target: &T,
) -> Retained<NSSwitch> {
    unsafe {
        let lab = NSTextField::labelWithString(&NSString::from_str(title), mtm);
        lab.setFrame(NSRect::new(
            NSPoint::new(CONTENT_PAD, *y),
            NSSize::new(88.0, 22.0),
        ));
        let sw = NSSwitch::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(FIELD_X, *y - 4.0), NSSize::new(42.0, 24.0)),
        );
        sw.setState(NSControlStateValueOn);
        let _ = target;
        parent.addSubview(&lab);
        parent.addSubview(&sw);
        *y += 36.0;
        sw
    }
}

pub(crate) fn icon_row<T: Message>(
    mtm: MainThreadMarker,
    parent: &NSView,
    y: &mut f64,
    target: &T,
) -> (Retained<NSTextField>, Retained<NSImageView>) {
    unsafe {
        let lab = NSTextField::labelWithString(&NSString::from_str("图标"), mtm);
        lab.setFrame(NSRect::new(
            NSPoint::new(CONTENT_PAD, *y),
            NSSize::new(88.0, 22.0),
        ));
        let preview = NSImageView::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(FIELD_X, *y - 6.0), NSSize::new(28.0, 28.0)),
        );
        preview.setEditable(false);
        preview.setImageScaling(NSImageScaling::ScaleProportionallyDown);
        let field = NSTextField::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(142.0, *y - 2.0), NSSize::new(214.0, 24.0)),
        );
        field.setEditable(true);
        field.setBezeled(true);
        field.setPlaceholderString(Some(&NSString::from_str("copy.png")));
        field.setTarget(Some(action_target(target)));
        field.setAction(Some(objc2::sel!(iconEdited:)));
        let pick = NSButton::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(364.0, *y - 3.0), NSSize::new(72.0, 26.0)),
        );
        pick.setTitle(&NSString::from_str("选择…"));
        pick.setTarget(Some(action_target(target)));
        pick.setAction(Some(objc2::sel!(pickIconClicked:)));
        let clear = NSButton::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(440.0, *y - 3.0), NSSize::new(56.0, 26.0)),
        );
        clear.setTitle(&NSString::from_str("清除"));
        clear.setTarget(Some(action_target(target)));
        clear.setAction(Some(objc2::sel!(clearIconClicked:)));
        parent.addSubview(&lab);
        parent.addSubview(&preview);
        parent.addSubview(&field);
        parent.addSubview(&pick);
        parent.addSubview(&clear);
        *y += 40.0;
        (field, preview)
    }
}

pub(crate) fn load_template_icon_image(
    mtm: MainThreadMarker,
    dirs: &[std::path::PathBuf],
    name: &str,
) -> Option<Retained<NSImage>> {
    if name.is_empty() {
        return None;
    }
    let path = resolve_icon_path(dirs, name)?;
    let bytes = std::fs::read(&path).ok()?;
    decode_template_image(mtm, &bytes)
}

pub(crate) fn load_icon_image(
    mtm: MainThreadMarker,
    dirs: &[std::path::PathBuf],
    name: &str,
) -> Option<Retained<NSImage>> {
    let path = resolve_icon_path(dirs, name)?;
    let image = NSImage::initWithContentsOfFile(
        mtm.alloc::<NSImage>(),
        &NSString::from_str(&path.to_string_lossy()),
    )?;
    image.setSize(NSSize::new(28.0, 28.0));
    Some(image)
}

pub(crate) fn popup<T: Message>(
    mtm: MainThreadMarker,
    parent: &NSView,
    title: &str,
    items: &[&str],
    y: &mut f64,
    target: &T,
    action: objc2::runtime::Sel,
) -> Retained<NSPopUpButton> {
    unsafe {
        let lab = NSTextField::labelWithString(&NSString::from_str(title), mtm);
        lab.setFrame(NSRect::new(
            NSPoint::new(CONTENT_PAD, *y),
            NSSize::new(88.0, 22.0),
        ));
        let p = NSPopUpButton::initWithFrame_pullsDown(
            mtm.alloc(),
            NSRect::new(NSPoint::new(FIELD_X, *y - 2.0), NSSize::new(364.0, 26.0)),
            false,
        );
        let titles: Vec<_> = items.iter().map(|s| NSString::from_str(s)).collect();
        p.addItemsWithTitles(&NSArray::from_retained_slice(&titles));
        p.setTarget(Some(action_target(target)));
        p.setAction(Some(action));
        parent.addSubview(&lab);
        parent.addSubview(&p);
        *y += 34.0;
        p
    }
}

pub(crate) fn show_alert(mtm: MainThreadMarker, title: &str, message: &str) {
    unsafe {
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(title));
        alert.setInformativeText(&NSString::from_str(message));
        alert.addButtonWithTitle(&NSString::from_str("好"));
        let _ = alert.runModal();
    }
}

pub(crate) fn footer_btn<T: Message>(
    mtm: MainThreadMarker,
    target: &T,
    title: &str,
    action: objc2::runtime::Sel,
) -> Retained<NSButton> {
    unsafe {
        let b = NSButton::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 12.0), NSSize::new(100.0, 28.0)),
        );
        b.setTitle(&NSString::from_str(title));
        b.setTarget(Some(action_target(target)));
        b.setAction(Some(action));
        b
    }
}

pub(crate) fn appearance_section<T: Message>(
    mtm: MainThreadMarker,
    parent: &NSView,
    y: &mut f64,
    target: &T,
) -> AppearanceUi {
    unsafe {
        let header = NSTextField::labelWithString(&NSString::from_str("外观"), mtm);
        header.setFrame(NSRect::new(
            NSPoint::new(CONTENT_PAD, *y),
            NSSize::new(88.0, 22.0),
        ));
        parent.addSubview(&header);
        *y += 28.0;

        let preview_lab = NSTextField::labelWithString(&NSString::from_str("预览"), mtm);
        preview_lab.setFrame(NSRect::new(
            NSPoint::new(CONTENT_PAD, *y + 18.0),
            NSSize::new(88.0, 22.0),
        ));
        let preview = NSView::initWithFrame(
            mtm.alloc(),
            NSRect::new(
                NSPoint::new(FIELD_X, *y),
                NSSize::new(PREVIEW_SIZE, PREVIEW_SIZE),
            ),
        );
        let preview_button = NSButton::initWithFrame(
            mtm.alloc(),
            NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(PREVIEW_SIZE, PREVIEW_SIZE),
            ),
        );
        preview_button.setBordered(false);
        preview_button.setTitle(&NSString::from_str(""));
        preview_button.setEnabled(false);
        preview.addSubview(&preview_button);

        let appearance_hint = NSTextField::labelWithString(
            &NSString::from_str("继承全局主题"),
            mtm,
        );
        appearance_hint.setFrame(NSRect::new(
            NSPoint::new(184.0, *y + 4.0),
            NSSize::new(320.0, 18.0),
        ));
        appearance_hint.setTextColor(Some(&NSColor::secondaryLabelColor()));

        parent.addSubview(&preview_lab);
        parent.addSubview(&preview);
        parent.addSubview(&appearance_hint);
        *y += PREVIEW_SIZE + 12.0;

        let preset_lab = NSTextField::labelWithString(&NSString::from_str("快速配色"), mtm);
        preset_lab.setFrame(NSRect::new(
            NSPoint::new(CONTENT_PAD, *y),
            NSSize::new(88.0, 22.0),
        ));
        parent.addSubview(&preset_lab);
        *y += 26.0;

        let mut preset_buttons = Vec::with_capacity(BUTTON_COLOR_PRESETS.len());
        for (index, preset) in BUTTON_COLOR_PRESETS.iter().enumerate() {
            let col = index % PRESET_COLS;
            let row = index / PRESET_COLS;
            let x = FIELD_X + col as f64 * (PRESET_CELL + PRESET_GAP);
            let cell_y = *y + row as f64 * (PRESET_CELL + PRESET_GAP);
            preset_buttons.push(make_preset_button(
                mtm,
                parent,
                x,
                cell_y,
                index,
                preset,
                target,
            ));
        }
        let preset_rows = BUTTON_COLOR_PRESETS.len().div_ceil(PRESET_COLS);
        *y += preset_rows as f64 * (PRESET_CELL + PRESET_GAP) + 8.0;

        let bg_well = color_well_row(
            mtm,
            parent,
            y,
            "背景颜色",
            target,
            objc2::sel!(bgColorChanged:),
            objc2::sel!(inheritBgClicked:),
        );
        let icon_well = color_well_row(
            mtm,
            parent,
            y,
            "图标颜色",
            target,
            objc2::sel!(iconColorChanged:),
            objc2::sel!(inheritIconClicked:),
        );

        AppearanceUi {
            preview,
            preview_button,
            appearance_hint,
            preset_buttons,
            bg_well,
            icon_well,
        }
    }
}

pub(crate) fn color_well_row<T: Message>(
    mtm: MainThreadMarker,
    parent: &NSView,
    y: &mut f64,
    title: &str,
    target: &T,
    well_action: objc2::runtime::Sel,
    inherit_action: objc2::runtime::Sel,
) -> Retained<NSColorWell> {
    unsafe {
        let lab = NSTextField::labelWithString(&NSString::from_str(title), mtm);
        lab.setFrame(NSRect::new(
            NSPoint::new(CONTENT_PAD, *y),
            NSSize::new(88.0, 22.0),
        ));
        let well = NSColorWell::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(FIELD_X, *y - 2.0), NSSize::new(44.0, 24.0)),
        );
        well.setTarget(Some(action_target(target)));
        well.setAction(Some(well_action));
        let inherit = NSButton::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(164.0, *y - 3.0), NSSize::new(88.0, 26.0)),
        );
        inherit.setTitle(&NSString::from_str("继承全局"));
        inherit.setTarget(Some(action_target(target)));
        inherit.setAction(Some(inherit_action));
        parent.addSubview(&lab);
        parent.addSubview(&well);
        parent.addSubview(&inherit);
        *y += 36.0;
        well
    }
}

pub(crate) fn make_preset_button<T: Message>(
    mtm: MainThreadMarker,
    parent: &NSView,
    x: f64,
    y: f64,
    index: usize,
    preset: &crate::config::ButtonColorPreset,
    target: &T,
) -> Retained<NSButton> {
    unsafe {
        let btn = NSButton::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(x, y), NSSize::new(PRESET_CELL, PRESET_CELL)),
        );
        btn.setBordered(false);
        btn.setTitle(&NSString::from_str(""));
        btn.setTag(index as isize);
        btn.setTarget(Some(action_target(target)));
        btn.setAction(Some(objc2::sel!(presetClicked:)));
        btn.setToolTip(Some(&NSString::from_str(&format!(
            "{}：{} / {}",
            preset.name, preset.background_color, preset.icon_color
        ))));
        btn.setWantsLayer(true);
        if let Some(layer) = btn.layer() {
            let _: () = msg_send![&*layer, setCornerRadius: BUTTON_CORNER_RADIUS];
            let _: () = msg_send![&*layer, setMasksToBounds: true];
            if let Some(bg) = parse_color(preset.background_color) {
                let color = ns_color_from_rgb(bg);
                let _: () = msg_send![&*layer, setBackgroundColor: &*color.CGColor()];
            }
        }

        let dot = NSBox::initWithFrame(
            mtm.alloc(),
            NSRect::new(
                NSPoint::new((PRESET_CELL - 14.0) / 2.0, (PRESET_CELL - 14.0) / 2.0),
                NSSize::new(14.0, 14.0),
            ),
        );
        dot.setBoxType(NSBoxType::Custom);
        dot.setTitlePosition(objc2_app_kit::NSTitlePosition::NoTitle);
        dot.setBorderWidth(0.0);
        if let Some(icon) = parse_color(preset.icon_color) {
            dot.setFillColor(&ns_color_from_rgb(icon));
        }
        btn.addSubview(&dot);

        parent.addSubview(&btn);
        btn
    }
}

pub(crate) fn style_preset_selection(btn: &NSButton, selected: bool) {
    unsafe {
        if let Some(layer) = btn.layer() {
            if selected {
                let accent = NSColor::controlAccentColor();
                let _: () = msg_send![&*layer, setBorderWidth: 2.0_f64];
                let _: () = msg_send![&*layer, setBorderColor: &*accent.CGColor()];
            } else {
                let _: () = msg_send![&*layer, setBorderWidth: 0.0_f64];
            }
        }
    }
}

pub(crate) fn paint_view_background(view: &NSView, color: RgbColor, corner_radius: f64) {
    unsafe {
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            let _: () = msg_send![&*layer, setCornerRadius: corner_radius];
            let _: () = msg_send![&*layer, setMasksToBounds: true];
            let ns = ns_color_from_rgb(color);
            let _: () = msg_send![&*layer, setBackgroundColor: &*ns.CGColor()];
        }
    }
}

pub(crate) fn set_well_color(well: &NSColorWell, rgb: RgbColor) {
    unsafe {
        well.setColor(&ns_color_from_rgb(rgb));
    }
}

pub(crate) fn well_to_string(well: &NSColorWell) -> String {
    let color = unsafe { well.color() };
    let srgb = unsafe { color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace()) };
    let Some(c) = srgb else {
        return "transparent".into();
    };
    let r = (unsafe { c.redComponent() } * 255.0).round().clamp(0.0, 255.0) as u8;
    let g = (unsafe { c.greenComponent() } * 255.0).round().clamp(0.0, 255.0) as u8;
    let b = (unsafe { c.blueComponent() } * 255.0).round().clamp(0.0, 255.0) as u8;
    let a = (unsafe { c.alphaComponent() } * 255.0).round().clamp(0.0, 255.0) as u8;
    color_to_string(RgbColor::rgba(r, g, b, a))
}

pub(crate) fn ns_color_from_rgb(c: RgbColor) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        c.r as f64 / 255.0,
        c.g as f64 / 255.0,
        c.b as f64 / 255.0,
        c.a as f64 / 255.0,
    )
}

#[cfg(test)]
mod tests {
    use super::scope_from_ui;
    use crate::config::ButtonScope;

    #[test]
    fn always_scope_ignores_bundle_fields() {
        assert!(matches!(
            scope_from_ui(false, "com.apple.Safari".into(), "Safari".into()),
            ButtonScope::Always
        ));
    }

    #[test]
    fn when_active_writes_pick_result() {
        match scope_from_ui(true, " com.apple.Safari ".into(), "Safari".into()) {
            ButtonScope::WhenActive(rule) => {
                assert_eq!(rule.bundle_id.as_deref(), Some("com.apple.Safari"));
                assert_eq!(rule.app_name.as_deref(), Some("Safari"));
            }
            other => panic!("expected WhenActive, got {other:?}"),
        }
    }
}
