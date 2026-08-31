#[cfg(any(feature = "ui-egui", test))]
pub mod editor_title;
pub mod watcher;

#[cfg(any(feature = "ui-egui", test))]
pub use editor_title::header_app_tooltip;
pub use watcher::{fetch_active_window, spawn_watcher, WindowWatcherControl};
