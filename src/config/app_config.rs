use crate::config::color::parse_color;
use crate::config::model::{ButtonConfig, DisplayMode, PanelConfig, Position, UiStyle};

pub use crate::config::model::{MatchRule, WindowInfo};

/// 上下文模式：Profile 整页切换，或单池按钮过滤
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContextMode {
    #[default]
    Profile,
    ButtonFilter,
}

impl ContextMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ContextMode::Profile => "profile",
            ContextMode::ButtonFilter => "button_filter",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "button_filter" | "button-filter" | "filter" => ContextMode::ButtonFilter,
            _ => ContextMode::Profile,
        }
    }
}

/// 完整应用配置（含多 Profile）
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub global_panel: GlobalPanelDefaults,
    pub hotkeys: GlobalHotkeys,
    pub gesture: GestureConfig,
    pub profiles: Vec<Profile>,
    pub poll_interval_ms: u64,
    /// 上下文模式，默认 Profile（现有行为）
    pub context_mode: ContextMode,
}

/// 全局鼠标手势：右键按住拖动 / 中键点击唤起面板
#[derive(Debug, Clone)]
pub struct GestureConfig {
    pub enabled: bool,
    /// 中键单击在光标位置唤起面板
    pub middle_click_enabled: bool,
    pub drag_threshold_px: f32,
    pub dismiss_on_outside_click: bool,
    pub hide_after_button: bool,
    /// 全局优先：后台可唤起，拖动时拦截右键事件避免被其他应用处理
    pub global_priority: bool,
}

impl Default for GestureConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            middle_click_enabled: false,
            drag_threshold_px: 8.0,
            dismiss_on_outside_click: true,
            hide_after_button: true,
            global_priority: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GlobalHotkeys {
    pub toggle_visibility: String,
    pub reload_config: String,
    pub quit: String,
    pub open_settings: String,
}

