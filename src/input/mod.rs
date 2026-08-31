pub mod hotkey;

#[cfg(feature = "ui-egui")]
pub mod keyboard;

pub use hotkey::{HotkeyAction, HotkeyManager};

#[cfg(feature = "ui-egui")]
pub use keyboard::{handle_panel_keyboard, KeyboardAction};
