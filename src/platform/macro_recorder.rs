//! 鼠标/键盘宏录制：单击捕获 / 序列录制（独立 CGEventTap，懒启动）。
//!
//! 序列录制 API（`start_sequence` / `finish_sequence`）当前主要由 egui 使用。

#![cfg_attr(not(feature = "ui-egui"), allow(dead_code))]

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use crate::action::macro_simplify::{RawMacroEvent, RawMouseKind};
use crate::app::WakeHandle;
use crate::config::Key;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RecordMode {
    Idle = 0,
    CaptureOnce = 1,
    Sequence = 2,
}

impl RecordMode {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::CaptureOnce,
            2 => Self::Sequence,
            _ => Self::Idle,
        }
    }
}

#[derive(Debug, Clone)]
pub enum MacroRecordOutcome {
    CaptureOnce { x: f64, y: f64 },
    Sequence { events: Vec<RawMacroEvent> },
    Cancelled,
}

struct Shared {
    mode: AtomicU8,
    buffer: Mutex<Vec<RawMacroEvent>>,
    outcome: Mutex<Option<MacroRecordOutcome>>,
    wake: WakeHandle,
}

pub struct MacroRecorder {
    #[cfg(target_os = "macos")]
    shared: Arc<Shared>,
    #[cfg(target_os = "macos")]
    tap_started: AtomicBool,
}

