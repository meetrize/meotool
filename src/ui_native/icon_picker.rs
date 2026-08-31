//! 图标选择：先按名称过滤，再为匹配项按需加载缩略图。

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSButton, NSCellImagePosition, NSImage, NSImageScaling, NSScrollView,
    NSSearchField, NSTextField, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};

use crate::platform::{list_available_icons, resolve_icon_path};

use super::events::{emit, IconPickTarget, NativeUiEvent};

const COLS: usize = 6;
const CELL: f64 = 52.0;
const GAP: f64 = 8.0;
/// 同时解码的缩略图上限，避免一次加载全部 2000+ PNG。
const MAX_TILES: usize = 48;

#[derive(Default)]
struct PickerTargetIvars;

#[derive(Default)]
struct FlippedDocIvars;

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "MeoToolIconPickerTarget"]
    #[ivars = PickerTargetIvars]
    struct PickerTarget;

    unsafe impl NSObjectProtocol for PickerTarget {}

    impl PickerTarget {
        #[unsafe(method(cancelClicked:))]
        fn cancel_clicked(&self, _s: &AnyObject) {
            emit_picker(PickerAction::Cancel);
        }
        #[unsafe(method(searchChanged:))]
        fn search_changed(&self, _s: &AnyObject) {
            emit_picker(PickerAction::Filter);
        }
        #[unsafe(method(tileClicked:))]
        fn tile_clicked(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            if tag >= 0 {
                emit_picker(PickerAction::Pick(tag as usize));
            }
        }
    }
);

impl PickerTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PickerTargetIvars);
        unsafe { msg_send![super(this), init] }
    }
}

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[name = "MeoToolIconPickerDoc"]
    #[ivars = FlippedDocIvars]
    struct FlippedDocView;

    impl FlippedDocView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

