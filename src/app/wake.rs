//! 跨线程唤醒主循环（替代 `egui::Context::request_repaint`）。
//!
//! 阶段 0：回调由 egui 入口注入；阶段 1+ 可换成 CFRunLoopSource / AppKit。

use std::sync::{Arc, Mutex};

type WakeCallback = Arc<dyn Fn() + Send + Sync + 'static>;

/// 可克隆的唤醒句柄；后台线程（热键 / 手势 / tray / 宏）只依赖本类型。
#[derive(Clone, Default)]
pub struct WakeHandle {
    inner: Arc<WakeInner>,
}

#[derive(Default)]
struct WakeInner {
    callback: Mutex<Option<WakeCallback>>,
}

impl WakeHandle {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注入主线程唤醒逻辑（例如 `ctx.request_repaint()`）。
    pub fn set_callback<F>(&self, f: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        if let Ok(mut slot) = self.inner.callback.lock() {
            *slot = Some(Arc::new(f));
        }
    }

    /// 清空回调（例如 UI 销毁时）。
    #[allow(dead_code)]
    pub fn clear_callback(&self) {
        if let Ok(mut slot) = self.inner.callback.lock() {
            *slot = None;
        }
    }

    /// 请求主循环尽快处理事件。
    pub fn poke(&self) {
        let cb = self
            .inner
            .callback
            .lock()
            .ok()
            .and_then(|g| g.as_ref().cloned());
        if let Some(cb) = cb {
            cb();
        }
    }
}
