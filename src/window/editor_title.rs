use crate::config::WindowInfo;

const CURSOR_BUNDLE_ID: &str = "com.todesktop.230313mzl4w4u92";

pub fn is_cursor_window(window: &WindowInfo) -> bool {
    window.app_name.eq_ignore_ascii_case("Cursor")
        || window
            .bundle_id
            .as_deref()
            .is_some_and(|id| id == CURSOR_BUNDLE_ID)
}

/// Cursor 激活时，顶栏悬停提示显示完整窗口标题
pub fn header_app_tooltip(window: &WindowInfo) -> Option<String> {
    if is_cursor_window(window) && !window.title.is_empty() {
        Some(window.title.clone())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::WindowInfo;

    fn cursor_window(title: &str) -> WindowInfo {
        WindowInfo {
            app_name: "Cursor".into(),
            bundle_id: Some(CURSOR_BUNDLE_ID.into()),
            title: title.into(),
            process_id: 1,
            process_path: PathBuf::from("/Applications/Cursor.app"),
            page_url: None,
        }
    }

    #[test]
    fn non_cursor_window_has_no_header_tooltip() {
        let window = WindowInfo {
            app_name: "Safari".into(),
            bundle_id: Some("com.apple.Safari".into()),
            title: "Apple".into(),
            process_id: 1,
            process_path: PathBuf::from("/Applications/Safari.app"),
            page_url: None,
        };
        assert_eq!(header_app_tooltip(&window), None);
    }

    #[test]
    fn cursor_tooltip_shows_full_window_title() {
        let window = cursor_window("main.rs — meotool — Cursor");
        assert_eq!(window.display_name(), "Cursor");
        assert_eq!(
            header_app_tooltip(&window).as_deref(),
            Some("main.rs — meotool — Cursor")
        );
    }
}
