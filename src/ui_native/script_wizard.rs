//! 动作脚本参数采集（Native AppKit 对话框）。

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertSecondButtonReturn, NSAlertStyle, NSApplication,
    NSBorderType, NSEvent, NSEventMask, NSModalResponseOK, NSOpenPanel, NSScrollView, NSTextField,
    NSTextView,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSPoint, NSRect, NSSize, NSString, NSURL};

use crate::config::FlowProgram;

static HOTKEY_CAPTURE: OnceLock<Mutex<Option<String>>> = OnceLock::new();

/// 打开应用预设：浏览 .app 或点选窗口。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAppChoice {
    BrowseApp,
    PickWindow,
}

/// 流程模板插入冲突时的选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowInsertChoice {
    Append,
    ReplaceAll,
}

/// 单行 / 多行文本输入。
pub fn prompt_text(
    mtm: MainThreadMarker,
    title: &str,
    message: &str,
    multiline: bool,
    default: &str,
) -> Option<String> {
    if multiline {
        prompt_multiline(mtm, title, message, default)
    } else {
        prompt_singleline(mtm, title, message, default)
    }
}

fn prompt_singleline(
    mtm: MainThreadMarker,
    title: &str,
    message: &str,
    default: &str,
) -> Option<String> {
    let alert = NSAlert::new(mtm);
    unsafe {
        alert.setAlertStyle(NSAlertStyle::Informational);
        alert.setMessageText(&NSString::from_str(title));
        alert.setInformativeText(&NSString::from_str(message));
        let field = NSTextField::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(300.0, 24.0)),
        );
        field.setStringValue(&NSString::from_str(default));
        alert.setAccessoryView(Some(&field));
        alert.addButtonWithTitle(&NSString::from_str("确定"));
        alert.addButtonWithTitle(&NSString::from_str("取消"));
        if alert.runModal() != NSAlertFirstButtonReturn {
            return None;
        }
        let s = field.stringValue().to_string();
        let t = s.trim();
        if t.is_empty() {
            None
        } else if multiline_hint(title) {
            Some(s)
        } else {
            Some(t.to_string())
        }
    }
}

fn multiline_hint(title: &str) -> bool {
    title.contains("粘贴") || title.contains("命令")
}

fn prompt_multiline(
    mtm: MainThreadMarker,
    title: &str,
    message: &str,
    default: &str,
) -> Option<String> {
    // 使用 NSAlert + accessory，避免自管 nextEvent 嵌套 RunLoop（在 tick/定时器里会 SIGABRT）。
    let alert = NSAlert::new(mtm);
    let (scroll, view) = unsafe {
        alert.setAlertStyle(NSAlertStyle::Informational);
        alert.setMessageText(&NSString::from_str(title));
        alert.setInformativeText(&NSString::from_str(message));

        let scroll = NSScrollView::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(360.0, 140.0)),
        );
        scroll.setHasVerticalScroller(true);
        scroll.setBorderType(NSBorderType::BezelBorder);
        scroll.setAutoresizingMask(
            objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let view = NSTextView::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(360.0, 140.0)),
        );
        view.setMinSize(NSSize::new(0.0, 140.0));
        view.setMaxSize(NSSize::new(f64::MAX, f64::MAX));
        view.setVerticallyResizable(true);
        view.setHorizontallyResizable(false);
        view.setAutoresizingMask(objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable);
        view.setString(&NSString::from_str(default));
        scroll.setDocumentView(Some(&view));
        alert.setAccessoryView(Some(&scroll));
        alert.addButtonWithTitle(&NSString::from_str("确定"));
        alert.addButtonWithTitle(&NSString::from_str("取消"));
        alert.layout();
        (scroll, view)
    };
    let _ = scroll;
    if alert.runModal() != NSAlertFirstButtonReturn {
        return None;
    }
    let result = unsafe { view.string().to_string() };
    let t = result.trim();
    if t.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// URL 输入；空则取消；无 scheme 时补 `https://`。
pub fn prompt_url(mtm: MainThreadMarker) -> Option<String> {
    let raw = prompt_singleline(
        mtm,
        "打开链接",
        "输入要在浏览器中打开的网址",
        "https://",
    )?;
    if raw.contains("://") {
        Some(raw)
    } else {
        Some(format!("https://{raw}"))
    }
}

