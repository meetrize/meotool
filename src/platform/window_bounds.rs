//! 前台窗口 bounds 与屏幕/归一化坐标互转。
//!
//! 坐标约定与 CGEvent / enigo Absolute 一致：原点在主屏左上角，x 向右、y 向下。

use std::process::Command;

use active_win_pos_rs::get_active_window;

use crate::config::CoordSpace;
use crate::platform::{bundle_id_from_process_path, process_path_from_pid};

/// CGWindowList 中的一层窗口（前→后顺序）。
#[derive(Debug, Clone, PartialEq)]
pub struct ListedWindow {
    pub process_id: u64,
    pub app_name: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl ListedWindow {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// 点击坐标命中的应用窗口身份。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HitWindow {
    pub process_id: u64,
    pub app_name: String,
    pub bundle_id: Option<String>,
}

const HIT_MIN_SIZE: f64 = 8.0;
const PLAYBACK_MIN_SIZE: f64 = 50.0;

#[derive(Debug, Clone, PartialEq)]
pub struct WindowBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub process_id: u64,
    pub bundle_id: Option<String>,
}

impl WindowBounds {
    #[cfg(any(feature = "ui-egui", feature = "ui-native", test))]
    pub fn screen_to_normalized(&self, screen_x: f64, screen_y: f64) -> Option<(f32, f32)> {
        if self.width <= 1.0 || self.height <= 1.0 {
            return None;
        }
        let nx = ((screen_x - self.x) / self.width).clamp(0.0, 1.0);
        let ny = ((screen_y - self.y) / self.height).clamp(0.0, 1.0);
        Some((nx as f32, ny as f32))
    }

    pub fn normalized_to_screen(&self, nx: f32, ny: f32) -> (f64, f64) {
        let sx = self.x + f64::from(nx.clamp(0.0, 1.0)) * self.width;
        let sy = self.y + f64::from(ny.clamp(0.0, 1.0)) * self.height;
        (sx, sy)
    }
}

/// 获取当前前台窗口 bounds（排除自身进程）。
///
/// Android Studio 等 Java 应用上，`active-win-pos-rs` 有时会返回 0 尺寸；
/// 此时改用 CGWindowList 按 PID 取最大可见窗口。
pub fn frontmost_window_bounds(exclude_pid: Option<u32>) -> Option<WindowBounds> {
    let active = get_active_window().ok()?;
    if exclude_pid.is_some_and(|pid| active.process_id as u32 == pid) {
        return None;
    }

    let bundle_id = bundle_id_from_process_path(&active.process_path);
    if active.position.width >= PLAYBACK_MIN_SIZE && active.position.height >= PLAYBACK_MIN_SIZE {
        return Some(WindowBounds {
            x: active.position.x,
            y: active.position.y,
            width: active.position.width,
            height: active.position.height,
            process_id: active.process_id,
            bundle_id,
        });
    }

    // active-win 尺寸无效时，按 PID 从窗口列表取最大窗口
    largest_window_for_pid(active.process_id, bundle_id)
}

/// 回放时解析目标窗口。
///
/// 调用方应先 `activate_bundle_id`。激活成功后**信任前台非自身窗口**，
/// 不再强制 Bundle ID 字符串一致（JetBrains 应用路径解析的 bundle 常对不上）。
///
/// 若 MeoTool 仍占前台（编辑器试运行常见），回退到窗口列表中最前的非自身窗口。
pub fn resolve_playback_bounds(
    exclude_pid: Option<u32>,
    preferred_bundle_id: Option<&str>,
) -> Option<WindowBounds> {
    // 1) 激活后的前台窗口（最常见成功路径）
    if let Some(bounds) = frontmost_window_bounds(exclude_pid) {
        if let Some(bundle) = preferred_bundle_id.filter(|s| !s.is_empty()) {
            if bounds.bundle_id.as_deref() == Some(bundle) {
                return Some(bounds);
            }
            // Bundle 对不上也接受：open -b 已激活目标应用
            log::info!(
                "playback using frontmost window pid={} bundle={:?} (recorded={bundle})",
                bounds.process_id,
                bounds.bundle_id
            );
            return Some(bounds);
        }
        return Some(bounds);
    }

    // 2) MeoTool 仍在前台：取 z-order 上第一个非自身、足够大的窗口
    //    （试运行常在此路径；「点击」本身会把焦点交给目标应用）
    if let Some(bounds) = topmost_usable_window_bounds(exclude_pid) {
        log::info!(
            "playback using topmost non-self window pid={} bundle={:?}",
            bounds.process_id,
            bounds.bundle_id
        );
        return Some(bounds);
    }

    // 3) System Events / 进程名（部分应用可用）
    if let Some(bundle) = preferred_bundle_id.filter(|s| !s.is_empty()) {
        if let Some(bounds) = bounds_for_bundle_id(bundle) {
            return Some(bounds);
        }
        if let Some(bounds) = bounds_for_process_name_guess(bundle) {
            return Some(bounds);
        }
    }

    None
}