impl FlippedDocView {
    fn with_frame(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(FlippedDocIvars);
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

#[derive(Debug, Clone)]
enum PickerAction {
    Cancel,
    Filter,
    Pick(usize),
}

static PICKER_TX: OnceLock<Mutex<Option<Sender<PickerAction>>>> = OnceLock::new();

fn emit_picker(a: PickerAction) {
    if let Ok(g) = PICKER_TX.get_or_init(|| Mutex::new(None)).lock() {
        if let Some(tx) = g.as_ref() {
            let _ = tx.send(a);
        }
    }
}

pub struct IconPickerWindow {
    window: Retained<NSWindow>,
    #[allow(dead_code)]
    target: Retained<PickerTarget>,
    search: Retained<NSSearchField>,
    hint: Retained<NSTextField>,
    scroll: Retained<NSScrollView>,
    all_icons: Vec<String>,
    shown: Vec<String>,
    icon_dirs: Vec<PathBuf>,
    target_pick: Option<IconPickTarget>,
    action_rx: Receiver<PickerAction>,
    mtm: MainThreadMarker,
    front_held: bool,
}

impl IconPickerWindow {
    pub fn new(mtm: MainThreadMarker) -> Self {
        let (tx, rx) = mpsc::channel();
        *PICKER_TX.get_or_init(|| Mutex::new(None)).lock().unwrap() = Some(tx);
        let target = PickerTarget::new(mtm);
        let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable;
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc(),
                NSRect::new(NSPoint::new(240.0, 160.0), NSSize::new(460.0, 420.0)),
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe {
            window.setReleasedWhenClosed(false);
            window.setTitle(&NSString::from_str("选择图标"));
        }
        let content = window.contentView().unwrap();

        let search = unsafe {
            let s = NSSearchField::initWithFrame(
                mtm.alloc(),
                NSRect::new(NSPoint::new(16.0, 378.0), NSSize::new(428.0, 24.0)),
            );
            s.setPlaceholderString(Some(&NSString::from_str("输入名称过滤，如 copy、search")));
            s.setSendsSearchStringImmediately(true);
            s.setSendsWholeSearchString(false);
            s.setTarget(Some(&target));
            s.setAction(Some(objc2::sel!(searchChanged:)));
            content.addSubview(&s);
            s
        };
        let hint = unsafe {
            let t = NSTextField::labelWithString(
                &NSString::from_str("输入关键词后再显示图标，避免一次加载全部图标"),
                mtm,
            );
            t.setFrame(NSRect::new(
                NSPoint::new(16.0, 352.0),
                NSSize::new(428.0, 20.0),
            ));
            content.addSubview(&t);
            t
        };
        let scroll = unsafe {
            let s = NSScrollView::initWithFrame(
                mtm.alloc(),
                NSRect::new(NSPoint::new(16.0, 52.0), NSSize::new(428.0, 292.0)),
            );
            s.setHasVerticalScroller(true);
            s.setDrawsBackground(false);
            content.addSubview(&s);
            s
        };
        let cancel = unsafe {
            let b = NSButton::initWithFrame(
                mtm.alloc(),
                NSRect::new(NSPoint::new(354.0, 16.0), NSSize::new(90.0, 28.0)),
            );
            b.setTitle(&NSString::from_str("关闭"));
            b.setTarget(Some(&target));
            b.setAction(Some(objc2::sel!(cancelClicked:)));
            content.addSubview(&b);
            b
        };
        let _ = cancel;

        Self {
            window,
            target,
            search,
            hint,
            scroll,
            all_icons: Vec::new(),
            shown: Vec::new(),
            icon_dirs: Vec::new(),
            target_pick: None,
            action_rx: rx,
            mtm,
            front_held: false,
        }
    }

    pub fn open(&mut self, dirs: &[PathBuf], target: IconPickTarget) {
        self.target_pick = Some(target);
        self.icon_dirs = dirs.to_vec();
        self.all_icons = list_available_icons(dirs);
        unsafe {
            self.search.setStringValue(&NSString::from_str(""));
        }
        self.rebuild_grid();
        super::activation::present_utility_window(self.mtm, &self.window, &mut self.front_held);
    }

    pub fn hide(&mut self) {
        super::activation::dismiss_utility_window(self.mtm, &self.window, &mut self.front_held);
        self.clear_grid();
        self.all_icons.clear();
        self.shown.clear();
        self.icon_dirs.clear();
        self.target_pick = None;
    }

    pub fn poll(&mut self) {
        while let Ok(a) = self.action_rx.try_recv() {
            match a {
                PickerAction::Filter => self.rebuild_grid(),
                PickerAction::Pick(idx) => {
                    if let Some(name) = self.shown.get(idx).cloned() {
                        if let Some(target) = self.target_pick.clone() {
                            emit(NativeUiEvent::IconPicked { target, name });
                        }
                    }
                    self.hide();
                }
                PickerAction::Cancel => {
                    emit(NativeUiEvent::CancelIconPicker);
                    self.hide();
                }
            }
        }
    }

    fn rebuild_grid(&mut self) {
        let query = unsafe { self.search.stringValue().to_string() }
            .trim()
            .to_ascii_lowercase();
        let total = self.all_icons.len();
        self.clear_grid();
        self.shown.clear();

        if query.is_empty() {
            unsafe {
                self.hint.setStringValue(&NSString::from_str(&format!(
                    "共 {total} 个图标。输入名称后再显示预览（最多 {MAX_TILES} 个）"
                )));
            }
            return;
        }

        let matches: Vec<String> = self
            .all_icons
            .iter()
            .filter(|name| name.to_ascii_lowercase().contains(&query))
            .cloned()
            .collect();
        let match_count = matches.len();
        self.shown = matches.into_iter().take(MAX_TILES).collect();

        let hint = if match_count == 0 {
            format!("无匹配「{query}」的图标")
        } else if match_count > MAX_TILES {
            format!("匹配 {match_count} 个，仅显示前 {MAX_TILES} 个，请继续输入缩小范围")
        } else {
            format!("匹配 {match_count} 个，点击图标即可选用")
        };
        unsafe {
            self.hint.setStringValue(&NSString::from_str(&hint));
        }
        if self.shown.is_empty() {
            return;
        }

        let rows = self.shown.len().div_ceil(COLS);
        let width = 428.0;
        let height = (GAP + rows as f64 * (CELL + GAP)).max(292.0);
        let doc = FlippedDocView::with_frame(
            self.mtm,
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height)),
        );

        for (i, name) in self.shown.iter().enumerate() {
            let col = i % COLS;
            let row = i / COLS;
            let x = GAP + col as f64 * (CELL + GAP);
            let y = GAP + row as f64 * (CELL + GAP);
            let btn = unsafe {
                let b = NSButton::initWithFrame(
                    self.mtm.alloc(),
                    NSRect::new(NSPoint::new(x, y), NSSize::new(CELL, CELL)),
                );
                b.setBordered(false);
                b.setTitle(&NSString::from_str(""));
                b.setToolTip(Some(&NSString::from_str(name)));
                b.setTag(i as isize);
                b.setTarget(Some(&self.target));
                b.setAction(Some(objc2::sel!(tileClicked:)));
                if let Some(img) = load_tile_image(self.mtm, &self.icon_dirs, name) {
                    b.setImage(Some(&img));
                    b.setImagePosition(NSCellImagePosition::ImageOnly);
                    b.setImageScaling(NSImageScaling::ScaleProportionallyDown);
                } else {
                    b.setTitle(&NSString::from_str("?"));
                }
                doc.addSubview(&b);
                b
            };
            let _ = btn;
        }
        unsafe {
            self.scroll.setDocumentView(Some(&doc));
        }
    }

    fn clear_grid(&self) {
        unsafe {
            self.scroll.setDocumentView(None);
        }
    }
}

fn load_tile_image(
    mtm: MainThreadMarker,
    dirs: &[PathBuf],
    name: &str,
) -> Option<Retained<NSImage>> {
    let path = resolve_icon_path(dirs, name)?;
    let img = NSImage::initByReferencingFile(
        mtm.alloc::<NSImage>(),
        &NSString::from_str(&path.to_string_lossy()),
    )?;
    img.setSize(NSSize::new(36.0, 36.0));
    Some(img)
}
