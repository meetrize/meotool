//! 点选屏幕上的应用窗口：拦截下一次左键，解析 Bundle ID / 应用名。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::app::WakeHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickOutcome {
    Picked {
        bundle_id: Option<String>,
        app_name: String,
    },
    Cancelled,
}

struct Shared {
    picking: AtomicBool,
    swallow_up: AtomicBool,
    outcome: Mutex<Option<PickOutcome>>,
    wake: WakeHandle,
}

pub struct AppClickPicker {
    #[cfg(target_os = "macos")]
    shared: Arc<Shared>,
    #[cfg(target_os = "macos")]
    tap_started: AtomicBool,
}

impl AppClickPicker {
    pub fn new(wake: WakeHandle) -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        {
            Ok(Self {
                shared: Arc::new(Shared {
                    picking: AtomicBool::new(false),
                    swallow_up: AtomicBool::new(false),
                    outcome: Mutex::new(None),
                    wake,
                }),
                tap_started: AtomicBool::new(false),
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = wake;
            Err("app click picker only supported on macOS".into())
        }
    }

    pub fn is_picking(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.shared.picking.load(Ordering::Relaxed)
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }

    pub fn start(&self) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            self.ensure_tap()?;
            if let Ok(mut out) = self.shared.outcome.lock() {
                *out = None;
            }
            self.shared.swallow_up.store(false, Ordering::Relaxed);
            self.shared.picking.store(true, Ordering::Relaxed);
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err("app click picker only supported on macOS".into())
        }
    }

    pub fn cancel(&self) {
        #[cfg(target_os = "macos")]
        {
            if !self.shared.picking.swap(false, Ordering::Relaxed) {
                return;
            }
            self.shared.swallow_up.store(false, Ordering::Relaxed);
            if let Ok(mut out) = self.shared.outcome.lock() {
                *out = Some(PickOutcome::Cancelled);
            }
            self.shared.wake.poke();
        }
    }

    pub fn poll(&self) -> Option<PickOutcome> {
        #[cfg(target_os = "macos")]
        {
            self.shared.outcome.lock().ok().and_then(|mut g| g.take())
        }
        #[cfg(not(target_os = "macos"))]
        {
            None
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
        match macos_impl::spawn_event_tap(self.shared.clone()) {
            Ok(()) => {
                log::info!("app click picker event tap lazily started");
                Ok(())
            }
            Err(err) => {
                self.tap_started.store(false, Ordering::Release);
                Err(err)
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod macos_impl {
    use super::{PickOutcome, Shared};
    use crate::platform::window_bounds::window_at_screen_point;
    use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
    use core_graphics::event::{
        CGEvent, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
        CGEventType, EventField,
    };
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::thread;

    pub fn spawn_event_tap(shared: Arc<Shared>) -> Result<(), String> {
        thread::Builder::new()
            .name("meotool-app-picker-tap".into())
            .spawn(move || {
                if let Err(err) = run_event_tap_loop(shared) {
                    log::error!("app click picker tap failed: {err}");
                }
            })
            .map_err(|e| format!("spawn app picker tap thread: {e}"))?;
        Ok(())
    }

    fn run_event_tap_loop(shared: Arc<Shared>) -> Result<(), String> {
        let events = vec![
            CGEventType::LeftMouseDown,
            CGEventType::LeftMouseUp,
            CGEventType::KeyDown,
        ];
        let shared_cb = shared.clone();
        let tap = CGEventTap::new(
            CGEventTapLocation::HID,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            events,
            move |_proxy, event_type, event| handle_tap_event(&shared_cb, event_type, event),
        )
        .map_err(|_| {
            "CGEventTapCreate failed for app picker (need Input Monitoring)".to_string()
        })?;

        let loop_source = tap
            .mach_port
            .create_runloop_source(0)
            .map_err(|_| "create_runloop_source failed".to_string())?;
        let run_loop = CFRunLoop::get_current();
        unsafe {
            run_loop.add_source(&loop_source, kCFRunLoopCommonModes);
        }
        tap.enable();
        log::info!("app click picker event tap started");
        CFRunLoop::run_current();
        Ok(())
    }

    fn handle_tap_event(
        shared: &Shared,
        event_type: CGEventType,
        event: &CGEvent,
    ) -> Option<CGEvent> {
        if matches!(event_type, CGEventType::LeftMouseUp)
            && shared.swallow_up.swap(false, Ordering::Relaxed)
        {
            return None;
        }

        if !shared.picking.load(Ordering::Relaxed) {
            return Some(event.clone());
        }

        match event_type {
            CGEventType::KeyDown => {
                let keycode = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
                if keycode == 53 {
                    finish(shared, PickOutcome::Cancelled);
                    return None;
                }
                Some(event.clone())
            }
            CGEventType::LeftMouseDown => {
                let location = event.location();
                let own_pid = u64::from(std::process::id());
                match window_at_screen_point(location.x, location.y) {
                    Some(hit) if hit.process_id == own_pid => {
                        finish(shared, PickOutcome::Cancelled);
                        shared.swallow_up.store(true, Ordering::Relaxed);
                        None
                    }
                    Some(hit) => {
                        finish(
                            shared,
                            PickOutcome::Picked {
                                bundle_id: hit.bundle_id,
                                app_name: hit.app_name,
                            },
                        );
                        shared.swallow_up.store(true, Ordering::Relaxed);
                        None
                    }
                    None => None,
                }
            }
            _ => Some(event.clone()),
        }
    }

    fn finish(shared: &Shared, outcome: PickOutcome) {
        shared.picking.store(false, Ordering::Relaxed);
        if let Ok(mut out) = shared.outcome.lock() {
            *out = Some(outcome);
        }
        shared.wake.poke();
    }
}
