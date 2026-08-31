use std::path::Path;

use super::app_config::{AppConfig, MatchRule, PanelSettings, Profile, ThemeSettings};
use super::backup::{atomic_write, backup_before_write};
use super::migrate::{now_timestamp, CURRENT_CONFIG_VERSION};
use super::model::{Action, ButtonConfig, ButtonScope, DisplayMode, Key, MacroStep};
use super::toml_model::{
    coord_space_to_str, mouse_button_to_str, ActionFile, ButtonFile, ButtonScopeFile, ConfigFile,
    GestureSettingsFile, GlobalConfig, MacroStepFile, MatchRuleFile, PanelSettingsFile,
    PositionSettings, ProfileFile, ThemeSettingsFile,
};

pub fn save_app_config(path: &Path, app: &AppConfig) -> Result<(), String> {
    let file = app_config_to_file(app);
    let content = toml::to_string_pretty(&file).map_err(|e| format!("serialize config: {e}"))?;

    // 备份失败不阻断保存：新配置优先落盘
    if let Err(err) = backup_before_write(path) {
        log::warn!("config backup skipped: {err}");
    }

    atomic_write(path, content)?;
    log::info!("saved config to {}", path.display());
    Ok(())
}

pub fn app_config_to_file(app: &AppConfig) -> ConfigFile {
    ConfigFile {
        version: CURRENT_CONFIG_VERSION,
        updated_at: Some(now_timestamp()),
        global: GlobalConfig {
            panel: panel_settings_to_file(
                &app.global_panel.settings,
                Some(&app.global_panel.position),
            ),
            gesture: gesture_settings_to_file(&app.gesture),
            toggle_visibility: Some(app.hotkeys.toggle_visibility.clone()),
            reload_config: Some(app.hotkeys.reload_config.clone()),
            quit: Some(app.hotkeys.quit.clone()),
            open_settings: Some(app.hotkeys.open_settings.clone()),
        },
        profiles: app.profiles.iter().map(profile_to_file).collect(),
        window_poll_interval_ms: app.poll_interval_ms,
        context_mode: app.context_mode.as_str().to_string(),
    }
}

fn profile_to_file(profile: &Profile) -> ProfileFile {
    ProfileFile {
        name: profile.name.clone(),
        r#match: profile.match_rule.as_ref().map(match_rule_to_file),
        panel: panel_settings_to_file(&profile.panel, None),
        buttons: profile.buttons.iter().map(button_to_file).collect(),
    }
}

fn match_rule_to_file(rule: &MatchRule) -> MatchRuleFile {
    MatchRuleFile {
        bundle_id: rule.bundle_id.clone(),
        app_name: rule.app_name.clone(),
        title_regex: rule
            .title_regex_text
            .clone()
            .or_else(|| rule.title_regex.as_ref().map(|r| r.as_str().to_string())),
    }
}

fn button_dims_equal(w: f32, h: f32) -> bool {
    (w - h).abs() < f32::EPSILON
}

fn button_size_for_file(settings: &PanelSettings) -> Option<f32> {
    match (settings.button_width, settings.button_height) {
        (Some(w), Some(h)) if button_dims_equal(w, h) => Some(w),
        (Some(w), None) => Some(w),
        (None, Some(h)) => Some(h),
        _ => None,
    }
}

fn button_width_for_file(settings: &PanelSettings) -> Option<f32> {
    match (settings.button_width, settings.button_height) {
        (Some(w), Some(h)) if !button_dims_equal(w, h) => Some(w),
        _ => None,
    }
}

fn button_height_for_file(settings: &PanelSettings) -> Option<f32> {
    match (settings.button_width, settings.button_height) {
        (Some(w), Some(h)) if !button_dims_equal(w, h) => Some(h),
        _ => None,
    }
}

fn gesture_settings_to_file(gesture: &super::app_config::GestureConfig) -> GestureSettingsFile {
    GestureSettingsFile {
        enabled: Some(gesture.enabled),
        middle_click_enabled: Some(gesture.middle_click_enabled),
        drag_threshold_px: Some(gesture.drag_threshold_px),
        dismiss_on_outside_click: Some(gesture.dismiss_on_outside_click),
        hide_after_button: Some(gesture.hide_after_button),
        global_priority: Some(gesture.global_priority),
    }
}

fn panel_settings_to_file(
    settings: &PanelSettings,
    position: Option<&super::model::Position>,
) -> PanelSettingsFile {
    PanelSettingsFile {
        rows: settings.rows,
        cols: settings.cols,
        display_mode: settings.display_mode.map(display_mode_to_str),
        collapsed: settings.collapsed,
        opacity: settings.opacity,
        button_size: button_size_for_file(settings),
        button_width: button_width_for_file(settings),
        button_height: button_height_for_file(settings),
        gap: settings.gap,
        collapsed_icon: settings.collapsed_icon.clone(),
        position: position.map(|p| PositionSettings { x: p.x, y: p.y }),
        theme: theme_settings_to_file(&settings.theme),
    }
}

