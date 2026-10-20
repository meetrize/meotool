use std::collections::HashMap;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::app::WakeHandle;
use crate::config::GlobalHotkeys;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    ToggleVisibility,
    ReloadConfig,
    Quit,
    OpenSettings,
    /// Option + 小键盘数字，触发对应序号按钮
    TriggerButton(usize),
}

pub struct HotkeyManager {
    manager: GlobalHotKeyManager,
    bindings: HashMap<u32, HotkeyAction>,
    registered: Vec<HotKey>,
    /// 按钮自定义快捷键（与全局快捷键分册注册，便于单独刷新）
    button_bindings: HashMap<u32, usize>,
    button_registered: Vec<HotKey>,
    numpad_bindings: HashMap<u32, usize>,
    #[allow(dead_code)]
    numpad_registered: Vec<HotKey>,
    #[allow(dead_code)]
    numpad_armed: bool,
    #[allow(dead_code)]
    numpad_slots: usize,
    event_rx: mpsc::Receiver<GlobalHotKeyEvent>,
    _pump: JoinHandle<()>,
}

impl HotkeyManager {
    pub fn register(hotkeys: &GlobalHotkeys, wake: WakeHandle) -> Result<Self, String> {
        let manager = GlobalHotKeyManager::new().map_err(|e| format!("hotkey manager: {e}"))?;
        let mut bindings = HashMap::new();
        let mut registered = Vec::new();

        register_one(
            &manager,
            &mut bindings,
            &mut registered,
            &hotkeys.toggle_visibility,
            HotkeyAction::ToggleVisibility,
        )?;
        register_one(
            &manager,
            &mut bindings,
            &mut registered,
            &hotkeys.reload_config,
            HotkeyAction::ReloadConfig,
        )?;
        register_one(
            &manager,
            &mut bindings,
            &mut registered,
            &hotkeys.quit,
            HotkeyAction::Quit,
        )?;
        register_one(
            &manager,
            &mut bindings,
            &mut registered,
            &hotkeys.open_settings,
            HotkeyAction::OpenSettings,
        )?;

        log::info!(
            "registered global hotkeys: {}, {}, {}, {}",
            hotkeys.toggle_visibility,
            hotkeys.reload_config,
            hotkeys.quit,
            hotkeys.open_settings
        );

        let (event_tx, event_rx) = mpsc::channel();
        let wake_for_thread = wake;
        let pump = thread::Builder::new()
            .name("meotool-hotkey-pump".into())
            .spawn(move || loop {
                match GlobalHotKeyEvent::receiver().recv() {
                    Ok(event) => {
                        let _ = event_tx.send(event);
                        wake_for_thread.poke();
                    }
                    Err(_) => break,
                }
            })
            .map_err(|e| format!("spawn hotkey pump: {e}"))?;

        Ok(Self {
            manager,
            bindings,
            registered,
            button_bindings: HashMap::new(),
            button_registered: Vec::new(),
            numpad_bindings: HashMap::new(),
            numpad_registered: Vec::new(),
            numpad_armed: false,
            numpad_slots: 0,
            event_rx,
            _pump: pump,
        })
    }

    /// 按当前配置档按钮槽位注册自定义快捷键；空字符串 / 非法规格会跳过。
    pub fn set_button_hotkeys(&mut self, specs: &[(usize, &str)]) -> Result<(), String> {
        self.unregister_button_hotkeys()?;
        let mut seen_ids = std::collections::HashSet::new();
        for &(index, spec) in specs {
            let trimmed = spec.trim();
            if trimmed.is_empty() {
                continue;
            }
            let hotkey = match parse_global_hotkey(trimmed) {
                Ok(hk) => hk,
                Err(err) => {
                    log::warn!("skip button {index} hotkey '{trimmed}': {err}");
                    continue;
                }
            };
            let id = hotkey.id();
            if self.bindings.contains_key(&id) || seen_ids.contains(&id) {
                log::warn!(
                    "button {index} hotkey '{trimmed}' conflicts with another binding, skipped"
                );
                continue;
            }
            if let Err(err) = self.manager.register(hotkey) {
                log::warn!("register button {index} hotkey '{trimmed}': {err}");
                continue;
            }
            seen_ids.insert(id);
            self.button_bindings.insert(id, index);
            self.button_registered.push(hotkey);
        }
        if !self.button_registered.is_empty() {
            log::info!(
                "registered {} button hotkey(s)",
                self.button_registered.len()
            );
        }
        Ok(())
    }

