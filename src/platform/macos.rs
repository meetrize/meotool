use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, Default)]
pub struct PermissionStatus {
    pub accessibility: bool,
    pub input_monitoring: bool,
}

impl PermissionStatus {
    pub fn all_granted(&self) -> bool {
        self.accessibility && self.input_monitoring
    }
}

#[cfg(target_os = "macos")]
mod macos_impl {
    use std::ffi::c_void;
    use std::os::unix::process::CommandExt;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::Duration;

    use super::PermissionStatus;

    const K_AX_ERROR_SUCCESS: i32 = 0;
    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

    #[link(name = "ApplicationServices", kind = "framework")]
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXUIElementCreateSystemWide() -> *const c_void;
        fn AXUIElementCopyAttributeValue(
            element: *const c_void,
            attribute: *const c_void,
            value: *mut *const c_void,
        ) -> i32;
        fn CGPreflightListenEventAccess() -> bool;
        fn CFRelease(cf: *const c_void);
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            c_str: *const i8,
            encoding: u32,
        ) -> *const c_void;
    }

    pub fn executable_path() -> Option<PathBuf> {
        std::env::current_exe().ok()
    }

    /// macOS 会缓存 TCC 查询结果；授权后需重启进程才能刷新。
    pub fn relaunch_app() -> ! {
        let exe = std::env::current_exe().expect("current_exe");
        let args: Vec<String> = std::env::args().skip(1).collect();
        log::info!("relaunching to refresh permission state: {}", exe.display());
        let err = Command::new(&exe).args(&args).exec();
        panic!("failed to relaunch {}: {err}", exe.display());
    }

    pub fn check_permissions() -> PermissionStatus {
        PermissionStatus {
            accessibility: check_accessibility(),
            input_monitoring: check_input_monitoring(),
        }
    }

    /// 用真实 AX 调用探测，避免仅依赖可能过期的 AXIsProcessTrusted 缓存。
    fn check_accessibility() -> bool {
        if probe_accessibility() {
            return true;
        }
        unsafe { AXIsProcessTrusted() }
    }

    fn check_input_monitoring() -> bool {
        if probe_input_monitoring() {
            return true;
        }
        unsafe { CGPreflightListenEventAccess() }
    }

    fn probe_accessibility() -> bool {
        unsafe {
            let attr = CFStringCreateWithCString(
                std::ptr::null(),
                c"AXFocusedApplication".as_ptr(),
                K_CF_STRING_ENCODING_UTF8,
            );
            if attr.is_null() {
                return false;
            }

            let system = AXUIElementCreateSystemWide();
            if system.is_null() {
                CFRelease(attr);
                return false;
            }

            let mut value = std::ptr::null();
            let err = AXUIElementCopyAttributeValue(system, attr, &mut value);
            if !value.is_null() {
                CFRelease(value);
            }
            CFRelease(system);
            CFRelease(attr);

            err == K_AX_ERROR_SUCCESS
        }
    }

    fn probe_input_monitoring() -> bool {
        use global_hotkey::hotkey::{Code, HotKey, Modifiers};
        use global_hotkey::GlobalHotKeyManager;

        let Ok(manager) = GlobalHotKeyManager::new() else {
            return false;
        };
        let hotkey = HotKey::new(Some(Modifiers::empty()), Code::F13);
        manager.register(hotkey).is_ok()
    }

    pub fn open_accessibility_settings() {
        const URLS: &[&str] = &[
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Accessibility",
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
        ];
        open_first_settings_url(URLS);
    }

    pub fn open_input_monitoring_settings() {
        const URLS: &[&str] = &[
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_ListenEvent",
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent",
        ];
        open_first_settings_url(URLS);
    }

    fn open_first_settings_url(urls: &[&str]) {
        for url in urls {
            if Command::new("open").arg(url).spawn().is_ok() {
                return;
            }
        }
    }

    pub fn activate_app(window: &crate::config::WindowInfo) -> Result<(), String> {
        let pid = window.process_id;
        if pid == 0 {
            return Err("invalid process id".into());
        }
        activate_pid(pid)
    }

    #[allow(dead_code)]
    pub fn activate_self() -> Result<(), String> {
        activate_pid(u64::from(std::process::id()))
    }

    fn activate_pid(pid: u64) -> Result<(), String> {
        #[cfg(feature = "ui-native")]
        {
            use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
            let pid_t = i32::try_from(pid).map_err(|_| format!("invalid process id {pid}"))?;
            let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid_t)
            else {
                return Err(format!("no running application for pid {pid}"));
            };
            let ok = app.activateWithOptions(NSApplicationActivationOptions::empty());
            if !ok {
                return Err(format!("activate pid {pid} was rejected"));
            }
            return Ok(());
        }

        #[cfg(not(feature = "ui-native"))]
        {
            // egui 路径无 AppKit 绑定：退回 osascript（会阻塞，仅作兜底）
            let script = format!(
                "tell application \"System Events\" to set frontmost of (first process whose unix id is {pid}) to true"
            );
            let output = Command::new("osascript")
                .args(["-e", &script])
                .output()
                .map_err(|e| format!("osascript failed: {e}"))?;
            if !output.status.success() {
                return Err(format!(
                    "activate pid {pid} failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
            Ok(())
        }
    }

    const SAFARI_BUNDLE_ID: &str = "com.apple.Safari";

    pub fn fetch_browser_page_url(window: &crate::config::WindowInfo) -> Option<String> {
        if window.bundle_id.as_deref() != Some(SAFARI_BUNDLE_ID) {
            return None;
        }
        throttled_script_result(
            "safari_url",
            Duration::from_secs(1),
            fetch_safari_front_tab_url,
        )
    }

    const CURSOR_BUNDLE_ID: &str = "com.todesktop.230313mzl4w4u92";

    fn is_cursor_window(window: &crate::config::WindowInfo) -> bool {
        window.app_name.eq_ignore_ascii_case("Cursor")
            || window
                .bundle_id
                .as_deref()
                .is_some_and(|id| id == CURSOR_BUNDLE_ID)
    }

    /// CGWindow API 对 Cursor 等 Electron 应用通常拿不到标题，改用 AppleScript 补全。
    pub fn enrich_editor_window_title(window: &mut crate::config::WindowInfo) {
        if !is_cursor_window(window) {
            return;
        }
        if let Some(title) = throttled_script_result(
            "cursor_title",
            Duration::from_secs(1),
            fetch_cursor_front_window_title,
        ) {
            window.title = title;
        }
    }

    fn throttled_script_result(
        key: &'static str,
        min_interval: Duration,
        fetch: fn() -> Option<String>,
    ) -> Option<String> {
        use std::collections::HashMap;
        use std::sync::Mutex;
        use std::time::Instant;

        struct Slot {
            at: Instant,
            value: Option<String>,
        }

        static CACHE: Mutex<Option<HashMap<&'static str, Slot>>> = Mutex::new(None);

        let mut guard = CACHE.lock().ok()?;
        let map = guard.get_or_insert_with(HashMap::new);
        if let Some(slot) = map.get(key) {
            if slot.at.elapsed() < min_interval {
                return slot.value.clone();
            }
        }
        let value = fetch();
        map.insert(
            key,
            Slot {
                at: Instant::now(),
                value: value.clone(),
            },
        );
        value
    }

    fn fetch_cursor_front_window_title() -> Option<String> {
        const SCRIPT: &str = r#"try
    tell application "System Events"
        tell application process "Cursor"
            if (count of windows) = 0 then return ""
            return name of front window
        end tell
    end tell
on error
    return ""
end try"#;

        let output = Command::new("osascript")
            .args(["-e", SCRIPT])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let title = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if title.is_empty() {
            None
        } else {
            Some(title)
        }
    }

    fn fetch_safari_front_tab_url() -> Option<String> {
        const SCRIPT: &str = r#"try
    tell application "Safari"
        if (count of windows) = 0 then return ""
        return URL of current tab of front window
    end tell
on error
    return ""
end try"#;

        let output = Command::new("osascript")
            .args(["-e", SCRIPT])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if url.is_empty() {
            None
        } else {
            Some(url)
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod macos_impl {
    use std::path::PathBuf;

    use super::PermissionStatus;
    use crate::config::WindowInfo;

    pub fn executable_path() -> Option<PathBuf> {
        std::env::current_exe().ok()
    }

    pub fn relaunch_app() -> ! {
        std::process::exit(0);
    }

    pub fn check_permissions() -> PermissionStatus {
        PermissionStatus {
            accessibility: true,
            input_monitoring: true,
        }
    }

    pub fn open_accessibility_settings() {}
    pub fn open_input_monitoring_settings() {}

    pub fn activate_app(_window: &WindowInfo) -> Result<(), String> {
        Ok(())
    }

    pub fn activate_self() -> Result<(), String> {
        Ok(())
    }

    pub fn fetch_browser_page_url(_window: &WindowInfo) -> Option<String> {
        None
    }

    pub fn enrich_editor_window_title(_window: &mut WindowInfo) {}
}

pub fn executable_path() -> Option<std::path::PathBuf> {
    macos_impl::executable_path()
}

pub fn relaunch_app() -> ! {
    macos_impl::relaunch_app()
}

pub fn check_permissions() -> PermissionStatus {
    macos_impl::check_permissions()
}

pub fn open_accessibility_settings() {
    macos_impl::open_accessibility_settings();
}

pub fn open_input_monitoring_settings() {
    macos_impl::open_input_monitoring_settings();
}

pub fn activate_app(window: &crate::config::WindowInfo) -> Result<(), String> {
    macos_impl::activate_app(window)
}

/// 将本应用激活并置于最前
#[cfg(feature = "ui-egui")]
pub fn activate_self() -> Result<(), String> {
    macos_impl::activate_self()
}

/// 当前鼠标的全局屏幕坐标（点，原点在主屏左上，y 向下）
pub fn global_mouse_pos() -> Option<(f32, f32)> {
    #[cfg(target_os = "macos")]
    {
        use core_graphics::event::CGEvent;
        use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
        let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState).ok()?;
        let event = CGEvent::new(source).ok()?;
        let p = event.location();
        Some((p.x as f32, p.y as f32))
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

pub fn fetch_browser_page_url(window: &crate::config::WindowInfo) -> Option<String> {
    macos_impl::fetch_browser_page_url(window)
}

pub fn enrich_editor_window_title(window: &mut crate::config::WindowInfo) {
    macos_impl::enrich_editor_window_title(window);
}

/// 由 PID 解析可执行路径（`proc_pidpath`），供点选窗口反查 Bundle ID。
#[cfg(target_os = "macos")]
pub fn process_path_from_pid(pid: u64) -> Option<std::path::PathBuf> {
    const PROC_PIDPATHINFO_MAXSIZE: u32 = 4096;
    let mut buf = vec![0u8; PROC_PIDPATHINFO_MAXSIZE as usize];
    let n = unsafe { proc_pidpath(pid as i32, buf.as_mut_ptr().cast(), PROC_PIDPATHINFO_MAXSIZE) };
    if n <= 0 {
        return None;
    }
    let len = buf.iter().position(|&b| b == 0).unwrap_or(n as usize);
    let path = std::str::from_utf8(&buf[..len]).ok()?;
    if path.is_empty() {
        None
    } else {
        Some(std::path::PathBuf::from(path))
    }
}

#[cfg(target_os = "macos")]
extern "C" {
    fn proc_pidpath(pid: i32, buffer: *mut std::ffi::c_char, buffersize: u32) -> i32;
}

#[cfg(not(target_os = "macos"))]
pub fn process_path_from_pid(_pid: u64) -> Option<std::path::PathBuf> {
    None
}

#[cfg(target_os = "macos")]
pub fn bundle_id_from_process_path(process_path: &Path) -> Option<String> {
    lazy_bundle_cache(|cache| {
        if let Some(cached) = cache.get(process_path) {
            return Some(cached.clone());
        }
        let resolved = resolve_bundle_id_from_process_path(process_path);
        if let Some(id) = resolved.clone() {
            if cache.len() > 256 {
                cache.clear();
            }
            cache.insert(process_path.to_path_buf(), id);
        }
        resolved
    })
}

#[cfg(target_os = "macos")]
fn lazy_bundle_cache<R>(f: impl FnOnce(&mut HashMap<std::path::PathBuf, String>) -> R) -> R {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static CACHE: Mutex<Option<HashMap<std::path::PathBuf, String>>> = Mutex::new(None);
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    f(map)
}

#[cfg(target_os = "macos")]
fn resolve_bundle_id_from_process_path(process_path: &Path) -> Option<String> {
    let mut current: Option<&Path> = Some(process_path);
    while let Some(path) = current {
        if path.extension().and_then(|e| e.to_str()) == Some("app") {
            let plist = path.join("Contents/Info.plist");
            return read_bundle_id_from_plist(&plist);
        }
        current = path.parent();
    }
    None
}

#[cfg(not(target_os = "macos"))]
pub fn bundle_id_from_process_path(_process_path: &Path) -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
fn read_bundle_id_from_plist(path: &Path) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    let value: plist::Value = plist::from_bytes(&data).ok()?;
    value
        .as_dictionary()
        .and_then(|dict| dict.get("CFBundleIdentifier"))
        .and_then(|v| v.as_string())
        .map(|s| s.to_string())
}

pub fn check_accessibility_hint() {
    let status = check_permissions();
    if status.all_granted() {
        return;
    }
    log::warn!(
        "MeoTool 需要「辅助功能」与「输入监控」权限。\n\
         启动后将显示引导对话框。"
    );
}
