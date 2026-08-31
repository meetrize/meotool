use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crate::app::WakeHandle;
use crate::config::GestureConfig;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MouseGestureAction {
    ShowAt { x: f32, y: f32 },
    LeftMouseDown { x: f32, y: f32 },
}

pub struct MouseGestureMonitor {
    #[cfg(target_os = "macos")]
    shared: Arc<TapSharedState>,
    #[cfg(target_os = "macos")]
    receiver: std::sync::mpsc::Receiver<RawMouseEvent>,
    #[cfg(target_os = "macos")]
    sender: std::sync::mpsc::Sender<RawMouseEvent>,
    #[cfg(target_os = "macos")]
    tap_started: AtomicBool,
    dismiss_armed: bool,
}

#[cfg(target_os = "macos")]
enum RawMouseEvent {
    GestureTriggered { x: f64, y: f64 },
    LeftDown { x: f64, y: f64 },
}

#[cfg(target_os = "macos")]
struct TapSharedState {
    enabled: AtomicBool,
    global_priority: AtomicBool,
    drag_threshold_bits: AtomicU32,
    dismiss_armed: AtomicBool,
    ignore_rect: RwLock<Option<[f32; 4]>>,
    pending_right: Mutex<Option<(f64, f64)>>,
    suppress_right: AtomicBool,
    gesture_sent: AtomicBool,
    wake: WakeHandle,
}

#[cfg(target_os = "macos")]
impl TapSharedState {
    fn new(config: &GestureConfig, wake: WakeHandle) -> Self {
        Self {
            enabled: AtomicBool::new(config.enabled),
            global_priority: AtomicBool::new(config.global_priority),
            drag_threshold_bits: AtomicU32::new(config.drag_threshold_px.to_bits()),
            dismiss_armed: AtomicBool::new(false),
            ignore_rect: RwLock::new(None),
            pending_right: Mutex::new(None),
            suppress_right: AtomicBool::new(false),
            gesture_sent: AtomicBool::new(false),
            wake,
        }
    }

    fn apply_config(&self, config: &GestureConfig) {
        self.enabled.store(config.enabled, Ordering::Relaxed);
        self.global_priority
            .store(config.global_priority, Ordering::Relaxed);
        self.drag_threshold_bits
            .store(config.drag_threshold_px.to_bits(), Ordering::Relaxed);
    }

    fn drag_threshold(&self) -> f64 {
        f32::from_bits(self.drag_threshold_bits.load(Ordering::Relaxed)) as f64
    }

    fn wake_ui(&self) {
        self.wake.poke();
    }
}