impl Default for GlobalHotkeys {
    fn default() -> Self {
        Self {
            toggle_visibility: "Cmd+Shift+P".into(),
            reload_config: "Cmd+Shift+R".into(),
            quit: "Cmd+Shift+Q".into(),
            open_settings: "Cmd+Shift+,".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GlobalPanelDefaults {
    pub settings: PanelSettings,
    pub position: Position,
}

#[derive(Debug, Clone, Default)]
pub struct PanelSettings {
    pub rows: Option<u8>,
    pub cols: Option<u8>,
    pub display_mode: Option<DisplayMode>,
    pub collapsed: Option<bool>,
    pub opacity: Option<f32>,
    pub button_width: Option<f32>,
    pub button_height: Option<f32>,
    /// 按钮左右间距
    pub gap_x: Option<f32>,
    /// 按钮上下间距
    pub gap_y: Option<f32>,
    pub collapsed_icon: Option<String>,
    pub theme: ThemeSettings,
}

/// 界面配色（全局默认可被 Profile 覆盖）
#[derive(Debug, Clone, Default)]
pub struct ThemeSettings {
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

impl ThemeSettings {
    pub fn merge(base: &ThemeSettings, override_: &ThemeSettings) -> ThemeSettings {
        ThemeSettings {
            background_color: override_
                .background_color
                .clone()
                .or_else(|| base.background_color.clone()),
            button_color: override_
                .button_color
                .clone()
                .or_else(|| base.button_color.clone()),
            letter_background_color: override_
                .letter_background_color
                .clone()
                .or_else(|| base.letter_background_color.clone()),
            letter_color: override_
                .letter_color
                .clone()
                .or_else(|| base.letter_color.clone()),
            letter_size: override_.letter_size.or(base.letter_size),
            letter_font: override_
                .letter_font
                .clone()
                .or_else(|| base.letter_font.clone()),
            window_border_enabled: override_
                .window_border_enabled
                .or(base.window_border_enabled),
            window_border_width: override_.window_border_width.or(base.window_border_width),
            window_border_color: override_
                .window_border_color
                .clone()
                .or_else(|| base.window_border_color.clone()),
            button_border_enabled: override_
                .button_border_enabled
                .or(base.button_border_enabled),
            button_border_width: override_.button_border_width.or(base.button_border_width),
            button_border_color: override_
                .button_border_color
                .clone()
                .or_else(|| base.button_border_color.clone()),
        }
    }

    pub fn resolve(&self) -> UiStyle {
        let defaults = UiStyle::default();
        UiStyle {
            background: parse_color_opt(self.background_color.as_deref())
                .unwrap_or(defaults.background),
            button_bg: parse_color_opt(self.button_color.as_deref()).unwrap_or(defaults.button_bg),
            letter_bg: parse_color_opt(self.letter_background_color.as_deref())
                .unwrap_or(defaults.letter_bg),
            letter_color: parse_color_opt(self.letter_color.as_deref())
                .unwrap_or(defaults.letter_color),
            letter_size: self.letter_size.unwrap_or(defaults.letter_size),
            letter_font: self.letter_font.clone().filter(|s| !s.trim().is_empty()),
            window_border_enabled: self
                .window_border_enabled
                .unwrap_or(defaults.window_border_enabled),
            window_border_width: self
                .window_border_width
                .unwrap_or(defaults.window_border_width),
            window_border_color: parse_color_opt(self.window_border_color.as_deref())
                .unwrap_or(defaults.window_border_color),
            button_border_enabled: self
                .button_border_enabled
                .unwrap_or(defaults.button_border_enabled),
            button_border_width: self
                .button_border_width
                .unwrap_or(defaults.button_border_width),
            button_border_color: parse_color_opt(self.button_border_color.as_deref())
                .unwrap_or(defaults.button_border_color),
        }
    }
}

fn parse_color_opt(s: Option<&str>) -> Option<crate::config::color::RgbColor> {
    s.filter(|s| !s.trim().is_empty()).and_then(parse_color)
}

const DEFAULT_BUTTON_SIZE: f32 = 64.0;

fn resolve_button_width(profile: &PanelSettings, global: &PanelSettings) -> f32 {
    profile
        .button_width
        .or(global.button_width)
        .unwrap_or(DEFAULT_BUTTON_SIZE)
}

fn resolve_button_height(profile: &PanelSettings, global: &PanelSettings) -> f32 {
    profile
        .button_height
        .or(global.button_height)
        .unwrap_or(DEFAULT_BUTTON_SIZE)
}

#[derive(Debug, Clone)]
pub struct Profile {
    pub name: String,
    pub match_rule: Option<MatchRule>,
    pub panel: PanelSettings,
    pub buttons: Vec<ButtonConfig>,
}

impl AppConfig {
    pub fn default_profile_index(&self) -> usize {
        self.profiles
            .iter()
            .position(|p| p.name == "default")
            .unwrap_or(0)
    }

    pub fn build_panel(&self, profile: &Profile) -> PanelConfig {
        let g = &self.global_panel.settings;
        let p = &profile.panel;

        let theme = ThemeSettings::merge(&g.theme, &p.theme).resolve();

        PanelConfig {
            rows: p.rows.or(g.rows).unwrap_or(3),
            cols: p.cols.or(g.cols).unwrap_or(3),
            display_mode: p
                .display_mode
                .or(g.display_mode)
                .unwrap_or(DisplayMode::Text),
            collapsed: p.collapsed.or(g.collapsed).unwrap_or(false),
            opacity: p.opacity.or(g.opacity).unwrap_or(0.85),
            button_width: resolve_button_width(p, g),
            button_height: resolve_button_height(p, g),
            gap_x: p.gap_x.or(g.gap_x).unwrap_or(4.0),
            gap_y: p.gap_y.or(g.gap_y).unwrap_or(4.0),
            position: self.global_panel.position.clone(),
            collapsed_icon: p
                .collapsed_icon
                .clone()
                .filter(|s| !s.trim().is_empty())
                .or_else(|| g.collapsed_icon.clone().filter(|s| !s.trim().is_empty())),
            buttons: profile.buttons.clone(),
            ui_style: theme,
        }
    }
}
