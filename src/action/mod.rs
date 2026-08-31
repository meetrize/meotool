pub mod executor;
pub mod legacy_convert;
pub mod macro_player;
#[cfg(any(feature = "ui-egui", feature = "ui-native", test))]
pub mod macro_simplify;
pub mod script_compiler;
pub mod script_player;

pub use executor::{log_action_error, ActionExecutor};
#[cfg(any(feature = "ui-egui", feature = "ui-native", test))]
pub use macro_simplify::simplify_capture_once;
#[cfg(any(feature = "ui-egui", feature = "ui-native", test))]
pub use macro_simplify::simplify_events;
#[cfg(feature = "ui-egui")]
pub use macro_simplify::SimplifyResult;
pub use script_compiler::{compile, macro_to_meoflow, step_preview_label};
#[cfg(any(feature = "ui-egui", feature = "ui-native", test))]
pub use legacy_convert::{
    action_attached_macro, action_needs_legacy_upgrade, action_to_meoflow,
};
pub use script_player::{is_script_running, play_script_try_run};
