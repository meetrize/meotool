use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use active_win_pos_rs::get_active_window;

use crate::config::WindowInfo;
use crate::platform::{
    bundle_id_from_process_path, enrich_editor_window_title, fetch_browser_page_url,
};

/// 面板隐藏时后台轮询间隔下限（毫秒）
const HIDDEN_POLL_MIN_MS: u64 = 800;
/// 面板隐藏时间隔 = 可见间隔 × 此倍率（不低于下限）
const HIDDEN_POLL_FACTOR: u64 = 4;

/// 前台窗口 watcher 的轮询节奏（可见 / 隐藏 / 折叠空闲不同间隔）。
pub struct WindowWatcherControl {
    panel_visible: AtomicBool,
    /// 折叠收起且无交互：与隐藏同档降频，降低 osascript 压力
    relaxed: AtomicBool,
    visible_interval_ms: u64,
    hidden_interval_ms: u64,
}

impl WindowWatcherControl {
    pub fn new(visible_interval_ms: u64) -> Arc<Self> {
        let visible_interval_ms = visible_interval_ms.max(50);
        let hidden_interval_ms = visible_interval_ms
            .saturating_mul(HIDDEN_POLL_FACTOR)
            .max(HIDDEN_POLL_MIN_MS);
        Arc::new(Self {
            panel_visible: AtomicBool::new(false),
            relaxed: AtomicBool::new(false),
            visible_interval_ms,
            hidden_interval_ms,
        })
    }

    pub fn set_panel_visible(&self, visible: bool) {
        self.panel_visible.store(visible, Ordering::Relaxed);
    }

    pub fn set_relaxed(&self, relaxed: bool) {
        self.relaxed.store(relaxed, Ordering::Relaxed);
    }

    pub fn sleep_interval(&self) -> Duration {
        let visible = self.panel_visible.load(Ordering::Relaxed);
        let relaxed = self.relaxed.load(Ordering::Relaxed);
        let ms = if visible && !relaxed {
            self.visible_interval_ms
        } else {
            self.hidden_interval_ms
        };
        Duration::from_millis(ms)
    }

    #[cfg(test)]
    pub fn hidden_interval_ms(&self) -> u64 {
        self.hidden_interval_ms
    }
}

pub fn fetch_active_window() -> Option<WindowInfo> {
    let active = get_active_window().ok()?;
    let process_path = active.process_path.clone();
    let bundle_id = bundle_id_from_process_path(&process_path);

    let mut window = WindowInfo {
        app_name: active.app_name,
        bundle_id,
        title: active.title,
        process_id: active.process_id,
        process_path,
        page_url: None,
    };
    // Safari URL / Cursor 标题经 platform 侧限频缓存，避免每 200ms 起 osascript
    window.page_url = fetch_browser_page_url(&window);
    enrich_editor_window_title(&mut window);
    Some(window)
}

pub fn spawn_watcher(
    tx: std::sync::mpsc::Sender<WindowInfo>,
    control: Arc<WindowWatcherControl>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut last_key = String::new();
        let mut last_page_url: Option<String> = None;
        let mut last_tracked_window: Option<WindowInfo> = None;
        loop {
            if let Some(window) = fetch_active_window() {
                if window.is_own_process() {
                    if let Some(mut tracked) = last_tracked_window.clone() {
                        let mut changed = false;
                        let previous_title = tracked.title.clone();
                        enrich_editor_window_title(&mut tracked);
                        if tracked.title != previous_title {
                            changed = true;
                        }
                        let refreshed_url = fetch_browser_page_url(&tracked);
                        if refreshed_url != tracked.page_url {
                            tracked.page_url = refreshed_url;
                            changed = true;
                        }
                        if changed {
                            last_tracked_window = Some(tracked.clone());
                            last_key = tracked.identity_key();
                            last_page_url = tracked.page_url.clone();
                            if tx.send(tracked).is_err() {
                                break;
                            }
                        }
                    }
                    std::thread::sleep(control.sleep_interval());
                    continue;
                }
                let key = window.identity_key();
                let page_url_changed = window.page_url != last_page_url;
                if key != last_key || page_url_changed {
                    last_key = key;
                    last_page_url = window.page_url.clone();
                    last_tracked_window = Some(window.clone());
                    if tx.send(window).is_err() {
                        break;
                    }
                } else {
                    last_tracked_window = Some(window);
                }
            }
            std::thread::sleep(control.sleep_interval());
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_does_not_panic() {
        let _ = fetch_active_window();
    }

    #[test]
    fn hidden_poll_slower_than_visible() {
        let ctl = WindowWatcherControl::new(200);
        assert_eq!(ctl.hidden_interval_ms(), 800);
        ctl.set_panel_visible(true);
        assert_eq!(ctl.sleep_interval(), Duration::from_millis(200));
        ctl.set_panel_visible(false);
        assert_eq!(ctl.sleep_interval(), Duration::from_millis(800));
        ctl.set_panel_visible(true);
        ctl.set_relaxed(true);
        assert_eq!(ctl.sleep_interval(), Duration::from_millis(800));
        ctl.set_relaxed(false);
        assert_eq!(ctl.sleep_interval(), Duration::from_millis(200));
    }
}