/// 等待毫秒数；默认 200；支持末尾 `ms`；非法或超上限时提示并取消。
pub fn prompt_wait_ms(mtm: MainThreadMarker) -> Option<u64> {
    let raw = prompt_singleline(
        mtm,
        "等待",
        &format!(
            "输入等待时间（毫秒），范围 0–{}。确定后插入「等待 …」行。",
            FlowProgram::MAX_WAIT_MS
        ),
        "200",
    )?;
    let t = raw.trim().to_lowercase();
    let t = t.strip_suffix("ms").unwrap_or(&t).trim();
    if let Ok(ms) = t.parse::<u64>() {
        if ms <= FlowProgram::MAX_WAIT_MS {
            return Some(ms);
        }
    }
    let alert = NSAlert::new(mtm);
    unsafe {
        alert.setAlertStyle(NSAlertStyle::Warning);
        alert.setMessageText(&NSString::from_str("无效的等待时间"));
        alert.setInformativeText(&NSString::from_str(&format!(
            "请输入 0–{} 之间的整数（可带 ms 后缀）。",
            FlowProgram::MAX_WAIT_MS
        )));
        alert.addButtonWithTitle(&NSString::from_str("确定"));
        alert.runModal();
    }
    None
}

/// Shell 命令（多行）。
pub fn prompt_shell(mtm: MainThreadMarker) -> Option<String> {
    prompt_multiline(
        mtm,
        "执行命令",
        "通过 sh -c 执行。请谨慎使用，权限与直接选 Shell 相同。",
        "",
    )
}

/// 录制快捷键（模态，Esc 取消）。
pub fn prompt_hotkey(mtm: MainThreadMarker) -> Option<String> {
    let alert = NSAlert::new(mtm);
    unsafe {
        alert.setAlertStyle(NSAlertStyle::Informational);
        alert.setMessageText(&NSString::from_str("录制快捷键"));
        alert.setInformativeText(&NSString::from_str(
            "请按下要模拟的快捷键组合（Esc 取消）",
        ));
        alert.addButtonWithTitle(&NSString::from_str("取消"));
    }

    *HOTKEY_CAPTURE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap() = None;

    let block = block2::RcBlock::new(
        |event: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
            let event = unsafe { event.as_ref() };
            if event.keyCode() == 53 {
                return event as *const NSEvent as *mut NSEvent;
            }
            if let Some(spec) = event_to_hotkey_spec(event) {
                if let Ok(mut g) = HOTKEY_CAPTURE.get_or_init(|| Mutex::new(None)).lock() {
                    *g = Some(spec);
                }
                let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
                app.stopModalWithCode(NSAlertFirstButtonReturn);
                return std::ptr::null_mut();
            }
            event as *const NSEvent as *mut NSEvent
        },
    );
    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &block)
    };
    unsafe {
        alert.runModal();
    }
    if let Some(mon) = monitor {
        unsafe { NSEvent::removeMonitor(&mon) };
    }
    HOTKEY_CAPTURE
        .get()
        .and_then(|m| m.lock().ok())
        .and_then(|mut g| g.take())
}

fn event_to_hotkey_spec(event: &NSEvent) -> Option<String> {
    use objc2_app_kit::NSEventModifierFlags;
    let flags = event.modifierFlags();
    let chars = event
        .charactersIgnoringModifiers()
        .map(|s| s.to_string())
        .unwrap_or_default();
    let keycode = event.keyCode();
    if chars.is_empty() && matches!(keycode, 54 | 55 | 56 | 57 | 58 | 59 | 60 | 61 | 62 | 63) {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    if flags.contains(NSEventModifierFlags::Command) {
        parts.push("Cmd".into());
    }
    if flags.contains(NSEventModifierFlags::Control) {
        parts.push("Ctrl".into());
    }
    if flags.contains(NSEventModifierFlags::Option) {
        parts.push("Alt".into());
    }
    if flags.contains(NSEventModifierFlags::Shift) {
        parts.push("Shift".into());
    }
    let key = keycode_to_name(keycode, &chars)?;
    if key.eq_ignore_ascii_case("Escape") {
        return None;
    }
    parts.push(key);
    Some(parts.join("+"))
}

fn keycode_to_name(keycode: u16, chars: &str) -> Option<String> {
    let name: Option<&str> = match keycode {
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
        _ => None,
    };
    if let Some(n) = name {
        return Some(n.to_string());
    }
    if chars.len() == 1 {
        return Some(chars.to_uppercase());
    }
    None
}

/// 打开应用：浏览或点选。
pub fn prompt_open_app_choice(mtm: MainThreadMarker) -> Option<OpenAppChoice> {
    let alert = NSAlert::new(mtm);
    unsafe {
        alert.setAlertStyle(NSAlertStyle::Informational);
        alert.setMessageText(&NSString::from_str("打开应用"));
        alert.setInformativeText(&NSString::from_str("选择如何指定要打开的应用"));
        alert.addButtonWithTitle(&NSString::from_str("选择 .app…"));
        alert.addButtonWithTitle(&NSString::from_str("点选窗口…"));
        alert.addButtonWithTitle(&NSString::from_str("取消"));
        let resp = alert.runModal();
        if resp == NSAlertFirstButtonReturn {
            Some(OpenAppChoice::BrowseApp)
        } else if resp == NSAlertSecondButtonReturn {
            Some(OpenAppChoice::PickWindow)
        } else {
            None
        }
    }
}

/// NSOpenPanel 选择 .app。
pub fn pick_app_path(mtm: MainThreadMarker) -> Option<String> {
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(false);
    #[allow(deprecated)]
    {
        let app_ext = NSString::from_str("app");
        panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&[app_ext])));
    }
    panel.setTitle(Some(&NSString::from_str("选择应用")));
    panel.setPrompt(Some(&NSString::from_str("选择")));
    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    let url = panel.URLs().firstObject()?;
    let path = url.path()?.to_string();
    Some(path_for_config(&path))
}