    fn unregister_button_hotkeys(&mut self) -> Result<(), String> {
        if !self.button_registered.is_empty() {
            self.manager
                .unregister_all(&self.button_registered)
                .map_err(|e| format!("unregister button hotkeys: {e}"))?;
        }
        self.button_bindings.clear();
        self.button_registered.clear();
        Ok(())
    }

    /// 面板可见时注册 Option+小键盘数字；隐藏时注销
    #[cfg_attr(not(any(feature = "ui-egui", test)), allow(dead_code))]
    pub fn set_numpad_triggers(&mut self, slot_count: usize, armed: bool) -> Result<(), String> {
        if armed == self.numpad_armed && slot_count == self.numpad_slots {
            return Ok(());
        }

        self.unregister_numpad()?;

        if armed && slot_count > 0 {
            let count = slot_count.min(10);
            for index in 0..count {
                let code = numpad_code_for_index(index)
                    .ok_or_else(|| format!("unsupported numpad slot index: {index}"))?;
                let hotkey = HotKey::new(Some(Modifiers::ALT), code);
                let id = hotkey.id();
                self.manager
                    .register(hotkey)
                    .map_err(|e| format!("register numpad hotkey for slot {}: {e}", index + 1))?;
                self.numpad_bindings.insert(id, index);
                self.numpad_registered.push(hotkey);
            }
            self.numpad_armed = true;
            self.numpad_slots = slot_count;
            log::debug!("armed option+numpad triggers for {count} button slots");
        } else {
            self.numpad_armed = false;
            self.numpad_slots = 0;
        }

        Ok(())
    }

    #[cfg_attr(not(any(feature = "ui-egui", test)), allow(dead_code))]
    fn unregister_numpad(&mut self) -> Result<(), String> {
        if !self.numpad_registered.is_empty() {
            self.manager
                .unregister_all(&self.numpad_registered)
                .map_err(|e| format!("unregister numpad hotkeys: {e}"))?;
        }
        self.numpad_bindings.clear();
        self.numpad_registered.clear();
        Ok(())
    }

    pub fn replace(&mut self, hotkeys: &GlobalHotkeys) -> Result<(), String> {
        validate_hotkeys(hotkeys)?;

        if !self.registered.is_empty() {
            self.manager
                .unregister_all(&self.registered)
                .map_err(|e| format!("unregister hotkeys: {e}"))?;
        }
        self.bindings.clear();
        self.registered.clear();

        register_one(
            &self.manager,
            &mut self.bindings,
            &mut self.registered,
            &hotkeys.toggle_visibility,
            HotkeyAction::ToggleVisibility,
        )?;
        register_one(
            &self.manager,
            &mut self.bindings,
            &mut self.registered,
            &hotkeys.reload_config,
            HotkeyAction::ReloadConfig,
        )?;
        register_one(
            &self.manager,
            &mut self.bindings,
            &mut self.registered,
            &hotkeys.quit,
            HotkeyAction::Quit,
        )?;
        register_one(
            &self.manager,
            &mut self.bindings,
            &mut self.registered,
            &hotkeys.open_settings,
            HotkeyAction::OpenSettings,
        )?;

        log::info!("re-registered global hotkeys");
        Ok(())
    }

    pub fn poll(&self) -> Option<HotkeyAction> {
        while let Ok(event) = self.event_rx.try_recv() {
            if event.state == HotKeyState::Pressed {
                if let Some(action) = self.bindings.get(&event.id) {
                    return Some(*action);
                }
                if let Some(&index) = self.button_bindings.get(&event.id) {
                    return Some(HotkeyAction::TriggerButton(index));
                }
                if let Some(&index) = self.numpad_bindings.get(&event.id) {
                    return Some(HotkeyAction::TriggerButton(index));
                }
            }
        }
        None
    }
}

/// 按钮序号 1-9 对应小键盘 1-9，第 10 个对应小键盘 0
#[cfg_attr(not(any(feature = "ui-egui", test)), allow(dead_code))]
fn numpad_code_for_index(index: usize) -> Option<Code> {
    match index {
        0 => Some(Code::Numpad1),
        1 => Some(Code::Numpad2),
        2 => Some(Code::Numpad3),
        3 => Some(Code::Numpad4),
        4 => Some(Code::Numpad5),
        5 => Some(Code::Numpad6),
        6 => Some(Code::Numpad7),
        7 => Some(Code::Numpad8),
        8 => Some(Code::Numpad9),
        9 => Some(Code::Numpad0),
        _ => None,
    }
}

