mod action;
mod app;
mod config;
mod input;
mod platform;
mod window;

#[cfg(feature = "ui-egui")]
mod ui;

#[cfg(all(feature = "ui-egui", not(feature = "ui-native")))]
mod egui_app;

#[cfg(feature = "ui-native")]
mod ui_native;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(feature = "ui-native")]
fn main() {
    if let Err(err) = ui_native::run_native() {
        eprintln!("meotool native failed: {err}");
        std::process::exit(1);
    }
}

#[cfg(all(feature = "ui-egui", not(feature = "ui-native")))]
fn main() -> eframe::Result<()> {
    egui_app::run()
}

#[cfg(all(not(feature = "ui-egui"), not(feature = "ui-native")))]
fn main() {
    eprintln!("enable feature ui-native or ui-egui");
    std::process::exit(1);
}