/// 选择要打开的文件或文件夹。
pub fn pick_open_file_path(mtm: MainThreadMarker, current: &str) -> Option<String> {
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(true);
    panel.setAllowsMultipleSelection(false);
    panel.setResolvesAliases(true);
    panel.setTitle(Some(&NSString::from_str("选择要打开的文件")));
    panel.setMessage(Some(&NSString::from_str("用系统默认应用打开所选文件或文件夹")));
    panel.setPrompt(Some(&NSString::from_str("选择")));
    if let Some(dir) = initial_directory_for_panel(current) {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&dir));
        panel.setDirectoryURL(Some(&url));
    }
    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    let url = panel.URLs().firstObject()?;
    let path = url.path()?.to_string();
    Some(path_for_config(&path))
}

fn initial_directory_for_panel(current: &str) -> Option<String> {
    let trimmed = current.trim().trim_matches(|c| c == '"' || c == '\'');
    if trimmed.is_empty() {
        return None;
    }
    let expanded = if trimmed == "~" {
        dirs::home_dir()?
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        dirs::home_dir()?.join(rest)
    } else {
        Path::new(trimmed).to_path_buf()
    };
    if expanded.is_dir() {
        Some(expanded.to_string_lossy().into_owned())
    } else {
        expanded
            .parent()
            .filter(|p| p.is_dir())
            .map(|p| p.to_string_lossy().into_owned())
    }
}

pub(crate) fn path_for_config(path: &str) -> String {
    let Some(home) = dirs::home_dir() else {
        return path.to_string();
    };
    let home = home.to_string_lossy();
    if path == home.as_ref() {
        return "~".into();
    }
    let prefix = format!("{home}/");
    if let Some(rest) = path.strip_prefix(&prefix) {
        format!("~/{rest}")
    } else {
        path.to_string()
    }
}

/// 非空脚本插入流程模板时的冲突选择。
pub fn prompt_flow_insert_choice(mtm: MainThreadMarker) -> Option<FlowInsertChoice> {
    let alert = NSAlert::new(mtm);
    unsafe {
        alert.setAlertStyle(NSAlertStyle::Informational);
        alert.setMessageText(&NSString::from_str("插入流程模板"));
        alert.setInformativeText(&NSString::from_str(
            "当前脚本已有内容。插入到光标处，还是替换全部？",
        ));
        alert.addButtonWithTitle(&NSString::from_str("插入到光标处"));
        alert.addButtonWithTitle(&NSString::from_str("替换全部"));
        alert.addButtonWithTitle(&NSString::from_str("取消"));
        let resp = alert.runModal();
        if resp == NSAlertFirstButtonReturn {
            Some(FlowInsertChoice::Append)
        } else if resp == NSAlertSecondButtonReturn {
            Some(FlowInsertChoice::ReplaceAll)
        } else {
            None
        }
    }
}