impl MacroRecorder {
    /// 仅创建共享状态；CGEventTap 在首次录制时懒启动，降低常驻开销。
    pub fn new(wake: WakeHandle) -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        {
            let shared = Arc::new(Shared {
                mode: AtomicU8::new(RecordMode::Idle as u8),
                buffer: Mutex::new(Vec::new()),
                outcome: Mutex::new(None),
                wake,
            });
            Ok(Self {
                shared,
                tap_started: AtomicBool::new(false),
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = wake;
            Ok(Self {})
        }
    }

    pub fn mode(&self) -> RecordMode {
        #[cfg(target_os = "macos")]
        {
            RecordMode::from_u8(self.shared.mode.load(Ordering::Relaxed))
        }
        #[cfg(not(target_os = "macos"))]
        {
            RecordMode::Idle
        }
    }

    pub fn start_capture_once(&self) {
        if let Err(err) = self.ensure_tap() {
            log::error!("macro tap start failed: {err}");
            return;
        }
        self.begin(RecordMode::CaptureOnce);
    }

    pub fn start_sequence(&self) {
        if let Err(err) = self.ensure_tap() {
            log::error!("macro tap start failed: {err}");
            return;
        }
        self.begin(RecordMode::Sequence);
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
                log::info!("macro recorder event tap lazily started");
                Ok(())
            }
            Err(err) => {
                self.tap_started.store(false, Ordering::Release);
                Err(err)
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn ensure_tap(&self) -> Result<(), String> {
        Ok(())
    }

    fn begin(&self, mode: RecordMode) {
        #[cfg(target_os = "macos")]
        {
            if let Ok(mut buf) = self.shared.buffer.lock() {
                buf.clear();
            }
            if let Ok(mut out) = self.shared.outcome.lock() {
                *out = None;
            }
            self.shared.mode.store(mode as u8, Ordering::Relaxed);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = mode;
        }
    }

    pub fn finish_sequence(&self) {
        #[cfg(target_os = "macos")]
        {
            if self.mode() != RecordMode::Sequence {
                return;
            }
            let events = self
                .shared
                .buffer
                .lock()
                .map(|b| b.clone())
                .unwrap_or_default();
            self.shared
                .mode
                .store(RecordMode::Idle as u8, Ordering::Relaxed);
            if let Ok(mut out) = self.shared.outcome.lock() {
                *out = Some(MacroRecordOutcome::Sequence { events });
            }
            self.wake();
        }
    }

    pub fn cancel(&self) {
        #[cfg(target_os = "macos")]
        {
            self.shared
                .mode
                .store(RecordMode::Idle as u8, Ordering::Relaxed);
            if let Ok(mut buf) = self.shared.buffer.lock() {
                buf.clear();
            }
            if let Ok(mut out) = self.shared.outcome.lock() {
                *out = Some(MacroRecordOutcome::Cancelled);
            }
            self.wake();
        }
    }

    pub fn poll_outcome(&self) -> Option<MacroRecordOutcome> {
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
    fn wake(&self) {
        self.shared.wake.poke();
    }
}

impl Default for MacroRecorder {
    fn default() -> Self {
        Self::new(WakeHandle::new()).unwrap_or_else(|_| Self {
            #[cfg(target_os = "macos")]
            shared: Arc::new(Shared {
                mode: AtomicU8::new(0),
                buffer: Mutex::new(Vec::new()),
                outcome: Mutex::new(None),
                wake: WakeHandle::new(),
            }),
            #[cfg(target_os = "macos")]
            tap_started: AtomicBool::new(false),
        })
    }
}

/// macOS 虚拟键码 → 可回放的主键（字母/数字/常用键）。
pub fn keycode_to_char(keycode: i64) -> Option<char> {
    // https://eastmanreference.com/complete-list-of-applescript-key-codes (US)
    match keycode {
        0 => Some('a'),
        1 => Some('s'),
        2 => Some('d'),
        3 => Some('f'),
        4 => Some('h'),
        5 => Some('g'),
        6 => Some('z'),
        7 => Some('x'),
        8 => Some('c'),
        9 => Some('v'),
        11 => Some('b'),
        12 => Some('q'),
        13 => Some('w'),
        14 => Some('e'),
        15 => Some('r'),
        16 => Some('y'),
        17 => Some('t'),
        18 => Some('1'),
        19 => Some('2'),
        20 => Some('3'),
        21 => Some('4'),
        22 => Some('6'),
        23 => Some('5'),
        24 => Some('='),
        25 => Some('9'),
        26 => Some('7'),
        27 => Some('-'),
        28 => Some('8'),
        29 => Some('0'),
        30 => Some(']'),
        31 => Some('o'),
        32 => Some('u'),
        33 => Some('['),
        34 => Some('i'),
        35 => Some('p'),
        37 => Some('l'),
        38 => Some('j'),
        39 => Some('\''),
        40 => Some('k'),
        41 => Some(';'),
        42 => Some('\\'),
        43 => Some(','),
        44 => Some('/'),
        45 => Some('n'),
        46 => Some('m'),
        47 => Some('.'),
        49 => Some(' '),
        50 => Some('`'),
        _ => None,
    }
}

pub fn flags_to_modifiers(cmd: bool, ctrl: bool, alt: bool, shift: bool) -> Vec<Key> {
    let mut keys = Vec::new();
    if cmd {
        keys.push(Key::Cmd);
    }
    if ctrl {
        keys.push(Key::Ctrl);
    }
    if alt {
        keys.push(Key::Alt);
    }
    if shift {
        keys.push(Key::Shift);
    }
    keys
}

#[cfg(target_os = "macos")]
mod macos_impl {
    use super::{
        flags_to_modifiers, keycode_to_char, MacroRecordOutcome, RawMacroEvent, RawMouseKind,
        RecordMode, Shared,
    };
    use crate::config::Key;
    use crate::platform::window_bounds::frontmost_window_bounds;
    use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
    use core_graphics::event::{
        CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions,
        CGEventTapPlacement, CGEventType, EventField,
    };
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::thread;
    use std::time::Instant;

    pub fn spawn_event_tap(shared: Arc<Shared>) -> Result<(), String> {
        thread::Builder::new()
            .name("meotool-macro-tap".into())
            .spawn(move || {
                if let Err(err) = run_event_tap_loop(shared) {
                    log::error!("macro recorder tap failed: {err}");
                }
            })
            .map_err(|e| format!("spawn macro tap thread: {e}"))?;
        Ok(())
    }

    fn run_event_tap_loop(shared: Arc<Shared>) -> Result<(), String> {
        let events = vec![
            CGEventType::LeftMouseDown,
            CGEventType::LeftMouseUp,
            CGEventType::RightMouseDown,
            CGEventType::RightMouseUp,
            CGEventType::MouseMoved,
            CGEventType::LeftMouseDragged,
            CGEventType::KeyDown,
        ];

        let shared_cb = shared.clone();
        let tap = CGEventTap::new(
            CGEventTapLocation::HID,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::ListenOnly,
            events,
            move |_proxy, event_type, event| {
                handle_tap_event(&shared_cb, event_type, event);
                Some(event.clone())
            },
        )
        .map_err(|_| {
            "CGEventTapCreate failed for macro recorder (need Input Monitoring)".to_string()
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
        log::info!("mouse macro recorder event tap started");
        CFRunLoop::run_current();
        Ok(())
    }

    fn current_frontmost_bundle() -> Option<String> {
        frontmost_window_bounds(Some(std::process::id())).and_then(|b| b.bundle_id)
    }

    fn handle_tap_event(shared: &Shared, event_type: CGEventType, event: &CGEvent) {
        let mode = RecordMode::from_u8(shared.mode.load(Ordering::Relaxed));
        if mode == RecordMode::Idle {
            return;
        }

        if matches!(event_type, CGEventType::KeyDown) {
            let keycode = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
            let flags = event.get_flags();
            let cmd = flags.contains(CGEventFlags::CGEventFlagCommand);
            let shift = flags.contains(CGEventFlags::CGEventFlagShift);
            let ctrl = flags.contains(CGEventFlags::CGEventFlagControl);
            let alt = flags.contains(CGEventFlags::CGEventFlagAlternate);

            // Esc 取消
            if keycode == 53 {
                shared.mode.store(RecordMode::Idle as u8, Ordering::Relaxed);
                if let Ok(mut out) = shared.outcome.lock() {
                    *out = Some(MacroRecordOutcome::Cancelled);
                }
                wake(shared);
                return;
            }
            // ⌘⇧. 结束序列
            if mode == RecordMode::Sequence && cmd && shift && keycode == 47 {
                let events = shared.buffer.lock().map(|b| b.clone()).unwrap_or_default();
                shared.mode.store(RecordMode::Idle as u8, Ordering::Relaxed);
                if let Ok(mut out) = shared.outcome.lock() {
                    *out = Some(MacroRecordOutcome::Sequence { events });
                }
                wake(shared);
                return;
            }

            // 序列模式：录制快捷键（忽略单独修饰键；忽略自动重复）
            if mode == RecordMode::Sequence {
                let autorepeat =
                    event.get_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT);
                if autorepeat != 0 {
                    return;
                }
                // 修饰键本身的 keycode：55 Cmd, 56 Shift, 58 Option, 59 Control
                if matches!(keycode, 54 | 55 | 56 | 57 | 58 | 59 | 60 | 61 | 62 | 63) {
                    return;
                }
                let Some(ch) = keycode_to_char(keycode) else {
                    return;
                };
                let mut keys = flags_to_modifiers(cmd, ctrl, alt, shift);
                keys.push(Key::Char(ch));
                push_event(
                    shared,
                    RawMacroEvent::Key {
                        keys,
                        at: Instant::now(),
                        frontmost_bundle: current_frontmost_bundle(),
                    },
                );
            }
            return;
        }

        let location = event.location();
        let x = location.x;
        let y = location.y;
        let kind = match event_type {
            CGEventType::LeftMouseDown => RawMouseKind::LeftDown,
            CGEventType::LeftMouseUp => RawMouseKind::LeftUp,
            CGEventType::RightMouseDown => RawMouseKind::RightDown,
            CGEventType::RightMouseUp => RawMouseKind::RightUp,
            CGEventType::MouseMoved | CGEventType::LeftMouseDragged => RawMouseKind::Move,
            _ => return,
        };

        match mode {
            RecordMode::CaptureOnce => {
                if kind == RawMouseKind::LeftDown {
                    shared.mode.store(RecordMode::Idle as u8, Ordering::Relaxed);
                    if let Ok(mut out) = shared.outcome.lock() {
                        *out = Some(MacroRecordOutcome::CaptureOnce { x, y });
                    }
                    wake(shared);
                }
            }
            RecordMode::Sequence => {
                let frontmost_bundle = if matches!(
                    kind,
                    RawMouseKind::LeftDown
                        | RawMouseKind::LeftUp
                        | RawMouseKind::RightDown
                        | RawMouseKind::RightUp
                ) {
                    current_frontmost_bundle()
                } else {
                    None
                };
                push_event(
                    shared,
                    RawMacroEvent::Mouse {
                        kind,
                        x,
                        y,
                        at: Instant::now(),
                        frontmost_bundle,
                    },
                );
            }
            RecordMode::Idle => {}
        }
    }

    fn push_event(shared: &Shared, event: RawMacroEvent) {
        if let Ok(mut buf) = shared.buffer.lock() {
            if buf.len() < 20_000 {
                buf.push(event);
            }
        }
    }

    fn wake(shared: &Shared) {
        shared.wake.poke();
    }
}
