pub mod core;
pub mod wake;

pub mod memory;
#[cfg(feature = "ui-egui")]
pub mod state;

pub use core::{AppCore, CoreCommand, CoreEffect};
pub use wake::WakeHandle;

pub use memory::{MemoryMonitor, NativeMemoryCounters};
#[cfg(feature = "ui-egui")]
pub use state::AppState;