/// z-order（前→后）上第一个非排除 PID、尺寸可用的窗口。
pub fn topmost_usable_window_bounds(exclude_pid: Option<u32>) -> Option<WindowBounds> {
    let windows = list_on_screen_windows();
    let hit = windows.into_iter().find(|w| {
        exclude_pid.map_or(true, |pid| w.process_id as u32 != pid)
            && w.width >= PLAYBACK_MIN_SIZE
            && w.height >= PLAYBACK_MIN_SIZE
    })?;
    let path = process_path_from_pid(hit.process_id);
    let bundle_id = path.as_deref().and_then(bundle_id_from_process_path);
    Some(WindowBounds {
        x: hit.x,
        y: hit.y,
        width: hit.width,
        height: hit.height,
        process_id: hit.process_id,
        bundle_id,
    })
}

/// 将屏幕坐标解析为宏目标：优先窗口归一化，失败则绝对坐标。
#[cfg(any(feature = "ui-egui", feature = "ui-native", test))]
pub fn resolve_click_target(
    screen_x: f64,
    screen_y: f64,
    exclude_pid: Option<u32>,
) -> (f32, f32, CoordSpace, Option<WindowBounds>) {
    if let Some(bounds) = frontmost_window_bounds(exclude_pid) {
        if let Some((nx, ny)) = bounds.screen_to_normalized(screen_x, screen_y) {
            return (nx, ny, CoordSpace::WindowNormalized, Some(bounds));
        }
    }
    (
        screen_x as f32,
        screen_y as f32,
        CoordSpace::ScreenAbs,
        None,
    )
}

pub fn target_to_screen(
    x: f32,
    y: f32,
    coord_space: CoordSpace,
    exclude_pid: Option<u32>,
    preferred_bundle_id: Option<&str>,
) -> Result<(i32, i32), String> {
    match coord_space {
        CoordSpace::ScreenAbs => Ok((x.round() as i32, y.round() as i32)),
        CoordSpace::WindowNormalized => {
            let bounds =
                resolve_playback_bounds(exclude_pid, preferred_bundle_id).ok_or_else(|| {
                    if let Some(bundle) = preferred_bundle_id {
                        format!("回放失败：找不到应用窗口（{bundle}）。请确认该应用已打开且可见")
                    } else {
                        "回放失败：无法获取目标窗口位置（焦点可能仍在 MeoTool，请先切换到目标应用）"
                            .into()
                    }
                })?;
            let (sx, sy) = bounds.normalized_to_screen(x, y);
            Ok((sx.round() as i32, sy.round() as i32))
        }
    }
}

