pub mod app_picker;
pub mod icon_tint;
pub mod icons_fs;
pub mod macos;
#[cfg(any(feature = "ui-egui", feature = "ui-native", test))]
pub mod macro_recorder;
pub mod mouse_monitor;
pub mod tray;
pub mod window_bounds;

pub use app_picker::{AppClickPicker, PickOutcome};
pub use icons_fs::{list_available_icons, resolve_icon_path};
pub use icon_tint::{make_tintable_rgba, resolve_native_icon_tint};
pub use macos::{
    bundle_id_from_process_path, check_accessibility_hint, check_permissions,
    enrich_editor_window_title, executable_path, fetch_browser_page_url, global_mouse_pos,
    open_accessibility_settings, open_input_monitoring_settings, process_path_from_pid,
    relaunch_app,
};
#[allow(unused_imports)]
pub use macos::PermissionStatus;
pub use macos::activate_app;
#[cfg(feature = "ui-egui")]
pub use macos::activate_self;
#[cfg(any(feature = "ui-egui", feature = "ui-native", test))]
pub use macro_recorder::{MacroRecordOutcome, MacroRecorder};
#[cfg(any(feature = "ui-egui", test))]
pub use macro_recorder::RecordMode;
pub use mouse_monitor::{MouseGestureAction, MouseGestureMonitor};
pub use tray::{TrayAction, TrayManager};
