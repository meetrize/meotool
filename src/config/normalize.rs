//! 将 AppConfig 归一化为稳定内存形态，避免 UI 同步与磁盘省略字段产生语义漂移。

use super::app_config::{AppConfig, PanelSettings, ThemeSettings};
use super::model::empty_slot_button;

/// 原地归一化：补齐按钮槽位、清理空字体、clamp 行列等。
pub fn normalize_app_config(config: &mut AppConfig) {
    normalize_panel_settings(&mut config.global_panel.settings);

    let global_rows = config.global_panel.settings.rows;
    let global_cols = config.global_panel.settings.cols;

    for profile in &mut config.profiles {
        normalize_panel_settings(&mut profile.panel);

        let rows = profile.panel.rows.or(global_rows).unwrap_or(3);
        let cols = profile.panel.cols.or(global_cols).unwrap_or(3);
        let slots = (rows as usize) * (cols as usize);

        if profile.buttons.len() < slots {
            profile
                .buttons
                .resize_with(slots, empty_slot_button);
        } else if profile.buttons.len() > slots {
            profile.buttons.truncate(slots);
        }
    }

    config.poll_interval_ms = config.poll_interval_ms.max(50);
}

fn normalize_panel_settings(settings: &mut PanelSettings) {
    if let Some(rows) = settings.rows.as_mut() {
        *rows = (*rows).clamp(1, 9);
    }
    if let Some(cols) = settings.cols.as_mut() {
        *cols = (*cols).clamp(1, 9);
    }
    if let Some(opacity) = settings.opacity.as_mut() {
        *opacity = (*opacity).clamp(0.05, 1.0);
    }
    if let Some(gap) = settings.gap_x.as_mut() {
        *gap = (*gap).clamp(0.0, 32.0);
    }
    if let Some(gap) = settings.gap_y.as_mut() {
        *gap = (*gap).clamp(0.0, 32.0);
    }
    normalize_theme(&mut settings.theme);
}

fn normalize_theme(theme: &mut ThemeSettings) {
    if let Some(font) = theme.letter_font.as_ref() {
        if font.trim().is_empty() {
            theme.letter_font = None;
        }
    }
    if let Some(r) = theme.button_corner_radius.as_mut() {
        *r = (*r).clamp(0.0, 32.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::app_config::{
        ContextMode, GestureConfig, GlobalHotkeys, GlobalPanelDefaults, Profile,
    };
    use crate::config::model::Position;

    fn base_config() -> AppConfig {
        AppConfig {
            global_panel: GlobalPanelDefaults {
                settings: PanelSettings::default(),
                position: Position { x: 0.0, y: 0.0 },
            },
            hotkeys: GlobalHotkeys::default(),
            gesture: GestureConfig::default(),
            profiles: vec![Profile {
                name: "default".into(),
                match_rule: None,
                panel: PanelSettings::default(),
                buttons: vec![],
            }],
            poll_interval_ms: 200,
            context_mode: ContextMode::Profile,
            tray_show_buttons: false,
        }
    }

    #[test]
    fn pads_buttons_to_grid_size() {
        let mut config = base_config();
        config.global_panel.settings.rows = Some(2);
        config.global_panel.settings.cols = Some(3);
        config.profiles[0].buttons.clear();

        normalize_app_config(&mut config);

        assert_eq!(config.profiles[0].buttons.len(), 6);
        assert!(config.profiles[0]
            .buttons
            .iter()
            .all(|b| b.label.is_empty() && !b.enabled));
    }

    #[test]
    fn truncates_excess_buttons() {
        let mut config = base_config();
        config.profiles[0].panel.rows = Some(1);
        config.profiles[0].panel.cols = Some(2);
        config.profiles[0].buttons = (0..5).map(|_| empty_slot_button()).collect();

        normalize_app_config(&mut config);

        assert_eq!(config.profiles[0].buttons.len(), 2);
    }

    #[test]
    fn empty_letter_font_becomes_none() {
        let mut config = base_config();
        config.global_panel.settings.theme.letter_font = Some("  ".into());
        config.profiles[0].panel.theme.letter_font = Some(String::new());

        normalize_app_config(&mut config);

        assert!(config.global_panel.settings.theme.letter_font.is_none());
        assert!(config.profiles[0].panel.theme.letter_font.is_none());
    }

    #[test]
    fn normalize_twice_is_idempotent() {
        let mut config = base_config();
        config.global_panel.settings.rows = Some(0);
        config.global_panel.settings.cols = Some(99);
        config.global_panel.settings.opacity = Some(0.01);
        config.global_panel.settings.gap_x = Some(-3.0);
        config.global_panel.settings.gap_y = Some(99.0);
        config.global_panel.settings.theme.letter_font = Some("".into());
        config.profiles[0].panel.rows = Some(2);
        config.profiles[0].panel.cols = Some(2);
        config.profiles[0].buttons = vec![empty_slot_button()];
        config.poll_interval_ms = 10;

        normalize_app_config(&mut config);
        let once = format!("{config:?}");

        normalize_app_config(&mut config);
        let twice = format!("{config:?}");

        assert_eq!(once, twice);
        assert_eq!(config.global_panel.settings.rows, Some(1));
        assert_eq!(config.global_panel.settings.cols, Some(9));
        assert_eq!(config.global_panel.settings.opacity, Some(0.05));
        assert_eq!(config.global_panel.settings.gap_x, Some(0.0));
        assert_eq!(config.global_panel.settings.gap_y, Some(32.0));
        assert_eq!(config.poll_interval_ms, 50);
        assert_eq!(config.profiles[0].buttons.len(), 4);
    }
}
