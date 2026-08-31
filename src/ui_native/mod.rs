//! AppKit 原生 UI：面板 + 设置 / 编辑 / 图标 / 权限（无 egui/GL）。
//!
//! objc2 0.3 起大量 AppKit API 已标为安全，历史代码仍包着 `unsafe`；
//! 逐处拆掉收益低，这里允许 unused_unsafe，避免淹没真正的问题。
#![allow(unused_unsafe)]

mod activation;
mod appearance_editor;
mod button_edit_session;
mod button_editor;
mod button_editor_common;
mod events;
mod backup_dialog;
mod export_dialog;
mod import_dialog;
mod icon_cache;
mod icon_picker;
mod layout;
mod panel;
mod permissions;
mod runtime;
mod meoflow_highlight;
mod script_editor_pane;
mod script_editor_toolbar;
mod script_wizard;
mod settings;

pub use runtime::run_native;
