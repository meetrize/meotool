pub mod app_config;
pub mod backup;
pub mod button_presets;
pub mod cli;
pub mod color;
pub mod export;
pub mod import_export;
pub mod migrate;
pub mod model;
pub mod normalize;
pub mod parse;
pub mod persist;
pub mod resolver;
pub mod script_presets;
pub mod store;
pub mod toml_model;
pub mod watcher;

pub use app_config::{
    AppConfig, ContextMode, GestureConfig, GlobalHotkeys, PanelSettings, Profile, ThemeSettings,
    WindowInfo,
};
#[allow(unused_imports)]
pub use app_config::GlobalPanelDefaults;
#[allow(unused_imports)]
pub use backup::{
    backup_before_write, backups_dir, format_backup_entry, list_backups, restore_from_backup,
    BackupEntry, MAX_BACKUPS,
};
pub use cli::{parse_cli_args, run_cli};
#[allow(unused_imports)]
pub use button_presets::{
    appearance_hint_text, apply_preset, preset_index_for, ButtonColorPreset, BUTTON_COLOR_PRESETS,
};
pub use color::{parse_color, RgbColor};
#[allow(unused_imports)]
pub use color::color_to_string;
pub use export::save_app_config;
#[allow(unused_imports)]
pub use import_export::{
    default_bundle_dirname, export_bundle, export_config_toml, find_profile_conflicts,
    import_config, import_result_summary, preview_import, validate_config_file,
    ExportBundleResult, ImportMode, ImportPreview, ImportResult, ProfileConflictStrategy,
};
#[allow(unused_imports)]
pub use script_presets::{
    escape_meoflow_string, is_comments_only, preset_by_id, render_template, PresetBuildParams,
    PresetGroup, PresetParamKind, ScriptPreset, DEFAULT_SCRIPT_SKELETON, FLOW_PRESETS,
    QUICK_PRESETS,
};
pub use model::{
    empty_slot_button, Action, ActionScript, ButtonConfig, ButtonScope, CoordSpace, DisplayMode,
    FlowProgram, FlowStep, Key, MacroStep, MatchRule, MouseButtonKind, MouseMacro, PanelConfig,
    SpecialKey,
};
#[allow(unused_imports)]
pub use normalize::normalize_app_config;
pub use parse::parse_config_content;
pub use persist::save_panel_position;
pub use resolver::ProfileResolver;
pub use store::{app_data_dir, icon_search_dirs, LoadedConfig};
#[cfg(any(feature = "ui-egui", test))]
pub use watcher::debounce_duration;
pub use watcher::spawn_config_watcher;