/// MeoFlow 语法速查。
pub fn show_syntax_help(mtm: MainThreadMarker) {
    let alert = NSAlert::new(mtm);
    unsafe {
        alert.setAlertStyle(NSAlertStyle::Informational);
        alert.setMessageText(&NSString::from_str("常用 MeoFlow 命令"));
        alert.setInformativeText(&NSString::from_str(include_str!(
            "../../docs/meoflow_cheatsheet.txt"
        )));
        alert.addButtonWithTitle(&NSString::from_str("关闭"));
        alert.runModal();
    }
}

/// 导入脚本：选择文本文件并读入 UTF-8。
pub fn pick_script_import(mtm: MainThreadMarker) -> Result<Option<(std::path::PathBuf, String)>, String> {
    let panel = NSOpenPanel::openPanel(mtm);
    unsafe {
        panel.setTitle(Some(&NSString::from_str("导入脚本")));
        panel.setMessage(Some(&NSString::from_str(
            "选择 MeoFlow 脚本文件（.meoflow / .txt）",
        )));
        panel.setPrompt(Some(&NSString::from_str("导入")));
        panel.setCanChooseFiles(true);
        panel.setCanChooseDirectories(false);
        panel.setAllowsMultipleSelection(false);
        #[allow(deprecated)]
        {
            let meo = NSString::from_str("meoflow");
            let txt = NSString::from_str("txt");
            let mf = NSString::from_str("mf");
            panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&[meo, txt, mf])));
        }
    }
    if panel.runModal() != NSModalResponseOK {
        return Ok(None);
    }
    let path = open_panel_path(&panel).ok_or_else(|| "未选择文件".to_string())?;
    let raw = std::fs::read(&path).map_err(|e| format!("读取失败：{e}"))?;
    let text = decode_script_bytes(&raw)?;
    Ok(Some((path, text)))
}

/// 导出脚本：选择保存路径。
pub fn pick_script_export_path(
    mtm: MainThreadMarker,
    default_name: &str,
) -> Option<std::path::PathBuf> {
    use objc2_app_kit::NSSavePanel;
    let panel = NSSavePanel::savePanel(mtm);
    unsafe {
        panel.setTitle(Some(&NSString::from_str("导出脚本")));
        panel.setMessage(Some(&NSString::from_str("保存为 MeoFlow 脚本文件")));
        panel.setPrompt(Some(&NSString::from_str("导出")));
        panel.setNameFieldStringValue(&NSString::from_str(default_name));
        panel.setCanCreateDirectories(true);
        panel.setExtensionHidden(false);
        #[allow(deprecated)]
        {
            let meo = NSString::from_str("meoflow");
            let txt = NSString::from_str("txt");
            panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&[meo, txt])));
        }
    }
    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    save_panel_path(&panel)
}

pub fn write_script_file(path: &std::path::Path, source: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
        }
    }
    std::fs::write(path, source.as_bytes()).map_err(|e| format!("写入失败：{e}"))
}

/// 导入时若编辑器已有内容，询问追加或替换。
pub fn prompt_script_import_choice(mtm: MainThreadMarker) -> Option<FlowInsertChoice> {
    let alert = NSAlert::new(mtm);
    unsafe {
        alert.setAlertStyle(NSAlertStyle::Informational);
        alert.setMessageText(&NSString::from_str("导入脚本"));
        alert.setInformativeText(&NSString::from_str(
            "当前脚本已有内容。插入到光标处，还是替换全部？",
        ));
        alert.addButtonWithTitle(&NSString::from_str("插入到光标处"));
        alert.addButtonWithTitle(&NSString::from_str("替换全部"));
        alert.addButtonWithTitle(&NSString::from_str("取消"));
        let resp = alert.runModal();
        if resp == NSAlertFirstButtonReturn {
            Some(FlowInsertChoice::Append)
        } else if resp == NSAlertSecondButtonReturn {
            Some(FlowInsertChoice::ReplaceAll)
        } else {
            None
        }
    }
}

fn decode_script_bytes(raw: &[u8]) -> Result<String, String> {
    let raw = raw.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(raw);
    String::from_utf8(raw.to_vec()).map_err(|_| "文件不是有效的 UTF-8 文本".into())
}

fn open_panel_path(panel: &NSOpenPanel) -> Option<std::path::PathBuf> {
    let url = panel.URLs().firstObject()?;
    let path = url.path()?.to_string();
    Some(std::path::PathBuf::from(path))
}

fn save_panel_path(panel: &objc2_app_kit::NSSavePanel) -> Option<std::path::PathBuf> {
    let url = panel.URL()?;
    let path = url.path()?.to_string();
    Some(std::path::PathBuf::from(path))
}