fn validate_hotkeys(hotkeys: &GlobalHotkeys) -> Result<(), String> {
    parse_global_hotkey(&hotkeys.toggle_visibility)?;
    parse_global_hotkey(&hotkeys.reload_config)?;
    parse_global_hotkey(&hotkeys.quit)?;
    parse_global_hotkey(&hotkeys.open_settings)?;
    Ok(())
}

fn register_one(
    manager: &GlobalHotKeyManager,
    bindings: &mut HashMap<u32, HotkeyAction>,
    registered: &mut Vec<HotKey>,
    spec: &str,
    action: HotkeyAction,
) -> Result<(), String> {
    let hotkey = parse_global_hotkey(spec)?;
    let id = hotkey.id();
    manager
        .register(hotkey)
        .map_err(|e| format!("register hotkey '{spec}': {e}"))?;
    bindings.insert(id, action);
    registered.push(hotkey);
    Ok(())
}

pub fn parse_global_hotkey(spec: &str) -> Result<HotKey, String> {
    let parts: Vec<&str> = spec
        .split('+')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return Err("empty hotkey".into());
    }

    let mut mods = Modifiers::empty();
    let mut key_token: Option<&str> = None;

    for part in parts {
        let lower = part.to_lowercase();
        match lower.as_str() {
            "cmd" | "command" | "meta" | "super" => mods |= Modifiers::META,
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "alt" | "option" => mods |= Modifiers::ALT,
            "shift" => mods |= Modifiers::SHIFT,
            _ => key_token = Some(part),
        }
    }

    let token = key_token.ok_or_else(|| format!("hotkey '{spec}' missing key"))?;
    let code = token_to_code(token)?;
    Ok(HotKey::new(Some(mods), code))
}

fn token_to_code(token: &str) -> Result<Code, String> {
    if token.len() == 1 {
        let c = token.chars().next().unwrap();
        if c.is_ascii_digit() {
            return match c {
                '0' => Ok(Code::Digit0),
                '1' => Ok(Code::Digit1),
                '2' => Ok(Code::Digit2),
                '3' => Ok(Code::Digit3),
                '4' => Ok(Code::Digit4),
                '5' => Ok(Code::Digit5),
                '6' => Ok(Code::Digit6),
                '7' => Ok(Code::Digit7),
                '8' => Ok(Code::Digit8),
                '9' => Ok(Code::Digit9),
                _ => Err(format!("unsupported digit key: {c}")),
            };
        }
        if c.is_ascii_alphabetic() {
            return letter_to_code(c.to_ascii_uppercase());
        }
    }

    match token.to_lowercase().as_str() {
        "`" | "backquote" => Ok(Code::Backquote),
        "," => Ok(Code::Comma),
        other => Err(format!("unsupported hotkey key token: {other}")),
    }
}

fn letter_to_code(c: char) -> Result<Code, String> {
    Ok(match c {
        'A' => Code::KeyA,
        'B' => Code::KeyB,
        'C' => Code::KeyC,
        'D' => Code::KeyD,
        'E' => Code::KeyE,
        'F' => Code::KeyF,
        'G' => Code::KeyG,
        'H' => Code::KeyH,
        'I' => Code::KeyI,
        'J' => Code::KeyJ,
        'K' => Code::KeyK,
        'L' => Code::KeyL,
        'M' => Code::KeyM,
        'N' => Code::KeyN,
        'O' => Code::KeyO,
        'P' => Code::KeyP,
        'Q' => Code::KeyQ,
        'R' => Code::KeyR,
        'S' => Code::KeyS,
        'T' => Code::KeyT,
        'U' => Code::KeyU,
        'V' => Code::KeyV,
        'W' => Code::KeyW,
        'X' => Code::KeyX,
        'Y' => Code::KeyY,
        'Z' => Code::KeyZ,
        _ => return Err(format!("unsupported letter: {c}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cmd_shift_p() {
        let _hk = parse_global_hotkey("Cmd+Shift+P").unwrap();
    }

    #[test]
    fn parse_cmd_shift_comma() {
        let _hk = parse_global_hotkey("Cmd+Shift+,").unwrap();
    }

    #[test]
    fn numpad_index_maps_to_expected_codes() {
        assert_eq!(numpad_code_for_index(0), Some(Code::Numpad1));
        assert_eq!(numpad_code_for_index(8), Some(Code::Numpad9));
        assert_eq!(numpad_code_for_index(9), Some(Code::Numpad0));
    }
}
