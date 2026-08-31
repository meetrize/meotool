use eframe::egui::{self, Context, Key, Modifiers};

/// 快捷键录制槽位
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordingSlot {
    ToggleVisibility,
    ReloadConfig,
    Quit,
    OpenSettings,
    Button { profile: usize, button: usize },
}

pub struct HotkeyRecorder {
    pub active: Option<RecordingSlot>,
}

impl Default for HotkeyRecorder {
    fn default() -> Self {
        Self { active: None }
    }
}

impl HotkeyRecorder {
    pub fn start(&mut self, slot: RecordingSlot) {
        self.active = Some(slot);
    }

    pub fn cancel(&mut self) {
        self.active = None;
    }

    pub fn is_recording(&self, slot: RecordingSlot) -> bool {
        self.active == Some(slot)
    }

    /// 录制中则尝试捕获按键，返回 Some(hotkey_string)
    pub fn poll(&mut self, ctx: &Context, slot: RecordingSlot) -> Option<String> {
        if self.active != Some(slot) {
            return None;
        }

        let mut captured = None;
        ctx.input(|input| {
            for event in &input.events {
                let egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } = event
                else {
                    continue;
                };

                if is_modifier_only(*key, *modifiers) {
                    continue;
                }

                if let Some(spec) = key_event_to_spec(*key, *modifiers) {
                    captured = Some(spec);
                }
            }
        });

        if captured.is_some() {
            self.active = None;
        }
        captured
    }
}

fn is_modifier_only(key: Key, modifiers: Modifiers) -> bool {
    key_to_name(key).is_none() && (modifiers.any())
}

fn key_event_to_spec(key: Key, modifiers: Modifiers) -> Option<String> {
    let mut parts = Vec::new();
    if modifiers.command {
        parts.push("Cmd");
    }
    if modifiers.ctrl {
        parts.push("Ctrl");
    }
    if modifiers.alt {
        parts.push("Alt");
    }
    if modifiers.shift {
        parts.push("Shift");
    }

    if let Some(name) = key_to_name(key) {
        parts.push(name);
        Some(parts.join("+"))
    } else {
        None
    }
}

fn key_to_name(key: Key) -> Option<&'static str> {
    match key {
        Key::A => Some("A"),
        Key::B => Some("B"),
        Key::C => Some("C"),
        Key::D => Some("D"),
        Key::E => Some("E"),
        Key::F => Some("F"),
        Key::G => Some("G"),
        Key::H => Some("H"),
        Key::I => Some("I"),
        Key::J => Some("J"),
        Key::K => Some("K"),
        Key::L => Some("L"),
        Key::M => Some("M"),
        Key::N => Some("N"),
        Key::O => Some("O"),
        Key::P => Some("P"),
        Key::Q => Some("Q"),
        Key::R => Some("R"),
        Key::S => Some("S"),
        Key::T => Some("T"),
        Key::U => Some("U"),
        Key::V => Some("V"),
        Key::W => Some("W"),
        Key::X => Some("X"),
        Key::Y => Some("Y"),
        Key::Z => Some("Z"),
        Key::Num0 => Some("0"),
        Key::Num1 => Some("1"),
        Key::Num2 => Some("2"),
        Key::Num3 => Some("3"),
        Key::Num4 => Some("4"),
        Key::Num5 => Some("5"),
        Key::Num6 => Some("6"),
        Key::Num7 => Some("7"),
        Key::Num8 => Some("8"),
        Key::Num9 => Some("9"),
        Key::F1 => Some("F1"),
        Key::F2 => Some("F2"),
        Key::F3 => Some("F3"),
        Key::F4 => Some("F4"),
        Key::F5 => Some("F5"),
        Key::F6 => Some("F6"),
        Key::F7 => Some("F7"),
        Key::F8 => Some("F8"),
        Key::F9 => Some("F9"),
        Key::F10 => Some("F10"),
        Key::F11 => Some("F11"),
        Key::F12 => Some("F12"),
        Key::Space => Some("Space"),
        Key::Tab => Some("Tab"),
        Key::Enter => Some("Enter"),
        Key::Escape => Some("Escape"),
        Key::Backspace => Some("Backspace"),
        Key::Delete => Some("Delete"),
        Key::Backtick => Some("`"),
        Key::Comma => Some(","),
        Key::Period => Some("."),
        Key::Slash => Some("/"),
        Key::Semicolon => Some(";"),
        Key::Quote => Some("'"),
        Key::OpenBracket => Some("["),
        Key::CloseBracket => Some("]"),
        Key::Backslash => Some("\\"),
        Key::Minus => Some("-"),
        Key::Equals => Some("="),
        _ => None,
    }
}

pub fn hotkey_field(
    ui: &mut egui::Ui,
    ctx: &Context,
    label: &str,
    value: &mut String,
    recorder: &mut HotkeyRecorder,
    slot: RecordingSlot,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        let recording = recorder.is_recording(slot);
        let btn_text = if recording {
            "按下快捷键…"
        } else {
            value.as_str()
        };
        if ui
            .add_enabled(
                !recording,
                egui::Button::new(btn_text).min_size(egui::vec2(160.0, 0.0)),
            )
            .clicked()
        {
            recorder.start(slot);
        }
        if recording && ui.button("取消").clicked() {
            recorder.cancel();
        }
        if let Some(spec) = recorder.poll(ctx, slot) {
            *value = spec;
        }
    });
}
