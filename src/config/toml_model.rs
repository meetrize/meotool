use serde::{Deserialize, Serialize};

use super::app_config::{
    AppConfig, ContextMode, GestureConfig, GlobalHotkeys, GlobalPanelDefaults, PanelSettings,
    Profile, ThemeSettings,
};
use super::model::{
    Action, ActionScript, ButtonConfig, ButtonScope, CoordSpace, DisplayMode, Key, MacroStep,
    MatchRule, MouseButtonKind, MouseMacro, Position,
};

/// 磁盘上的完整配置
#[derive(Debug, Deserialize, Serialize)]
pub struct ConfigFile {
    /// Schema 版本；缺省为 0（旧配置）
    #[serde(default)]
    pub version: u32,
    /// 最后写入时间（Unix 秒，UTC）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub global: GlobalConfig,
    #[serde(default)]
    pub profiles: Vec<ProfileFile>,
    #[serde(default = "default_poll_interval")]
    pub window_poll_interval_ms: u64,
    /// 上下文模式：profile（默认）或 button_filter
    #[serde(
        default = "default_context_mode",
        skip_serializing_if = "is_default_context_mode"
    )]
    pub context_mode: String,
}

fn default_poll_interval() -> u64 {
    200
}

fn default_context_mode() -> String {
    ContextMode::Profile.as_str().to_string()
}

fn is_default_context_mode(mode: &str) -> bool {
    ContextMode::parse(mode) == ContextMode::Profile
}