impl MouseGestureMonitor {
    /// 仅创建状态；CGEventTap 在手势启用后懒启动，降低冷启动常驻。
    pub fn new(config: &GestureConfig, wake: WakeHandle) -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        {
            let shared = Arc::new(TapSharedState::new(config, wake));
            let (tx, rx) = std::sync::mpsc::channel();
            Ok(Self {
                shared,
                receiver: rx,
                sender: tx,
                tap_started: AtomicBool::new(false),
                dismiss_armed: false,
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (config, wake);
            Ok(Self {
                dismiss_armed: false,
            })
        }
    }

    pub fn update_config(&mut self, config: &GestureConfig) {
        #[cfg(target_os = "macos")]
        {
            self.shared.apply_config(config);
            if config.enabled {
                if let Err(err) = self.ensure_tap() {
                    log::warn!("mouse gesture tap start failed: {err}");
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = config;
        }
    }

    #[cfg(target_os = "macos")]
    fn ensure_tap(&self) -> Result<(), String> {
        if self.tap_started.load(Ordering::Acquire) {
            return Ok(());
        }
        if self
            .tap_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(());
        }
        match macos_impl::spawn_event_tap(self.shared.clone(), self.sender.clone()) {
            Ok(()) => {
                log::info!("mouse gesture event tap lazily started");
                Ok(())
            }
            Err(err) => {
                self.tap_started.store(false, Ordering::Release);
                Err(err)
            }
        }
    }

    pub fn set_dismiss_armed(&mut self, armed: bool) {
        self.dismiss_armed = armed;
        #[cfg(target_os = "macos")]
        self.shared.dismiss_armed.store(armed, Ordering::Relaxed);
    }

    pub fn set_ignore_rect(&mut self, rect: Option<[f32; 4]>) {
        #[cfg(target_os = "macos")]
        {
            if let Ok(mut slot) = self.shared.ignore_rect.write() {
                *slot = rect;
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = rect;
        }
    }

    pub fn poll(&mut self) -> Vec<MouseGestureAction> {
        #[cfg(target_os = "macos")]
        {
            let mut actions = Vec::new();
            while let Ok(event) = self.receiver.try_recv() {
                match event {
                    RawMouseEvent::GestureTriggered { x, y } => {
                        let pos = macos_impl::event_to_viewport(x, y);
                        actions.push(MouseGestureAction::ShowAt { x: pos.0, y: pos.1 });
                    }
                    RawMouseEvent::LeftDown { x, y } => {
                        if !self.dismiss_armed {
                            continue;
                        }
                        let pos = macos_impl::event_to_viewport(x, y);
                        actions.push(MouseGestureAction::LeftMouseDown { x: pos.0, y: pos.1 });
                    }
                }
            }
            actions
        }
        #[cfg(not(target_os = "macos"))]
        {
            Vec::new()
        }
    }
}

#[cfg(target_os = "macos")]
mod macos_impl {
    use super::{RawMouseEvent, TapSharedState};
    use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
    use core_graphics::event::{
        CGEvent, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
        CGEventType,
    };
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::Sender;
    use std::sync::Arc;
    use std::thread;

    pub fn spawn_event_tap(
        shared: Arc<TapSharedState>,
        tx: Sender<RawMouseEvent>,
    ) -> Result<(), String> {
        thread::Builder::new()
            .name("meotool-mouse-tap".into())
            .spawn(move || {
                if let Err(err) = run_event_tap_loop(shared, tx) {
                    log::error!("mouse gesture tap failed: {err}");
                }
            })
            .map_err(|e| format!("spawn mouse tap thread: {e}"))?;

        Ok(())
    }

    fn run_event_tap_loop(
        shared: Arc<TapSharedState>,
        tx: Sender<RawMouseEvent>,
    ) -> Result<(), String> {
        let events = vec![
            CGEventType::RightMouseDown,
            CGEventType::RightMouseDragged,
            CGEventType::RightMouseUp,
            CGEventType::LeftMouseDown,
        ];

        let shared_cb = shared.clone();
        let tx_cb = tx.clone();
        let tap = CGEventTap::new(
            CGEventTapLocation::HID,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            events,
            move |_proxy, event_type, event| {
                handle_tap_event(&shared_cb, &tx_cb, event_type, event)
            },
        )
        .map_err(|_| "CGEventTapCreate failed (need Input Monitoring permission)".to_string())?;

        let loop_source = tap
            .mach_port
            .create_runloop_source(0)
            .map_err(|_| "create_runloop_source failed".to_string())?;

        let run_loop = CFRunLoop::get_current();
        unsafe {
            run_loop.add_source(&loop_source, kCFRunLoopCommonModes);
        }
        tap.enable();
        log::info!("mouse gesture event tap started (global priority capable)");
        CFRunLoop::run_current();

        Ok(())
    }

    fn handle_tap_event(
        shared: &TapSharedState,
        tx: &Sender<RawMouseEvent>,
        event_type: CGEventType,
        event: &CGEvent,
    ) -> Option<CGEvent> {
        if !shared.enabled.load(Ordering::Relaxed) {
            return Some(event.clone());
        }

        let global_priority = shared.global_priority.load(Ordering::Relaxed);
        let location = event.location();
        let x = location.x;
        let y = location.y;

        let consume = |event: &CGEvent| -> Option<CGEvent> {
            if global_priority {
                None
            } else {
                Some(event.clone())
            }
        };

        match event_type {
            CGEventType::RightMouseDown => {
                if point_in_ignore_rect(shared, x, y) {
                    return Some(event.clone());
                }
                if let Ok(mut pending) = shared.pending_right.lock() {
                    *pending = Some((x, y));
                }
                shared.gesture_sent.store(false, Ordering::Relaxed);
                shared.wake_ui();
                Some(event.clone())
            }
            CGEventType::RightMouseDragged => {
                let mut triggered = false;
                if let Ok(pending) = shared.pending_right.lock() {
                    if let Some((ox, oy)) = *pending {
                        let dx = x - ox;
                        let dy = y - oy;
                        let threshold = shared.drag_threshold();
                        if dx * dx + dy * dy >= threshold * threshold
                            && !shared.gesture_sent.load(Ordering::Relaxed)
                        {
                            shared.gesture_sent.store(true, Ordering::Relaxed);
                            if global_priority {
                                shared.suppress_right.store(true, Ordering::Relaxed);
                            }
                            let _ = tx.send(RawMouseEvent::GestureTriggered { x: ox, y: oy });
                            shared.wake_ui();
                            triggered = true;
                        }
                    }
                }
                if triggered || shared.suppress_right.load(Ordering::Relaxed) {
                    return consume(event);
                }
                Some(event.clone())
            }
            CGEventType::RightMouseUp => {
                if let Ok(mut pending) = shared.pending_right.lock() {
                    *pending = None;
                }
                let was_suppressing = shared.suppress_right.swap(false, Ordering::Relaxed);
                if was_suppressing {
                    return consume(event);
                }
                Some(event.clone())
            }
            CGEventType::LeftMouseDown => {
                if shared.dismiss_armed.load(Ordering::Relaxed) {
                    let _ = tx.send(RawMouseEvent::LeftDown { x, y });
                    shared.wake_ui();
                }
                Some(event.clone())
            }
            _ => Some(event.clone()),
        }
    }

    fn point_in_ignore_rect(shared: &TapSharedState, x: f64, y: f64) -> bool {
        let Ok(slot) = shared.ignore_rect.read() else {
            return false;
        };
        let Some([min_x, min_y, max_x, max_y]) = *slot else {
            return false;
        };
        let px = x as f32;
        let py = y as f32;
        px >= min_x && px <= max_x && py >= min_y && py <= max_y
    }

    pub fn event_to_viewport(x: f64, y: f64) -> (f32, f32) {
        (x as f32, y as f32)
    }
}