/// 按 Bundle ID 激活应用（回放前调用）。
pub fn activate_bundle_id(bundle_id: &str) -> Result<(), String> {
    if bundle_id.is_empty() {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("open")
            .args(["-b", bundle_id])
            .output()
            .map_err(|e| format!("activate bundle failed: {e}"))?;
        if output.status.success() {
            return Ok(());
        }
        let script = format!("tell application id \"{bundle_id}\" to activate");
        let output = Command::new("osascript")
            .args(["-e", &script])
            .output()
            .map_err(|e| format!("osascript activate failed: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "无法激活 {bundle_id}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = bundle_id;
        Err("activate_bundle_id only supported on macOS".into())
    }
}

/// 屏幕坐标命中最前窗口（含本进程）。调用方自行判断是否取消点选。
pub fn window_at_screen_point(x: f64, y: f64) -> Option<HitWindow> {
    let listed = list_on_screen_windows();
    let hit = first_window_at_point(&listed, x, y)?;
    let path = process_path_from_pid(hit.process_id);
    let bundle_id = path.as_deref().and_then(bundle_id_from_process_path);
    Some(HitWindow {
        process_id: hit.process_id,
        app_name: hit.app_name.clone(),
        bundle_id,
    })
}

/// 前→后第一个包含该点且尺寸足够的窗口。
pub fn first_window_at_point(windows: &[ListedWindow], x: f64, y: f64) -> Option<&ListedWindow> {
    windows
        .iter()
        .find(|w| w.width >= HIT_MIN_SIZE && w.height >= HIT_MIN_SIZE && w.contains(x, y))
}

#[cfg(target_os = "macos")]
fn largest_window_for_pid(pid: u64, bundle_id: Option<String>) -> Option<WindowBounds> {
    let windows = list_on_screen_windows();
    let best = windows
        .into_iter()
        .filter(|w| {
            w.process_id == pid && w.width >= PLAYBACK_MIN_SIZE && w.height >= PLAYBACK_MIN_SIZE
        })
        .max_by(|a, b| {
            (a.width * a.height)
                .partial_cmp(&(b.width * b.height))
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
    Some(WindowBounds {
        x: best.x,
        y: best.y,
        width: best.width,
        height: best.height,
        process_id: best.process_id,
        bundle_id,
    })
}

#[cfg(not(target_os = "macos"))]
fn largest_window_for_pid(_pid: u64, _bundle_id: Option<String>) -> Option<WindowBounds> {
    None
}

fn list_on_screen_windows() -> Vec<ListedWindow> {
    #[cfg(target_os = "macos")]
    {
        macos_cg::list_on_screen_windows()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
mod macos_cg {
    use super::ListedWindow;
    use core_foundation::base::{CFGetTypeID, CFRelease, CFTypeID, CFTypeRef, TCFType, ToVoid};
    use core_foundation::boolean::CFBooleanGetTypeID;
    use core_foundation::dictionary::{
        CFDictionaryGetTypeID, CFDictionaryGetValueIfPresent, CFDictionaryRef,
    };
    use core_foundation::number::{
        CFNumberGetType, CFNumberGetTypeID, CFNumberGetValue, CFNumberRef, CFNumberType,
    };
    use core_foundation::string::{CFString, CFStringGetTypeID, CFStringRef};
    use core_graphics::display::{
        kCGNullWindowID, kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly,
        CFArrayGetCount, CFArrayGetValueAtIndex, CGWindowListCopyWindowInfo,
    };
    use core_graphics::geometry::CGRect;
    use std::ffi::c_void;

    // Apple CFNumberType: kCFNumberSInt32Type / kCFNumberSInt64Type
    const CF_NUMBER_SINT32_TYPE: CFNumberType = 3;
    const CF_NUMBER_SINT64_TYPE: CFNumberType = 4;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGRectMakeWithDictionaryRepresentation(dict: CFDictionaryRef, rect: *mut CGRect) -> u8;
    }

    enum DictVal {
        Number(i64),
        Text(String),
        Rect { x: f64, y: f64, w: f64, h: f64 },
        Unknown,
    }

    fn get_from_dict(dict: CFDictionaryRef, key: &str) -> DictVal {
        let cf_key: CFString = key.into();
        let mut value: *const c_void = std::ptr::null();
        if unsafe { CFDictionaryGetValueIfPresent(dict, cf_key.to_void(), &mut value) } == 0 {
            return DictVal::Unknown;
        }
        let type_id: CFTypeID = unsafe { CFGetTypeID(value) };
        if type_id == unsafe { CFNumberGetTypeID() } {
            let value = value as CFNumberRef;
            match unsafe { CFNumberGetType(value) } {
                CF_NUMBER_SINT64_TYPE => {
                    let mut v = 0_i64;
                    if unsafe {
                        CFNumberGetValue(value, CF_NUMBER_SINT64_TYPE, (&mut v as *mut i64).cast())
                    } {
                        return DictVal::Number(v);
                    }
                }
                CF_NUMBER_SINT32_TYPE => {
                    let mut v = 0_i32;
                    if unsafe {
                        CFNumberGetValue(value, CF_NUMBER_SINT32_TYPE, (&mut v as *mut i32).cast())
                    } {
                        return DictVal::Number(v as i64);
                    }
                }
                _ => {
                    let mut v = 0_i64;
                    if unsafe {
                        CFNumberGetValue(value, CF_NUMBER_SINT64_TYPE, (&mut v as *mut i64).cast())
                    } {
                        return DictVal::Number(v);
                    }
                }
            }
        } else if type_id == unsafe { CFDictionaryGetTypeID() } && key == "kCGWindowBounds" {
            unsafe {
                let mut rect = std::mem::zeroed();
                if CGRectMakeWithDictionaryRepresentation(value.cast(), &mut rect) != 0 {
                    return DictVal::Rect {
                        x: rect.origin.x,
                        y: rect.origin.y,
                        w: rect.size.width,
                        h: rect.size.height,
                    };
                }
            }
        } else if type_id == unsafe { CFStringGetTypeID() } {
            let s = unsafe { CFString::wrap_under_get_rule(value as CFStringRef) };
            return DictVal::Text(s.to_string());
        } else if type_id == unsafe { CFBooleanGetTypeID() } {
            // ignore
        }
        DictVal::Unknown
    }

    pub fn list_on_screen_windows() -> Vec<ListedWindow> {
        let mut out = Vec::new();
        unsafe {
            let info = CGWindowListCopyWindowInfo(
                kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
                kCGNullWindowID,
            );
            if info.is_null() {
                return out;
            }
            let count = CFArrayGetCount(info);
            for i in 0..count {
                let dict = CFArrayGetValueAtIndex(info, i) as CFDictionaryRef;
                if dict.is_null() {
                    continue;
                }
                let DictVal::Number(win_pid) = get_from_dict(dict, "kCGWindowOwnerPID") else {
                    continue;
                };
                if win_pid <= 0 {
                    continue;
                }
                let DictVal::Rect { x, y, w, h } = get_from_dict(dict, "kCGWindowBounds") else {
                    continue;
                };
                let app_name = match get_from_dict(dict, "kCGWindowOwnerName") {
                    DictVal::Text(name) => name,
                    _ => String::new(),
                };
                out.push(ListedWindow {
                    process_id: win_pid as u64,
                    app_name,
                    x,
                    y,
                    width: w,
                    height: h,
                });
            }
            CFRelease(info as CFTypeRef);
        }
        out
    }
}

/// System Events 按 bundle identifier（对 JetBrains 常失败）。
#[cfg(target_os = "macos")]
fn bounds_for_bundle_id(bundle_id: &str) -> Option<WindowBounds> {
    let script = format!(
        r#"tell application "System Events"
  try
    if not (exists process whose bundle identifier is "{bundle_id}") then return ""
    tell (first process whose bundle identifier is "{bundle_id}")
      if (count of windows) is 0 then return ""
      set p to position of front window
      set s to size of front window
      set unixPid to unix id
      return ((item 1 of p) as text) & "," & ((item 2 of p) as text) & "," & ((item 1 of s) as text) & "," & ((item 2 of s) as text) & "," & (unixPid as text)
    end tell
  on error
    return ""
  end try
end tell"#
    );
    parse_bounds_script(&script, Some(bundle_id))
}

#[cfg(not(target_os = "macos"))]
fn bounds_for_bundle_id(_bundle_id: &str) -> Option<WindowBounds> {
    None
}

/// 按常见进程名再试一次（Android Studio / IntelliJ）。
#[cfg(target_os = "macos")]
fn bounds_for_process_name_guess(bundle_id: &str) -> Option<WindowBounds> {
    let names: &[&str] = match bundle_id {
        "com.google.android.studio" => &["Android Studio", "studio"],
        id if id.contains("intellij") => &["IntelliJ IDEA"],
        id if id.contains("WebStorm") || id.contains("webstorm") => &["WebStorm"],
        _ => return None,
    };
    for name in names {
        let script = format!(
            r#"tell application "System Events"
  try
    if not (exists process "{name}") then return ""
    tell process "{name}"
      if (count of windows) is 0 then return ""
      set p to position of front window
      set s to size of front window
      set unixPid to unix id
      return ((item 1 of p) as text) & "," & ((item 2 of p) as text) & "," & ((item 1 of s) as text) & "," & ((item 2 of s) as text) & "," & (unixPid as text)
    end tell
  on error
    return ""
  end try
end tell"#
        );
        if let Some(b) = parse_bounds_script(&script, Some(bundle_id)) {
            return Some(b);
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn bounds_for_process_name_guess(_bundle_id: &str) -> Option<WindowBounds> {
    None
}

#[cfg(target_os = "macos")]
fn parse_bounds_script(script: &str, bundle_id: Option<&str>) -> Option<WindowBounds> {
    let output = Command::new("osascript")
        .args(["-e", script])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let parts: Vec<&str> = text.split(',').map(str::trim).collect();
    if parts.len() < 4 {
        return None;
    }
    let x: f64 = parts[0].parse().ok()?;
    let y: f64 = parts[1].parse().ok()?;
    let width: f64 = parts[2].parse().ok()?;
    let height: f64 = parts[3].parse().ok()?;
    if width < PLAYBACK_MIN_SIZE || height < PLAYBACK_MIN_SIZE {
        return None;
    }
    let process_id = parts
        .get(4)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    Some(WindowBounds {
        x,
        y,
        width,
        height,
        process_id,
        bundle_id: bundle_id.map(|s| s.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_roundtrip() {
        let bounds = WindowBounds {
            x: 100.0,
            y: 200.0,
            width: 400.0,
            height: 300.0,
            process_id: 1,
            bundle_id: None,
        };
        let (nx, ny) = bounds.screen_to_normalized(300.0, 350.0).unwrap();
        assert!((nx - 0.5).abs() < 1e-5);
        assert!((ny - 0.5).abs() < 1e-5);
        let (sx, sy) = bounds.normalized_to_screen(nx, ny);
        assert!((sx - 300.0).abs() < 1e-5);
        assert!((sy - 350.0).abs() < 1e-5);
    }

    fn listed(pid: u64, name: &str, x: f64, y: f64, w: f64, h: f64) -> ListedWindow {
        ListedWindow {
            process_id: pid,
            app_name: name.into(),
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn first_window_at_point_front_to_back() {
        let windows = vec![
            listed(1, "overlay", 0.0, 0.0, 100.0, 100.0),
            listed(2, "behind", 0.0, 0.0, 400.0, 400.0),
        ];
        let hit = first_window_at_point(&windows, 50.0, 50.0).unwrap();
        assert_eq!(hit.process_id, 1);
        assert_eq!(hit.app_name, "overlay");
    }

    #[test]
    fn first_window_at_point_skips_tiny_and_misses() {
        let windows = vec![
            listed(1, "hairline", 0.0, 0.0, 4.0, 100.0),
            listed(2, "app", 10.0, 10.0, 200.0, 200.0),
        ];
        let hit = first_window_at_point(&windows, 50.0, 50.0).unwrap();
        assert_eq!(hit.process_id, 2);
        assert!(first_window_at_point(&windows, 500.0, 500.0).is_none());
    }

    #[test]
    fn own_process_is_frontmost_hit() {
        let windows = vec![
            listed(99, "meotool", 0.0, 0.0, 200.0, 200.0),
            listed(7, "Safari", 0.0, 0.0, 800.0, 600.0),
        ];
        let hit = first_window_at_point(&windows, 20.0, 20.0).unwrap();
        assert_eq!(hit.process_id, 99);
    }
}
