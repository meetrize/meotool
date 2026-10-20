//! 按钮外观快速配色预设（native / egui 编辑器共用）。

use super::color::parse_color;
use super::model::ButtonConfig;

/// 单组按钮配色：背景 + 图标色。
#[derive(Debug, Clone, Copy)]
pub struct ButtonColorPreset {
    pub name: &'static str,
    pub background_color: &'static str,
    pub icon_color: &'static str,
}

/// 九套预设；第一项为面板默认风（浅蓝底 + 深灰图标）。
pub const BUTTON_COLOR_PRESETS: [ButtonColorPreset; 9] = [
    ButtonColorPreset {
        name: "默认",
        background_color: "rgba(175, 182, 202, 0.878)",
        icon_color: "rgba(0, 0, 0, 0.875)",
    },
    ButtonColorPreset {
        name: "石墨",
        background_color: "#2A2A2A",
        icon_color: "#E8E8E8",
    },
    ButtonColorPreset {
        name: "深海",
        background_color: "#1E3A5F",
        icon_color: "#64B5F6",
    },
    ButtonColorPreset {
        name: "翡翠",
        background_color: "#1B4332",
        icon_color: "#95D5B2",
    },
    ButtonColorPreset {
        name: "暮光紫",
        background_color: "#4A1942",
        icon_color: "#E1BEE7",
    },
    ButtonColorPreset {
        name: "琥珀",
        background_color: "#3D3520",
        icon_color: "#FFD54F",
    },
    ButtonColorPreset {
        name: "珊瑚",
        background_color: "#4A1C2B",
        icon_color: "#FF8A80",
    },
    ButtonColorPreset {
        name: "青柠",
        background_color: "#1A3A3A",
        icon_color: "#A5D6A7",
    },
    ButtonColorPreset {
        name: "极简",
        background_color: "rgba(0, 0, 0, 0.35)",
        icon_color: "#FFFFFF",
    },
];

/// 将预设写入按钮配置（同时设置背景与图标色）。
pub fn apply_preset(cfg: &mut ButtonConfig, index: usize) {
    let Some(preset) = BUTTON_COLOR_PRESETS.get(index) else {
        return;
    };
    cfg.background_color = Some(preset.background_color.to_string());
    cfg.icon_color = Some(preset.icon_color.to_string());
}

/// 若当前按钮配色与某预设完全一致，返回其索引。
pub fn preset_index_for(bg: Option<&str>, icon: Option<&str>) -> Option<usize> {
    let bg = bg?;
    let icon = icon?;
    BUTTON_COLOR_PRESETS.iter().position(|p| {
        colors_match(bg, p.background_color) && colors_match(icon, p.icon_color)
    })
}

/// 外观区「继承 / 自定义」状态文案。
pub fn appearance_hint_text(inherit_bg: bool, inherit_icon: bool) -> &'static str {
    match (inherit_bg, inherit_icon) {
        (true, true) => "背景与图标均继承全局主题",
        (true, false) => "背景继承全局；图标已自定义",
        (false, true) => "背景已自定义；图标继承全局",
        (false, false) => "背景与图标均已自定义",
    }
}

fn colors_match(a: &str, b: &str) -> bool {
    match (parse_color(a), parse_color(b)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_presets_parse() {
        for preset in &BUTTON_COLOR_PRESETS {
            assert!(
                parse_color(preset.background_color).is_some(),
                "bg unparsable: {}",
                preset.name
            );
            assert!(
                parse_color(preset.icon_color).is_some(),
                "icon unparsable: {}",
                preset.name
            );
        }
    }

    #[test]
    fn apply_and_match_roundtrip() {
        let mut cfg = ButtonConfig {
            label: String::new(),
            icon: None,
            action: super::super::model::Action::None,
            enabled: true,
            background_color: None,
            border_color: None,
            icon_color: None,
            scope: super::super::model::ButtonScope::Always,
            hotkey: None,
        };
        apply_preset(&mut cfg, 3);
        assert_eq!(
            preset_index_for(cfg.background_color.as_deref(), cfg.icon_color.as_deref()),
            Some(3)
        );
    }

    #[test]
    fn appearance_hint_variants() {
        assert_eq!(appearance_hint_text(true, true), "背景与图标均继承全局主题");
        assert_eq!(appearance_hint_text(false, false), "背景与图标均已自定义");
    }
}