impl Default for ConfigFile {
    fn default() -> Self {
        Self {
            version: 0,
            updated_at: None,
            global: GlobalConfig::default(),
            profiles: Vec::new(),
            window_poll_interval_ms: default_poll_interval(),
            context_mode: default_context_mode(),
        }
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct GlobalConfig {
    #[serde(default)]
    pub panel: PanelSettingsFile,
    #[serde(default)]
    pub gesture: GestureSettingsFile,
    pub toggle_visibility: Option<String>,
    pub reload_config: Option<String>,
    pub quit: Option<String>,
    pub open_settings: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct GestureSettingsFile {
    pub enabled: Option<bool>,
    pub drag_threshold_px: Option<f32>,
    pub dismiss_on_outside_click: Option<bool>,
    pub hide_after_button: Option<bool>,
    pub global_priority: Option<bool>,
}

impl GestureSettingsFile {
    fn to_gesture_config(&self) -> GestureConfig {
        let defaults = GestureConfig::default();
        GestureConfig {
            enabled: self.enabled.unwrap_or(defaults.enabled),
            drag_threshold_px: self.drag_threshold_px.unwrap_or(defaults.drag_threshold_px),
            dismiss_on_outside_click: self
                .dismiss_on_outside_click
                .unwrap_or(defaults.dismiss_on_outside_click),
            hide_after_button: self.hide_after_button.unwrap_or(defaults.hide_after_button),
            global_priority: self.global_priority.unwrap_or(defaults.global_priority),
        }
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct ProfileFile {
    pub name: String,
    #[serde(default)]
    pub r#match: Option<MatchRuleFile>,
    #[serde(default)]
    pub panel: PanelSettingsFile,
    #[serde(default)]
    pub buttons: Vec<ButtonFile>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct PanelSettingsFile {
    pub rows: Option<u8>,
    pub cols: Option<u8>,
    pub display_mode: Option<String>,
    pub collapsed: Option<bool>,
    pub opacity: Option<f32>,
    /// 兼容旧配置：同时设置宽高
    pub button_size: Option<f32>,
    pub button_width: Option<f32>,
    pub button_height: Option<f32>,
    pub gap: Option<f32>,
    pub collapsed_icon: Option<String>,
    pub position: Option<PositionSettings>,
    #[serde(default)]
    pub theme: ThemeSettingsFile,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct ThemeSettingsFile {
    pub background_color: Option<String>,
    pub button_color: Option<String>,
    pub letter_background_color: Option<String>,
    pub letter_color: Option<String>,
    pub letter_size: Option<f32>,
    pub letter_font: Option<String>,
    pub window_border_enabled: Option<bool>,
    pub window_border_width: Option<f32>,
    pub window_border_color: Option<String>,
    pub button_border_enabled: Option<bool>,
    pub button_border_width: Option<f32>,
    pub button_border_color: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PositionSettings {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct MatchRuleFile {
    pub bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub title_regex: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ButtonScopeFile {
    #[default]
    Always,
    WhenActive {
        bundle_id: Option<String>,
        app_name: Option<String>,
        title_regex: Option<String>,
    },
}

fn is_always_scope(scope: &ButtonScopeFile) -> bool {
    matches!(scope, ButtonScopeFile::Always)
}

impl ButtonScopeFile {
    fn into_scope(self) -> Result<ButtonScope, String> {
        match self {
            ButtonScopeFile::Always => Ok(ButtonScope::Always),
            ButtonScopeFile::WhenActive {
                bundle_id,
                app_name,
                title_regex,
            } => Ok(ButtonScope::WhenActive(
                MatchRuleFile {
                    bundle_id,
                    app_name,
                    title_regex,
                }
                .into_match_rule()?,
            )),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ButtonFile {
    pub label: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub action: ActionFile,
    #[serde(default)]
    pub background_color: Option<String>,
    #[serde(default)]
    pub border_color: Option<String>,
    #[serde(default)]
    pub icon_color: Option<String>,
    #[serde(default, skip_serializing_if = "is_always_scope")]
    pub scope: ButtonScopeFile,
}

fn default_enabled() -> bool {
    true
}

fn default_pre_delay_ms() -> u64 {
    MouseMacro::DEFAULT_PRE_DELAY_MS
}

fn default_mouse_button() -> String {
    "left".into()
}

fn default_coord_space() -> String {
    "window_normalized".into()
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MacroStepFile {
    Click {
        #[serde(default = "default_mouse_button")]
        button: String,
        x: f32,
        y: f32,
        #[serde(default = "default_coord_space")]
        coord_space: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    Hotkey {
        keys: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    ActivateApp {
        bundle_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    Delay {
        ms: u64,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActionFile {
    Hotkey {
        keys: Vec<String>,
    },
    OpenApp {
        target: String,
    },
    Shell {
        command: String,
    },
    OpenUrl {
        url: String,
    },
    OpenFile {
        path: String,
    },
    PasteText {
        text: String,
    },
    MouseMacro {
        #[serde(default)]
        steps: Vec<MacroStepFile>,
        #[serde(default = "default_pre_delay_ms")]
        pre_delay_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recorded_bundle_id: Option<String>,
    },
    Script {
        source: String,
    },
    None,
}

impl ConfigFile {
    pub fn into_app_config(self) -> Result<AppConfig, String> {
        if self.profiles.is_empty() {
            return Err("config has no profiles".into());
        }

        let global_position = self
            .global
            .panel
            .position
            .clone()
            .map(|p| Position { x: p.x, y: p.y })
            .unwrap_or(Position { x: 100.0, y: 200.0 });

        let global_settings = self.global.panel.to_panel_settings();

        let profiles = self
            .profiles
            .into_iter()
            .map(|p| p.into_profile())
            .collect::<Result<Vec<_>, _>>()?;

        Ok(AppConfig {
            global_panel: GlobalPanelDefaults {
                settings: global_settings,
                position: global_position,
            },
            hotkeys: GlobalHotkeys {
                toggle_visibility: self
                    .global
                    .toggle_visibility
                    .unwrap_or_else(|| "Cmd+Shift+P".into()),
                reload_config: self
                    .global
                    .reload_config
                    .unwrap_or_else(|| "Cmd+Shift+R".into()),
                quit: self.global.quit.unwrap_or_else(|| "Cmd+Shift+Q".into()),
                open_settings: self
                    .global
                    .open_settings
                    .unwrap_or_else(|| "Cmd+Shift+,".into()),
            },
            gesture: self.global.gesture.to_gesture_config(),
            profiles,
            poll_interval_ms: self.window_poll_interval_ms,
            context_mode: ContextMode::parse(&self.context_mode),
        })
    }
}

impl ProfileFile {
    fn into_profile(self) -> Result<Profile, String> {
        let buttons = self
            .buttons
            .into_iter()
            .map(ButtonFile::into_button_config)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Profile {
            name: self.name,
            match_rule: self
                .r#match
                .map(MatchRuleFile::into_match_rule)
                .transpose()?,
            panel: self.panel.to_panel_settings(),
            buttons,
        })
    }
}

impl MatchRuleFile {
    fn into_match_rule(self) -> Result<MatchRule, String> {
        let title_regex_text = self.title_regex.clone();
        let title_regex = match self.title_regex {
            Some(pattern) => Some(
                regex::Regex::new(&pattern)
                    .map_err(|e| format!("invalid title_regex '{pattern}': {e}"))?,
            ),
            None => None,
        };
        Ok(MatchRule {
            bundle_id: self.bundle_id,
            app_name: self.app_name,
            title_regex,
            title_regex_text,
        })
    }
}

impl PanelSettingsFile {
    fn to_panel_settings(&self) -> PanelSettings {
        let legacy_collapsed = self
            .display_mode
            .as_deref()
            .is_some_and(|s| s.eq_ignore_ascii_case("collapsed"));
        let display_mode = self.display_mode.as_deref().map(|s| {
            if legacy_collapsed {
                DisplayMode::IndexIcon
            } else {
                parse_display_mode(s)
            }
        });
        PanelSettings {
            rows: self.rows,
            cols: self.cols,
            display_mode,
            collapsed: self.collapsed.or_else(|| legacy_collapsed.then_some(true)),
            opacity: self.opacity,
            button_width: self.button_width.or(self.button_size),
            button_height: self.button_height.or(self.button_size),
            gap: self.gap,
            collapsed_icon: self.collapsed_icon.clone(),
            theme: self.theme.to_theme_settings(),
        }
    }
}

impl ThemeSettingsFile {
    fn to_theme_settings(&self) -> ThemeSettings {
        ThemeSettings {
            background_color: self.background_color.clone(),
            button_color: self.button_color.clone(),
            letter_background_color: self.letter_background_color.clone(),
            letter_color: self.letter_color.clone(),
            letter_size: self.letter_size,
            letter_font: self.letter_font.clone(),
            window_border_enabled: self.window_border_enabled,
            window_border_width: self.window_border_width,
            window_border_color: self.window_border_color.clone(),
            button_border_enabled: self.button_border_enabled,
            button_border_width: self.button_border_width,
            button_border_color: self.button_border_color.clone(),
        }
    }
}

impl ButtonFile {
    fn into_button_config(self) -> Result<ButtonConfig, String> {
        Ok(ButtonConfig {
            label: self.label,
            icon: self.icon,
            action: self.action.to_action(),
            enabled: self.enabled,
            background_color: self.background_color,
            border_color: self.border_color,
            icon_color: self.icon_color,
            scope: self.scope.into_scope()?,
        })
    }
}

impl ActionFile {
    fn to_action(&self) -> Action {
        match self {
            ActionFile::Hotkey { keys } => Action::Hotkey(parse_keys(keys)),
            ActionFile::OpenApp { target } => Action::OpenApp {
                target: target.clone(),
            },
            ActionFile::Shell { command } => Action::Shell {
                command: command.clone(),
            },
            ActionFile::OpenUrl { url } => Action::OpenUrl { url: url.clone() },
            ActionFile::OpenFile { path } => Action::OpenFile { path: path.clone() },
            ActionFile::PasteText { text } => Action::PasteText { text: text.clone() },
            ActionFile::MouseMacro {
                steps,
                pre_delay_ms,
                recorded_bundle_id,
            } => Action::MouseMacro(MouseMacro {
                steps: steps.iter().filter_map(macro_step_from_file).collect(),
                pre_delay_ms: *pre_delay_ms,
                recorded_bundle_id: recorded_bundle_id.clone(),
            }),
            ActionFile::Script { source } => {
                let program = match crate::action::compile(source, None) {
                    Ok(p) => Some(p),
                    Err(err) => {
                        log::warn!(
                            "action script compile failed on load: {}",
                            err.primary_display()
                        );
                        None
                    }
                };
                Action::Script(ActionScript {
                    source: source.clone(),
                    program,
                    attached_macro: None,
                })
            }
            ActionFile::None => Action::None,
        }
    }
}

fn macro_step_from_file(step: &MacroStepFile) -> Option<MacroStep> {
    match step {
        MacroStepFile::Click {
            button,
            x,
            y,
            coord_space,
            note,
        } => Some(MacroStep::Click {
            button: parse_mouse_button(button),
            x: *x,
            y: *y,
            coord_space: parse_coord_space(coord_space),
            note: note.clone(),
        }),
        MacroStepFile::Hotkey { keys, note } => {
            let parsed = parse_keys(keys);
            if parsed.is_empty() {
                None
            } else {
                Some(MacroStep::Hotkey {
                    keys: parsed,
                    note: note.clone(),
                })
            }
        }
        MacroStepFile::ActivateApp { bundle_id, note } => {
            let id = bundle_id.trim();
            if id.is_empty() {
                None
            } else {
                Some(MacroStep::ActivateApp {
                    bundle_id: id.to_string(),
                    note: note.clone(),
                })
            }
        }
        MacroStepFile::Delay { ms } => Some(MacroStep::Delay { ms: *ms }),
    }
}

fn parse_mouse_button(s: &str) -> MouseButtonKind {
    match s.to_lowercase().as_str() {
        "right" => MouseButtonKind::Right,
        "middle" => MouseButtonKind::Middle,
        _ => MouseButtonKind::Left,
    }
}

fn parse_coord_space(s: &str) -> CoordSpace {
    match s.to_lowercase().as_str() {
        "screen_abs" | "screen" | "absolute" => CoordSpace::ScreenAbs,
        _ => CoordSpace::WindowNormalized,
    }
}

pub fn mouse_button_to_str(button: MouseButtonKind) -> &'static str {
    match button {
        MouseButtonKind::Left => "left",
        MouseButtonKind::Right => "right",
        MouseButtonKind::Middle => "middle",
    }
}

pub fn coord_space_to_str(space: CoordSpace) -> &'static str {
    match space {
        CoordSpace::WindowNormalized => "window_normalized",
        CoordSpace::ScreenAbs => "screen_abs",
    }
}

pub fn parse_display_mode(s: &str) -> DisplayMode {
    match s.to_lowercase().as_str() {
        "icon" => DisplayMode::Icon,
        "index_icon" | "indexicon" => DisplayMode::IndexIcon,
        "collapsed" => DisplayMode::IndexIcon,
        _ => DisplayMode::Text,
    }
}

pub fn parse_keys(keys: &[String]) -> Vec<Key> {
    keys.iter().filter_map(|k| parse_key(k)).collect()
}

fn parse_key(s: &str) -> Option<Key> {
    match s.to_lowercase().as_str() {
        "cmd" | "command" | "meta" | "super" => Some(Key::Cmd),
        "ctrl" | "control" => Some(Key::Ctrl),
        "alt" | "option" => Some(Key::Alt),
        "shift" => Some(Key::Shift),
        other if other.len() == 1 => {
            let c = other.chars().next()?;
            Some(Key::Char(c))
        }
        other if other.starts_with('f') && other.len() <= 3 => other.chars().last().map(Key::Char),
        _ => {
            log::warn!("unknown key name: {s}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hotkey_keys() {
        let keys = parse_keys(&["Cmd".into(), "Shift".into(), "4".into()]);
        assert_eq!(keys, vec![Key::Cmd, Key::Shift, Key::Char('4')]);
    }

    #[test]
    fn parse_open_file_action() {
        let action: ActionFile = toml::from_str(
            r#"
type = "open_file"
path = "~/Documents/report.pdf"
"#,
        )
        .expect("parse open_file");
        match action.to_action() {
            Action::OpenFile { path } => {
                assert_eq!(path, "~/Documents/report.pdf");
            }
            other => panic!("expected OpenFile, got {other:?}"),
        }
    }

    #[test]
    fn parse_paste_text_action() {
        let action: ActionFile = toml::from_str(
            r#"
type = "paste_text"
text = "ssh root@10.0.0.1 -p 22"
"#,
        )
        .expect("parse paste_text");
        match action.to_action() {
            Action::PasteText { text } => {
                assert_eq!(text, "ssh root@10.0.0.1 -p 22");
            }
            other => panic!("expected PasteText, got {other:?}"),
        }
    }

    #[test]
    fn parse_mouse_macro_with_hotkey_and_activate() {
        let action: ActionFile = toml::from_str(
            r#"
type = "mouse_macro"
pre_delay_ms = 80

[[steps]]
type = "activate_app"
bundle_id = "com.apple.Safari"

[[steps]]
type = "click"
button = "left"
x = 0.5
y = 0.25
coord_space = "window_normalized"

[[steps]]
type = "hotkey"
keys = ["Cmd", "s"]
"#,
        )
        .expect("parse mouse_macro hybrid");
        match action.to_action() {
            Action::MouseMacro(m) => {
                assert_eq!(m.steps.len(), 3);
                assert!(matches!(m.steps[0], MacroStep::ActivateApp { .. }));
                assert!(matches!(m.steps[1], MacroStep::Click { .. }));
                assert!(matches!(m.steps[2], MacroStep::Hotkey { .. }));
            }
            other => panic!("expected MouseMacro, got {other:?}"),
        }
    }

    #[test]
    fn legacy_config_without_scope_defaults_always() {
        let file: ConfigFile = toml::from_str(
            r#"
[[profiles]]
name = "default"

[[profiles.buttons]]
label = "复制"
enabled = true
[profiles.buttons.action]
type = "none"
"#,
        )
        .expect("parse legacy config");
        let app = file.into_app_config().expect("into app");
        assert_eq!(app.context_mode, ContextMode::Profile);
        assert!(matches!(
            app.profiles[0].buttons[0].scope,
            ButtonScope::Always
        ));
    }

    #[test]
    fn button_filter_scope_round_trip() {
        let file: ConfigFile = toml::from_str(
            r#"
context_mode = "button_filter"

[[profiles]]
name = "default"

[[profiles.buttons]]
label = "格式化"
[profiles.buttons.scope]
type = "when_active"
bundle_id = "com.microsoft.VSCode"
[profiles.buttons.action]
type = "none"
"#,
        )
        .expect("parse scoped config");
        let app = file.into_app_config().expect("into app");
        assert_eq!(app.context_mode, ContextMode::ButtonFilter);
        match &app.profiles[0].buttons[0].scope {
            ButtonScope::WhenActive(rule) => {
                assert_eq!(rule.bundle_id.as_deref(), Some("com.microsoft.VSCode"));
            }
            other => panic!("expected WhenActive, got {other:?}"),
        }

        let exported = crate::config::export::app_config_to_file(&app);
        assert_eq!(exported.context_mode, "button_filter");
        assert!(matches!(
            exported.profiles[0].buttons[0].scope,
            ButtonScopeFile::WhenActive { .. }
        ));
        let reloaded = exported.into_app_config().expect("reload");
        assert_eq!(reloaded.context_mode, ContextMode::ButtonFilter);
    }

    #[test]
    fn script_action_toml_round_trip() {
        let action: ActionFile = toml::from_str(
            r#"
type = "script"
source = """
输入 "hello"
等待 100
按键 Enter
"""
"#,
        )
        .expect("parse script");
        let runtime = action.to_action();
        match &runtime {
            Action::Script(s) => {
                assert!(s.source.contains("输入 \"hello\""));
                assert!(s.source.contains("等待 100"));
                assert!(s.source.contains("按键 Enter"));
                assert!(s.program.as_ref().is_some_and(|p| p.steps.len() == 3));
            }
            other => panic!("expected Script, got {other:?}"),
        }

        let file = crate::config::export::app_config_to_file(&AppConfig {
            global_panel: GlobalPanelDefaults {
                settings: PanelSettings::default(),
                position: Position { x: 0.0, y: 0.0 },
            },
            hotkeys: GlobalHotkeys::default(),
            gesture: Default::default(),
            context_mode: ContextMode::Profile,
            poll_interval_ms: 200,
            profiles: vec![Profile {
                name: "default".into(),
                match_rule: None,
                panel: PanelSettings::default(),
                buttons: vec![ButtonConfig {
                    label: "脚本".into(),
                    icon: None,
                    action: runtime,
                    enabled: true,
                    background_color: None,
                    border_color: None,
                    icon_color: None,
                    scope: ButtonScope::Always,
                }],
            }],
        });

        let serialized = toml::to_string_pretty(&file).expect("serialize");
        assert!(serialized.contains("type = \"script\""));
        assert!(serialized.contains("输入 \"hello\""));

        let reloaded: ConfigFile = toml::from_str(&serialized).expect("reparse");
        let app = reloaded.into_app_config().expect("into app");
        match &app.profiles[0].buttons[0].action {
            Action::Script(s) => {
                assert!(s.source.contains("输入 \"hello\""));
                assert!(s.source.contains("等待 100"));
                assert!(s.program.as_ref().is_some_and(|p| p.steps.len() == 3));
            }
            other => panic!("expected Script after round-trip, got {other:?}"),
        }
    }
}
