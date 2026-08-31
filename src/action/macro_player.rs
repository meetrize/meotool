use std::thread;
use std::time::Duration;

use enigo::{
    Button, Coordinate,
    Direction::{Click, Press, Release},
    Enigo, Key as EnigoKey, Keyboard, Mouse,
};
use log::info;

use crate::config::{CoordSpace, Key, MacroStep, MouseButtonKind, MouseMacro};
use crate::platform::window_bounds::{activate_bundle_id, target_to_screen};

pub fn play_mouse_macro(enigo: &mut Enigo, macro_action: &MouseMacro) -> Result<(), String> {
    if macro_action.steps.is_empty() {
        return Err("鼠标宏没有步骤".into());
    }
    if macro_action.steps.len() > MouseMacro::MAX_STEPS {
        return Err(format!(
            "鼠标宏步骤过多（最多 {} 步）",
            MouseMacro::MAX_STEPS
        ));
    }

    // 记录触发前光标位置（通常就是面板按钮上），回放结束后还原
    let restore_pos = enigo.location().ok();

    let exclude_pid = Some(std::process::id());
    let mut preferred_bundle = macro_action.recorded_bundle_id.clone();
    let mut spent_ms = 0u64;

    // 若步骤里没有 ActivateApp，仍用录制 Bundle 预激活一次
    let has_activate = macro_action
        .steps
        .iter()
        .any(|s| matches!(s, MacroStep::ActivateApp { .. }));
    if !has_activate {
        if let Some(bundle) = preferred_bundle.as_deref() {
            if let Err(err) = activate_bundle_id(bundle) {
                log::warn!("activate recorded app before macro: {err}");
            }
        }
    }

    let pre_delay = macro_action.pre_delay_ms.max(200);
    spent_ms = spent_ms.saturating_add(pre_delay);
    if spent_ms > MouseMacro::MAX_TOTAL_MS {
        restore_cursor(enigo, restore_pos);
        return Err("鼠标宏总时长超限".into());
    }
    thread::sleep(Duration::from_millis(pre_delay));

    let result = (|| {
        for (index, step) in macro_action.steps.iter().enumerate() {
            match step {
                MacroStep::Delay { ms } => {
                    let ms = (*ms).min(MouseMacro::MAX_TOTAL_MS);
                    spent_ms = spent_ms.saturating_add(ms);
                    if spent_ms > MouseMacro::MAX_TOTAL_MS {
                        return Err(format!("鼠标宏在第 {} 步超时", index + 1));
                    }
                    if ms > 0 {
                        thread::sleep(Duration::from_millis(ms));
                    }
                }
                MacroStep::ActivateApp { bundle_id, .. } => {
                    activate_bundle_id(bundle_id)?;
                    preferred_bundle = Some(bundle_id.clone());
                    spent_ms = spent_ms.saturating_add(150);
                    if spent_ms > MouseMacro::MAX_TOTAL_MS {
                        return Err(format!("鼠标宏在第 {} 步超时", index + 1));
                    }
                    thread::sleep(Duration::from_millis(150));
                }
                MacroStep::Hotkey { keys, .. } => {
                    play_hotkey(enigo, keys)?;
                    spent_ms = spent_ms.saturating_add(40);
                    if spent_ms > MouseMacro::MAX_TOTAL_MS {
                        return Err(format!("鼠标宏在第 {} 步超时", index + 1));
                    }
                    thread::sleep(Duration::from_millis(40));
                }
                MacroStep::Click {
                    button,
                    x,
                    y,
                    coord_space,
                    ..
                } => {
                    play_click(
                        enigo,
                        *button,
                        *x,
                        *y,
                        *coord_space,
                        exclude_pid,
                        preferred_bundle.as_deref(),
                    )?;
                    spent_ms = spent_ms.saturating_add(30);
                    if spent_ms > MouseMacro::MAX_TOTAL_MS {
                        return Err(format!("鼠标宏在第 {} 步超时", index + 1));
                    }
                    thread::sleep(Duration::from_millis(30));
                }
            }
        }
        Ok(())
    })();

    restore_cursor(enigo, restore_pos);

    result?;
    info!(
        "executed mouse macro: {} steps, bundle={:?}",
        macro_action.steps.len(),
        macro_action.recorded_bundle_id
    );
    Ok(())
}

fn restore_cursor(enigo: &mut Enigo, pos: Option<(i32, i32)>) {
    let Some((x, y)) = pos else {
        return;
    };
    if let Err(err) = enigo.move_mouse(x, y, Coordinate::Abs) {
        log::warn!("restore cursor after macro failed: {err}");
    }
}

fn play_click(
    enigo: &mut Enigo,
    button: MouseButtonKind,
    x: f32,
    y: f32,
    coord_space: CoordSpace,
    exclude_pid: Option<u32>,
    preferred_bundle_id: Option<&str>,
) -> Result<(), String> {
    let (sx, sy) = target_to_screen(x, y, coord_space, exclude_pid, preferred_bundle_id)?;
    enigo
        .move_mouse(sx, sy, Coordinate::Abs)
        .map_err(|e| format!("移动鼠标失败: {e}"))?;
    thread::sleep(Duration::from_millis(16));
    enigo
        .button(to_enigo_button(button), Click)
        .map_err(|e| format!("鼠标点击失败: {e}"))?;
    Ok(())
}

fn play_hotkey(enigo: &mut Enigo, keys: &[Key]) -> Result<(), String> {
    let mut modifiers = Vec::new();
    let mut main_key = None;

    for key in keys {
        match key {
            Key::Cmd | Key::Ctrl | Key::Alt | Key::Shift => modifiers.push(*key),
            Key::Char(c) => {
                if main_key.is_some() {
                    return Err("multiple main keys in hotkey".into());
                }
                main_key = Some(*c);
            }
        }
    }

    let main_key = main_key.ok_or_else(|| "hotkey missing main key".to_string())?;

    for modifier in &modifiers {
        press_modifier(enigo, *modifier)?;
    }

    enigo
        .key(EnigoKey::Unicode(main_key), Click)
        .map_err(|e| format!("key click failed: {e}"))?;

    for modifier in modifiers.iter().rev() {
        release_modifier(enigo, *modifier)?;
    }

    info!("macro hotkey: {:?}", keys);
    Ok(())
}

fn press_modifier(enigo: &mut Enigo, key: Key) -> Result<(), String> {
    let enigo_key = match key {
        Key::Cmd => EnigoKey::Meta,
        Key::Ctrl => EnigoKey::Control,
        Key::Alt => EnigoKey::Alt,
        Key::Shift => EnigoKey::Shift,
        Key::Char(_) => return Ok(()),
    };
    enigo
        .key(enigo_key, Press)
        .map_err(|e| format!("modifier press failed: {e}"))
}

fn release_modifier(enigo: &mut Enigo, key: Key) -> Result<(), String> {
    let enigo_key = match key {
        Key::Cmd => EnigoKey::Meta,
        Key::Ctrl => EnigoKey::Control,
        Key::Alt => EnigoKey::Alt,
        Key::Shift => EnigoKey::Shift,
        Key::Char(_) => return Ok(()),
    };
    enigo
        .key(enigo_key, Release)
        .map_err(|e| format!("modifier release failed: {e}"))
}

fn to_enigo_button(button: MouseButtonKind) -> Button {
    match button {
        MouseButtonKind::Left => Button::Left,
        MouseButtonKind::Right => Button::Right,
        MouseButtonKind::Middle => Button::Middle,
    }
}
