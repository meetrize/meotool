//! 宏关键点简化：鼠标点击、快捷键、激活应用、必要等待。
//!
//! 完整序列简化（`simplify_events`）主要由 egui 录制流使用；native 默认只走单击捕获。

#![cfg_attr(not(any(feature = "ui-egui", test)), allow(dead_code))]

use std::time::Instant;

use crate::config::{Key, MacroStep, MouseButtonKind, MouseMacro};
use crate::platform::window_bounds::resolve_click_target;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawMouseKind {
    Move,
    LeftDown,
    LeftUp,
    RightDown,
    RightUp,
}

#[derive(Debug, Clone)]
pub enum RawMacroEvent {
    Mouse {
        kind: RawMouseKind,
        x: f64,
        y: f64,
        at: Instant,
        /// 事件发生时的前台应用 Bundle ID（排除 MeoTool）
        frontmost_bundle: Option<String>,
    },
    Key {
        keys: Vec<Key>,
        at: Instant,
        frontmost_bundle: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct SimplifyResult {
    pub steps: Vec<MacroStep>,
    pub raw_count: usize,
    pub recorded_bundle_id: Option<String>,
}

const DRAG_MIN_DISTANCE_PX: f64 = 8.0;
const KEEP_DELAY_MS: u64 = 200;
const MAX_AUTO_DELAY_MS: u64 = 800;

/// Aggressive 简化：点击 / 快捷键 / 应用激活 / 必要 delay。
pub fn simplify_events(events: &[RawMacroEvent], exclude_pid: Option<u32>) -> SimplifyResult {
    let raw_count = events.len();
    if events.is_empty() {
        return SimplifyResult {
            steps: Vec::new(),
            raw_count,
            recorded_bundle_id: None,
        };
    }

    let mut steps = Vec::new();
    let mut pending_down: Option<&RawMacroEvent> = None;
    let mut last_step_at: Option<Instant> = None;
    let mut last_bundle: Option<String> = None;
    let mut recorded_bundle_id = None;

    for event in events {
        match event {
            RawMacroEvent::Mouse { kind, .. } => match kind {
                RawMouseKind::Move => {}
                RawMouseKind::LeftDown | RawMouseKind::RightDown => {
                    pending_down = Some(event);
                }
                RawMouseKind::LeftUp | RawMouseKind::RightUp => {
                    let Some(down) = pending_down.take() else {
                        continue;
                    };
                    let RawMacroEvent::Mouse {
                        kind: down_kind,
                        x: dx,
                        y: dy,
                        at: down_at,
                        frontmost_bundle,
                    } = down
                    else {
                        continue;
                    };
                    let RawMacroEvent::Mouse {
                        kind: up_kind,
                        x: ux,
                        y: uy,
                        at: up_at,
                        ..
                    } = event
                    else {
                        continue;
                    };

                    let dist = distance(*dx, *dy, *ux, *uy);
                    let click_x = if dist >= DRAG_MIN_DISTANCE_PX {
                        *ux
                    } else {
                        *dx
                    };
                    let click_y = if dist >= DRAG_MIN_DISTANCE_PX {
                        *uy
                    } else {
                        *dy
                    };
                    let button = if dist >= DRAG_MIN_DISTANCE_PX {
                        button_for_up(*up_kind)
                    } else {
                        button_for_down(*down_kind)
                    };
                    let at = if dist >= DRAG_MIN_DISTANCE_PX {
                        *up_at
                    } else {
                        *down_at
                    };

                    push_delay(&mut steps, &mut last_step_at, at);
                    maybe_activate(
                        &mut steps,
                        &mut last_bundle,
                        &mut recorded_bundle_id,
                        frontmost_bundle.as_deref(),
                    );
                    let (step, bundle) =
                        click_step_from_screen(click_x, click_y, button, exclude_pid);
                    if recorded_bundle_id.is_none() {
                        recorded_bundle_id = frontmost_bundle.clone().or_else(|| bundle.clone());
                    }
                    if let Some(b) = frontmost_bundle.clone().or_else(|| bundle.clone()) {
                        last_bundle = Some(b);
                    }
                    steps.push(step);
                    last_step_at = Some(at);
                }
            },
            RawMacroEvent::Key {
                keys,
                at,
                frontmost_bundle,
            } => {
                if keys.is_empty() {
                    continue;
                }
                push_delay(&mut steps, &mut last_step_at, *at);
                maybe_activate(
                    &mut steps,
                    &mut last_bundle,
                    &mut recorded_bundle_id,
                    frontmost_bundle.as_deref(),
                );
                if recorded_bundle_id.is_none() {
                    recorded_bundle_id = frontmost_bundle.clone();
                }
                if let Some(b) = frontmost_bundle.clone() {
                    last_bundle = Some(b);
                }
                steps.push(MacroStep::Hotkey {
                    keys: keys.clone(),
                    note: Some(format_hotkey(keys)),
                });
                last_step_at = Some(*at);
            }
        }
    }

    if let Some(down) = pending_down {
        if let RawMacroEvent::Mouse {
            kind,
            x,
            y,
            at,
            frontmost_bundle,
        } = down
        {
            push_delay(&mut steps, &mut last_step_at, *at);
            maybe_activate(
                &mut steps,
                &mut last_bundle,
                &mut recorded_bundle_id,
                frontmost_bundle.as_deref(),
            );
            let (step, bundle) =
                click_step_from_screen(*x, *y, button_for_down(*kind), exclude_pid);
            if recorded_bundle_id.is_none() {
                recorded_bundle_id = bundle.or_else(|| frontmost_bundle.clone());
            }
            steps.push(step);
        }
    }

    if steps.len() > MouseMacro::MAX_STEPS {
        steps.truncate(MouseMacro::MAX_STEPS);
    }

    SimplifyResult {
        steps,
        raw_count,
        recorded_bundle_id,
    }
}

/// 单击捕获：单次按下即生成一步（可含激活应用）。
pub fn simplify_capture_once(
    screen_x: f64,
    screen_y: f64,
    exclude_pid: Option<u32>,
) -> SimplifyResult {
    let (step, bundle) =
        click_step_from_screen(screen_x, screen_y, MouseButtonKind::Left, exclude_pid);
    let mut steps = Vec::new();
    if let Some(ref b) = bundle {
        steps.push(MacroStep::ActivateApp {
            bundle_id: b.clone(),
            note: None,
        });
    }
    steps.push(step);
    SimplifyResult {
        steps,
        raw_count: 1,
        recorded_bundle_id: bundle,
    }
}

fn maybe_activate(
    steps: &mut Vec<MacroStep>,
    last_bundle: &mut Option<String>,
    recorded_bundle_id: &mut Option<String>,
    bundle: Option<&str>,
) {
    let Some(bundle) = bundle.filter(|s| !s.is_empty()) else {
        return;
    };
    if recorded_bundle_id.is_none() {
        *recorded_bundle_id = Some(bundle.to_string());
    }
    if last_bundle.as_deref() == Some(bundle) {
        return;
    }
    steps.push(MacroStep::ActivateApp {
        bundle_id: bundle.to_string(),
        note: None,
    });
    *last_bundle = Some(bundle.to_string());
}

fn click_step_from_screen(
    screen_x: f64,
    screen_y: f64,
    button: MouseButtonKind,
    exclude_pid: Option<u32>,
) -> (MacroStep, Option<String>) {
    let (x, y, coord_space, bounds) = resolve_click_target(screen_x, screen_y, exclude_pid);
    let bundle = bounds.and_then(|b| b.bundle_id);
    (
        MacroStep::Click {
            button,
            x,
            y,
            coord_space,
            note: None,
        },
        bundle,
    )
}

fn push_delay(steps: &mut Vec<MacroStep>, last_step_at: &mut Option<Instant>, now: Instant) {
    let Some(prev) = *last_step_at else {
        return;
    };
    let gap_ms = now.duration_since(prev).as_millis() as u64;
    if gap_ms > KEEP_DELAY_MS {
        let ms = gap_ms.min(MAX_AUTO_DELAY_MS);
        steps.push(MacroStep::Delay { ms });
    }
}

fn distance(x0: f64, y0: f64, x1: f64, y1: f64) -> f64 {
    let dx = x1 - x0;
    let dy = y1 - y0;
    (dx * dx + dy * dy).sqrt()
}

fn button_for_down(kind: RawMouseKind) -> MouseButtonKind {
    match kind {
        RawMouseKind::RightDown => MouseButtonKind::Right,
        _ => MouseButtonKind::Left,
    }
}

fn button_for_up(kind: RawMouseKind) -> MouseButtonKind {
    match kind {
        RawMouseKind::RightUp => MouseButtonKind::Right,
        _ => MouseButtonKind::Left,
    }
}

fn format_hotkey(keys: &[Key]) -> String {
    keys.iter()
        .map(|k| match k {
            Key::Cmd => "Cmd".to_string(),
            Key::Ctrl => "Ctrl".to_string(),
            Key::Alt => "Alt".to_string(),
            Key::Shift => "Shift".to_string(),
            Key::Char(' ') => "Space".to_string(),
            Key::Char(c) => c.to_string(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn simplify_click_and_hotkey() {
        let base = Instant::now();
        let events = vec![
            RawMacroEvent::Mouse {
                kind: RawMouseKind::LeftDown,
                x: 100.0,
                y: 100.0,
                at: base,
                frontmost_bundle: Some("com.apple.Safari".into()),
            },
            RawMacroEvent::Mouse {
                kind: RawMouseKind::LeftUp,
                x: 101.0,
                y: 100.0,
                at: base + Duration::from_millis(40),
                frontmost_bundle: Some("com.apple.Safari".into()),
            },
            RawMacroEvent::Key {
                keys: vec![Key::Cmd, Key::Char('s')],
                at: base + Duration::from_millis(300),
                frontmost_bundle: Some("com.apple.Safari".into()),
            },
        ];
        let result = simplify_events(&events, Some(u32::MAX));
        assert!(result
            .steps
            .iter()
            .any(|s| matches!(s, MacroStep::Click { .. })));
        assert!(result
            .steps
            .iter()
            .any(|s| matches!(s, MacroStep::Hotkey { .. })));
        assert!(result
            .steps
            .iter()
            .any(|s| matches!(s, MacroStep::ActivateApp { .. })));
        assert_eq!(
            result.recorded_bundle_id.as_deref(),
            Some("com.apple.Safari")
        );
    }
}
