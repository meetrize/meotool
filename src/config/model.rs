#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    Text,
    Icon,
    IndexIcon,
}

#[derive(Debug, Clone)]
pub struct Position {
    pub x: f32,
    pub y: f32,
}

use crate::config::color::RgbColor;

#[derive(Debug, Clone)]
pub struct UiStyle {
    pub background: RgbColor,
    pub button_bg: RgbColor,
    pub letter_bg: RgbColor,
    pub letter_color: RgbColor,
    pub letter_size: f32,
    /// 自定义字母字体路径；原生 UI 暂未读取，配置仍往返保存
    #[allow(dead_code)]
    pub letter_font: Option<String>,
    pub window_border_enabled: bool,
    pub window_border_width: f32,
    pub window_border_color: RgbColor,
    pub button_border_enabled: bool,
    pub button_border_width: f32,
    pub button_border_color: RgbColor,
    /// 按钮圆角；0 为直角
    pub button_corner_radius: f32,
}

impl Default for UiStyle {
    fn default() -> Self {
        Self {
            background: RgbColor::rgba(0, 0, 0, 217),
            button_bg: RgbColor::rgba(40, 40, 40, 217),
            letter_bg: RgbColor::rgba(0, 0, 0, 153),
            letter_color: RgbColor::rgb(224, 224, 224),
            letter_size: 11.0,
            letter_font: None,
            window_border_enabled: false,
            window_border_width: 1.0,
            window_border_color: RgbColor::rgba(255, 255, 255, 38),
            button_border_enabled: false,
            button_border_width: 1.0,
            button_border_color: RgbColor::rgba(255, 255, 255, 38),
            button_corner_radius: 8.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PanelConfig {
    pub rows: u8,
    pub cols: u8,
    pub display_mode: DisplayMode,
    pub collapsed: bool,
    pub opacity: f32,
    pub button_width: f32,
    pub button_height: f32,
    /// 按钮左右间距（列间距）
    pub gap_x: f32,
    /// 按钮上下间距（行间距）
    pub gap_y: f32,
    pub position: Position,
    pub collapsed_icon: Option<String>,
    pub buttons: Vec<ButtonConfig>,
    pub ui_style: UiStyle,
}

/// 按钮可见范围：始终显示，或仅当前台窗口匹配时显示
#[derive(Debug, Clone, Default)]
pub enum ButtonScope {
    #[default]
    Always,
    WhenActive(MatchRule),
}

impl ButtonScope {
    pub fn is_visible(&self, window: Option<&WindowInfo>) -> bool {
        match self {
            ButtonScope::Always => true,
            ButtonScope::WhenActive(rule) => window.is_some_and(|w| rule.matches(w)),
        }
    }
}

/// Profile / 按钮 scope 共用的前台窗口匹配规则
#[derive(Debug, Clone)]
pub struct MatchRule {
    pub bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub title_regex: Option<regex::Regex>,
    /// 原始正则文本，用于配置导出
    pub title_regex_text: Option<String>,
}

impl MatchRule {
    pub fn matches(&self, window: &WindowInfo) -> bool {
        if let Some(bundle_id) = &self.bundle_id {
            if window.bundle_id.as_deref() == Some(bundle_id.as_str()) {
                return true;
            }
        }

        if let Some(re) = &self.title_regex {
            if !window.title.is_empty() && re.is_match(&window.title) {
                return true;
            }
        }

        if let Some(app_name) = &self.app_name {
            if window.app_name.eq_ignore_ascii_case(app_name) {
                return true;
            }
        }

        false
    }
}

/// 前台窗口信息
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub app_name: String,
    pub bundle_id: Option<String>,
    pub title: String,
    pub process_id: u64,
    pub process_path: std::path::PathBuf,
    /// 浏览器当前页面地址（如 Safari 前台标签页 URL）
    pub page_url: Option<String>,
}

impl WindowInfo {
    pub fn identity_key(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.bundle_id.as_deref().unwrap_or(""),
            self.app_name,
            self.title,
            self.process_id
        )
    }

    #[cfg(any(feature = "ui-egui", test))]
    pub fn display_name(&self) -> String {
        if !self.app_name.is_empty() {
            self.app_name.clone()
        } else if let Some(id) = &self.bundle_id {
            id.clone()
        } else {
            self.title.clone()
        }
    }

    pub fn is_own_process(&self) -> bool {
        self.process_id as u32 == std::process::id()
    }
}

#[derive(Debug, Clone)]
pub struct ButtonConfig {
    pub label: String,
    pub icon: Option<String>,
    pub action: Action,
    pub enabled: bool,
    /// 覆盖全局主题；None 表示继承
    pub background_color: Option<String>,
    pub border_color: Option<String>,
    pub icon_color: Option<String>,
    /// 可见范围，默认始终显示
    pub scope: ButtonScope,
    /// 全局快捷键（如 `Cmd+Shift+1`）；按下后执行本按钮动作。空 / None 表示未设置
    pub hotkey: Option<String>,
    /// 是否出现在菜单栏托盘下拉菜单（还需全局 `tray_show_buttons`）
    pub show_in_tray_menu: bool,
}

/// 空槽位占位（不可见 / 未配置）
pub fn empty_slot_button() -> ButtonConfig {
    ButtonConfig {
        label: String::new(),
        icon: None,
        action: Action::None,
        enabled: false,
        background_color: None,
        border_color: None,
        icon_color: None,
        scope: ButtonScope::Always,
        hotkey: None,
        show_in_tray_menu: false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButtonKind {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordSpace {
    /// 相对目标窗口内容区，归一化 0.0~1.0
    WindowNormalized,
    /// 屏幕绝对坐标（兜底）
    ScreenAbs,
}

#[derive(Debug, Clone)]
pub enum MacroStep {
    Click {
        button: MouseButtonKind,
        x: f32,
        y: f32,
        coord_space: CoordSpace,
        note: Option<String>,
    },
    /// 模拟快捷键（修饰键 + 主键）
    Hotkey {
        keys: Vec<Key>,
        note: Option<String>,
    },
    /// 激活目标应用（按 Bundle ID）
    ActivateApp {
        bundle_id: String,
        note: Option<String>,
    },
    Delay {
        ms: u64,
    },
}

#[derive(Debug, Clone)]
pub struct MouseMacro {
    pub steps: Vec<MacroStep>,
    pub pre_delay_ms: u64,
    pub recorded_bundle_id: Option<String>,
}

impl MouseMacro {
    pub const DEFAULT_PRE_DELAY_MS: u64 = 80;
    pub const MAX_STEPS: usize = 50;
    pub const MAX_TOTAL_MS: u64 = 10_000;

    pub fn is_configured(&self) -> bool {
        !self.steps.is_empty()
    }
}

/// 动作脚本专用单键（Enter/Tab 等）；与快捷键修饰 `Key` 区分
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialKey {
    Enter,
    Tab,
    Escape,
    Space,
    Delete,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

/// MeoFlow 编译后的单步 IR（比 MacroStep 更完整）
#[derive(Debug, Clone)]
pub enum FlowStep {
    Delay {
        ms: u64,
    },
    TypeText {
        text: String,
    },
    PasteText {
        text: String,
    },
    Key {
        key: SpecialKey,
    },
    Hotkey {
        keys: Vec<Key>,
    },
    Click {
        button: MouseButtonKind,
        x: f32,
        y: f32,
        coord_space: CoordSpace,
        /// 1 = 单击，2 = 双击
        clicks: u8,
    },
    OpenApp {
        target: String,
    },
    OpenUrl {
        url: String,
    },
    OpenFile {
        path: String,
    },
    Shell {
        command: String,
    },
    ActivateApp {
        bundle_id: String,
    },
    /// 播放内嵌鼠标宏（由「播放宏」或宏迁移产生）
    PlayMouseMacro {
        macro_: MouseMacro,
    },
}

#[derive(Debug, Clone)]
pub struct FlowProgram {
    pub steps: Vec<FlowStep>,
    pub estimated_ms: u64,
}

impl FlowProgram {
    pub const MAX_LINES: usize = 200;
    pub const MAX_STEPS: usize = 80;
    pub const MAX_TOTAL_MS: u64 = 30_000;
    pub const MAX_WAIT_MS: u64 = 10_000;
    pub const SHELL_TIMEOUT_MS: u64 = 5_000;
    pub const MAX_SOURCE_BYTES: usize = 32 * 1024;
}

#[derive(Debug, Clone)]
pub struct ActionScript {
    pub source: String,
    /// 保存/加载成功后缓存；以 source 为准可重建
    pub program: Option<FlowProgram>,
    /// 「播放宏」可引用的内嵌宏（通常来自鼠标宏迁移；不强制写入 TOML）
    pub attached_macro: Option<MouseMacro>,
}

impl ActionScript {
    #[allow(dead_code)]
    pub fn from_source(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            program: None,
            attached_macro: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Action {
    Hotkey(Vec<Key>),
    /// 打开应用：支持 .app 路径、Bundle ID 或应用名称（如 Terminal）
    OpenApp {
        target: String,
    },
    /// 执行 Shell 命令（通过 sh -c）
    Shell {
        command: String,
    },
    /// 打开 URL（浏览器、文件协议等）
    OpenUrl {
        url: String,
    },
    /// 用系统默认应用打开本地文件或文件夹
    OpenFile {
        path: String,
    },
    /// 将预设文本写入剪贴板并模拟 Cmd+V 粘贴到前台应用
    PasteText {
        text: String,
    },
    /// 鼠标宏：窗口相对坐标点击 / 短序列回放
    MouseMacro(MouseMacro),
    /// 动作脚本（MeoFlow）：源码 + 可选已编译 IR
    Script(ActionScript),
    None,
}

impl Action {
    pub fn is_configured(&self) -> bool {
        match self {
            Action::None => false,
            Action::MouseMacro(m) => m.is_configured(),
            Action::Script(s) => !s.source.trim().is_empty(),
            _ => true,
        }
    }

    /// 悬停预览用的动作正文（脚本优先展示源码）
    pub fn hover_body(&self) -> Option<String> {
        match self {
            Action::None => None,
            Action::Script(s) => {
                let src = s.source.trim();
                if src.is_empty() {
                    None
                } else {
                    Some(truncate_hover_text(src, HOVER_TOOLTIP_MAX_CHARS))
                }
            }
            Action::Hotkey(keys) => {
                let spec = crate::config::export::keys_to_hotkey_string(keys);
                if spec.is_empty() {
                    None
                } else {
                    Some(format!("快捷键 {spec}"))
                }
            }
            Action::OpenApp { target } => Some(format!("打开应用\n{target}")),
            Action::Shell { command } => Some(format!(
                "Shell\n{}",
                truncate_hover_text(command, HOVER_TOOLTIP_MAX_CHARS)
            )),
            Action::OpenUrl { url } => Some(format!("打开链接\n{url}")),
            Action::OpenFile { path } => Some(format!("打开文件\n{path}")),
            Action::PasteText { text } => Some(format!(
                "粘贴文本\n{}",
                truncate_hover_text(text, HOVER_TOOLTIP_MAX_CHARS)
            )),
            Action::MouseMacro(m) => Some(format!("鼠标宏（{} 步）", m.steps.len())),
        }
    }

    /// 用于面板指纹：脚本改内容后也能刷新 tooltip
    pub fn hover_fingerprint(&self) -> String {
        match self {
            Action::None => String::new(),
            Action::Script(s) => format!("script:{}", s.source),
            Action::Hotkey(keys) => {
                format!("hotkey:{}", crate::config::export::keys_to_hotkey_string(keys))
            }
            Action::OpenApp { target } => format!("app:{target}"),
            Action::Shell { command } => format!("shell:{command}"),
            Action::OpenUrl { url } => format!("url:{url}"),
            Action::OpenFile { path } => format!("file:{path}"),
            Action::PasteText { text } => format!("paste:{text}"),
            Action::MouseMacro(m) => format!("macro:{}", m.steps.len()),
        }
    }
}

/// 悬停 tooltip 最大字符数（按 Unicode 标量），避免超长脚本撑爆系统 tip
const HOVER_TOOLTIP_MAX_CHARS: usize = 1200;

fn truncate_hover_text(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max_chars).collect();
    format!("{truncated}…")
}

impl ButtonConfig {
    /// 鼠标悬停时展示的文案：标签 + 快捷键 + 脚本/动作内容
    pub fn hover_tooltip(&self) -> Option<String> {
        let label = self.label.trim();
        let body = self.action.hover_body();
        let hotkey = self
            .hotkey
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| format!("快捷键 {s}"));

        let mut parts: Vec<String> = Vec::new();
        if !label.is_empty() {
            parts.push(label.to_string());
        }
        if let Some(hk) = hotkey {
            parts.push(hk);
        }
        if let Some(body) = body {
            // 标签已单独展示时，避免正文首行重复标签
            let body = if !label.is_empty() && body.lines().next() == Some(label) {
                body.lines().skip(1).collect::<Vec<_>>().join("\n")
            } else {
                body
            };
            let body = body.trim();
            if !body.is_empty() {
                parts.push(body.to_string());
            }
        }

        if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n\n"))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Cmd,
    Ctrl,
    Alt,
    Shift,
    Char(char),
}

impl PanelConfig {
    /// 折叠桌宠显示宽高（接近 pet-cat.png 横版比例）
    pub const COLLAPSED_LAUNCHER_WIDTH: f32 = 104.0;
    pub const COLLAPSED_LAUNCHER_HEIGHT: f32 = 72.0;
    /// 抗锯齿与描边所需的额外边距（单侧），避免被窗口/裁剪矩形切边
    pub const COLLAPSED_LAUNCHER_BLEED: f32 = 4.0;
    /// 未配置时默认的折叠桌宠图标
    pub const DEFAULT_COLLAPSED_ICON: &'static str = "pet-cat.png";
    /// 顶栏（拖拽把手 + 设置按钮）
    pub const PANEL_HEADER_HEIGHT: f32 = 22.0;
    pub const PANEL_PADDING: f32 = 2.0;

    pub fn panel_size(&self) -> (f32, f32) {
        let w = self.cols as f32 * self.button_width + (self.cols as f32 - 1.0) * self.gap_x;
        let h = self.rows as f32 * self.button_height + (self.rows as f32 - 1.0) * self.gap_y;
        (w, h)
    }

    pub fn collapsed_launcher_alloc() -> (f32, f32) {
        (
            Self::COLLAPSED_LAUNCHER_WIDTH + Self::COLLAPSED_LAUNCHER_BLEED * 2.0,
            Self::COLLAPSED_LAUNCHER_HEIGHT + Self::COLLAPSED_LAUNCHER_BLEED * 2.0,
        )
    }

    /// 折叠模式窗口尺寸（含内边距）。
    /// 始终返回展开态尺寸：展开/收起只切换绘制，不改窗口大小，避免图标闪动。
    pub fn collapsed_viewport_size(&self, _expanded: bool) -> (f32, f32) {
        let pad = Self::PANEL_PADDING * 2.0;
        let (full_w, full_h) = self.panel_size();
        let (launcher_w, launcher_h) = Self::collapsed_launcher_alloc();
        (
            launcher_w.max(full_w) + pad,
            launcher_h + self.gap_y + full_h + pad,
        )
    }

    /// 含顶栏与内边距的窗口尺寸
    pub fn viewport_size(&self, collapsed_expanded: bool) -> (f32, f32) {
        if !self.collapsed {
            return self.viewport_size_for_grid(self.panel_size());
        }
        self.collapsed_viewport_size(collapsed_expanded)
    }

    pub fn viewport_size_for_grid(&self, grid_size: (f32, f32)) -> (f32, f32) {
        let pad = Self::PANEL_PADDING * 2.0;
        (
            grid_size.0 + pad,
            grid_size.1 + Self::PANEL_HEADER_HEIGHT + pad,
        )
    }

    pub fn slot_count(&self) -> usize {
        self.rows as usize * self.cols as usize
    }

    /// 按 rows×cols 补齐按钮槽位
    pub fn padded_buttons(&self) -> Vec<ButtonConfig> {
        let mut buttons = self.buttons.clone();
        let slots = self.slot_count();
        while buttons.len() < slots {
            buttons.push(empty_slot_button());
        }
        buttons.truncate(slots);
        buttons
    }

    /// 根据前台窗口解析可见按钮（固定槽位策略）
    pub fn resolve_visibility(&self, window: Option<&WindowInfo>) -> Vec<ButtonConfig> {
        let slots = self.slot_count();
        self.buttons
            .iter()
            .take(slots)
            .map(|btn| {
                if btn.scope.is_visible(window) {
                    btn.clone()
                } else {
                    empty_slot_button()
                }
            })
            .chain(std::iter::repeat_with(empty_slot_button))
            .take(slots)
            .collect()
    }

    /// 托盘菜单条目：`(槽位 index, 菜单标题)`，按槽位顺序。
    /// 需同时满足 enabled、已配置动作、`show_in_tray_menu`、scope 对当前窗口可见。
    pub fn tray_menu_entries(&self, window: Option<&WindowInfo>) -> Vec<(usize, String)> {
        let slots = self.slot_count();
        self.buttons
            .iter()
            .take(slots)
            .enumerate()
            .filter(|(_, btn)| {
                btn.enabled
                    && btn.show_in_tray_menu
                    && btn.action.is_configured()
                    && btn.scope.is_visible(window)
            })
            .map(|(i, btn)| {
                let label = btn.label.trim();
                let title = if label.is_empty() {
                    format!("按钮 {}", i + 1)
                } else {
                    label.to_string()
                };
                (i, title)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn window(app_name: &str, bundle_id: Option<&str>) -> WindowInfo {
        WindowInfo {
            app_name: app_name.into(),
            bundle_id: bundle_id.map(str::to_string),
            title: String::new(),
            process_id: 1,
            process_path: PathBuf::new(),
            page_url: None,
        }
    }

    fn button(label: &str, scope: ButtonScope) -> ButtonConfig {
        ButtonConfig {
            label: label.into(),
            icon: None,
            action: Action::None,
            enabled: true,
            background_color: None,
            border_color: None,
            icon_color: None,
            scope,
            hotkey: None,
            show_in_tray_menu: false,
        }
    }

    fn vscode_scope() -> ButtonScope {
        ButtonScope::WhenActive(MatchRule {
            bundle_id: Some("com.microsoft.VSCode".into()),
            app_name: None,
            title_regex: None,
            title_regex_text: None,
        })
    }

    #[test]
    fn scope_default_is_always() {
        assert!(matches!(ButtonScope::default(), ButtonScope::Always));
    }

    #[test]
    fn always_visible_with_any_window() {
        let scope = ButtonScope::Always;
        assert!(scope.is_visible(None));
        assert!(scope.is_visible(Some(&window("Safari", Some("com.apple.Safari")))));
    }

    #[test]
    fn when_active_matches_bundle() {
        let scope = vscode_scope();
        let vscode = window("Code", Some("com.microsoft.VSCode"));
        let safari = window("Safari", Some("com.apple.Safari"));
        assert!(scope.is_visible(Some(&vscode)));
        assert!(!scope.is_visible(Some(&safari)));
    }

    #[test]
    fn when_active_hidden_without_window() {
        assert!(!vscode_scope().is_visible(None));
    }

    #[test]
    fn resolve_visibility_fixed_slots() {
        let panel = PanelConfig {
            rows: 1,
            cols: 3,
            display_mode: DisplayMode::Text,
            collapsed: false,
            opacity: 1.0,
            button_width: 64.0,
            button_height: 64.0,
            gap_x: 4.0,
            gap_y: 4.0,
            position: Position { x: 0.0, y: 0.0 },
            collapsed_icon: None,
            buttons: vec![
                button("A", ButtonScope::Always),
                button("B", vscode_scope()),
                button("C", ButtonScope::Always),
            ],
            ui_style: UiStyle::default(),
        };

        let safari = window("Safari", Some("com.apple.Safari"));
        let resolved = panel.resolve_visibility(Some(&safari));
        assert_eq!(resolved.len(), 3);
        assert_eq!(resolved[0].label, "A");
        assert!(!resolved[1].enabled);
        assert!(resolved[1].label.is_empty());
        assert_eq!(resolved[2].label, "C");

        let vscode = window("Code", Some("com.microsoft.VSCode"));
        let resolved = panel.resolve_visibility(Some(&vscode));
        assert_eq!(resolved[1].label, "B");
        assert!(resolved[1].enabled);
    }

    #[test]
    fn tray_menu_entries_filters_and_orders() {
        let mut always = button("复制", ButtonScope::Always);
        always.show_in_tray_menu = true;
        always.action = Action::Hotkey(vec![Key::Cmd, Key::Char('c')]);

        let mut scoped = button("VS", vscode_scope());
        scoped.show_in_tray_menu = true;
        scoped.action = Action::OpenUrl {
            url: "https://example.com".into(),
        };

        let mut disabled = button("关", ButtonScope::Always);
        disabled.show_in_tray_menu = true;
        disabled.enabled = false;
        disabled.action = Action::OpenUrl {
            url: "https://x".into(),
        };

        let mut no_flag = button("面板", ButtonScope::Always);
        no_flag.action = Action::OpenUrl {
            url: "https://y".into(),
        };

        let mut empty_label = button("", ButtonScope::Always);
        empty_label.show_in_tray_menu = true;
        empty_label.action = Action::OpenUrl {
            url: "https://z".into(),
        };

        let panel = PanelConfig {
            rows: 1,
            cols: 5,
            display_mode: DisplayMode::Text,
            collapsed: false,
            opacity: 1.0,
            button_width: 64.0,
            button_height: 64.0,
            gap_x: 4.0,
            gap_y: 4.0,
            position: Position { x: 0.0, y: 0.0 },
            collapsed_icon: None,
            buttons: vec![always, scoped, disabled, no_flag, empty_label],
            ui_style: UiStyle::default(),
        };

        let safari = window("Safari", Some("com.apple.Safari"));
        let entries = panel.tray_menu_entries(Some(&safari));
        assert_eq!(
            entries,
            vec![(0, "复制".into()), (4, "按钮 5".into())]
        );

        let vscode = window("Code", Some("com.microsoft.VSCode"));
        let entries = panel.tray_menu_entries(Some(&vscode));
        assert_eq!(
            entries,
            vec![
                (0, "复制".into()),
                (1, "VS".into()),
                (4, "按钮 5".into())
            ]
        );
    }

    #[test]
    fn hover_tooltip_prefers_script_source() {
        let btn = ButtonConfig {
            label: "保存".into(),
            icon: None,
            action: Action::Script(ActionScript::from_source("快捷键 Cmd+S\n等待 100\n")),
            enabled: true,
            background_color: None,
            border_color: None,
            icon_color: None,
            scope: ButtonScope::Always,
            hotkey: Some("Cmd+Shift+1".into()),
            show_in_tray_menu: false,
        };
        let tip = btn.hover_tooltip().expect("tooltip");
        assert!(tip.contains("保存"));
        assert!(tip.contains("快捷键 Cmd+Shift+1"));
        assert!(tip.contains("快捷键 Cmd+S"));
        assert!(tip.contains("等待 100"));
    }
}