fn theme_settings_to_file(theme: &ThemeSettings) -> ThemeSettingsFile {
    ThemeSettingsFile {
        background_color: theme.background_color.clone(),
        button_color: theme.button_color.clone(),
        letter_background_color: theme.letter_background_color.clone(),
        letter_color: theme.letter_color.clone(),
        letter_size: theme.letter_size,
        letter_font: theme.letter_font.clone(),
        window_border_enabled: theme.window_border_enabled,
        window_border_width: theme.window_border_width,
        window_border_color: theme.window_border_color.clone(),
        button_border_enabled: theme.button_border_enabled,
        button_border_width: theme.button_border_width,
        button_border_color: theme.button_border_color.clone(),
    }
}

fn button_to_file(button: &ButtonConfig) -> ButtonFile {
    ButtonFile {
        label: button.label.clone(),
        icon: button.icon.clone(),
        enabled: button.enabled,
        action: action_to_file(&button.action),
        background_color: button.background_color.clone(),
        border_color: button.border_color.clone(),
        icon_color: button.icon_color.clone(),
        scope: scope_to_file(&button.scope),
    }
}

fn scope_to_file(scope: &ButtonScope) -> ButtonScopeFile {
    match scope {
        ButtonScope::Always => ButtonScopeFile::Always,
        ButtonScope::WhenActive(rule) => ButtonScopeFile::WhenActive {
            bundle_id: rule.bundle_id.clone(),
            app_name: rule.app_name.clone(),
            title_regex: rule
                .title_regex_text
                .clone()
                .or_else(|| rule.title_regex.as_ref().map(|r| r.as_str().to_string())),
        },
    }
}

fn action_to_file(action: &Action) -> ActionFile {
    match action {
        Action::Hotkey(keys) => ActionFile::Hotkey {
            keys: keys.iter().map(key_to_str).collect(),
        },
        Action::OpenApp { target } => ActionFile::OpenApp {
            target: target.clone(),
        },
        Action::Shell { command } => ActionFile::Shell {
            command: command.clone(),
        },
        Action::OpenUrl { url } => ActionFile::OpenUrl { url: url.clone() },
        Action::OpenFile { path } => ActionFile::OpenFile { path: path.clone() },
        Action::PasteText { text } => ActionFile::PasteText { text: text.clone() },
        Action::MouseMacro(m) => ActionFile::MouseMacro {
            steps: m.steps.iter().map(macro_step_to_file).collect(),
            pre_delay_ms: m.pre_delay_ms,
            recorded_bundle_id: m.recorded_bundle_id.clone(),
        },
        Action::Script(s) => ActionFile::Script {
            source: s.source.clone(),
        },
        Action::None => ActionFile::None,
    }
}

fn macro_step_to_file(step: &MacroStep) -> MacroStepFile {
    match step {
        MacroStep::Click {
            button,
            x,
            y,
            coord_space,
            note,
        } => MacroStepFile::Click {
            button: mouse_button_to_str(*button).to_string(),
            x: *x,
            y: *y,
            coord_space: coord_space_to_str(*coord_space).to_string(),
            note: note.clone(),
        },
        MacroStep::Hotkey { keys, note } => MacroStepFile::Hotkey {
            keys: keys.iter().map(key_to_str).collect(),
            note: note.clone(),
        },
        MacroStep::ActivateApp { bundle_id, note } => MacroStepFile::ActivateApp {
            bundle_id: bundle_id.clone(),
            note: note.clone(),
        },
        MacroStep::Delay { ms } => MacroStepFile::Delay { ms: *ms },
    }
}

fn key_to_str(key: &Key) -> String {
    match key {
        Key::Cmd => "Cmd".into(),
        Key::Ctrl => "Ctrl".into(),
        Key::Alt => "Alt".into(),
        Key::Shift => "Shift".into(),
        Key::Char(c) => c.to_string(),
    }
}

fn display_mode_to_str(mode: DisplayMode) -> String {
    match mode {
        DisplayMode::Text => "text".into(),
        DisplayMode::Icon => "icon".into(),
        DisplayMode::IndexIcon => "index_icon".into(),
    }
}

pub fn keys_to_hotkey_string(keys: &[Key]) -> String {
    keys.iter().map(key_to_str).collect::<Vec<_>>().join("+")
}

pub fn hotkey_string_to_keys(spec: &str) -> Vec<Key> {
    super::toml_model::parse_keys(
        &spec
            .split('+')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
    )
}
