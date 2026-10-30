use crate::config::app_config::{AppConfig, Profile, WindowInfo};
use crate::config::model::PanelConfig;

pub struct ProfileResolver<'a> {
    config: &'a AppConfig,
}

impl<'a> ProfileResolver<'a> {
    pub fn new(config: &'a AppConfig) -> Self {
        Self { config }
    }

    pub fn resolve(&self, window: &WindowInfo) -> (&'a Profile, PanelConfig) {
        let profile = self
            .find_matching_profile(window)
            .unwrap_or_else(|| self.default_profile());
        let panel = self.config.build_panel(profile);
        (profile, panel)
    }

    fn find_matching_profile(&self, window: &WindowInfo) -> Option<&'a Profile> {
        for profile in &self.config.profiles {
            if profile.name == "default" {
                continue;
            }
            if let Some(rule) = &profile.match_rule {
                if rule.matches(window) {
                    return Some(profile);
                }
            }
        }
        None
    }

    fn default_profile(&self) -> &'a Profile {
        let idx = self.config.default_profile_index();
        &self.config.profiles[idx]
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::app_config::{
        ContextMode, GestureConfig, GlobalHotkeys, GlobalPanelDefaults, PanelSettings, Profile,
    };
    use crate::config::model::{
        Action, ButtonConfig, ButtonScope, DisplayMode, MatchRule, Position,
    };

    fn sample_config() -> AppConfig {
        AppConfig {
            global_panel: GlobalPanelDefaults {
                settings: PanelSettings::default(),
                position: Position { x: 0.0, y: 0.0 },
            },
            hotkeys: GlobalHotkeys::default(),
            gesture: GestureConfig::default(),
            profiles: vec![
                Profile {
                    name: "default".into(),
                    match_rule: None,
                    panel: PanelSettings::default(),
                    buttons: vec![ButtonConfig {
                        label: "复制".into(),
                        icon: None,
                        action: Action::None,
                        enabled: true,
                        background_color: None,
                        border_color: None,
                        icon_color: None,
                        scope: ButtonScope::Always,
                        hotkey: None,
                        show_in_tray_menu: false,
                    }],
                },
                Profile {
                    name: "vscode".into(),
                    match_rule: Some(MatchRule {
                        bundle_id: Some("com.microsoft.VSCode".into()),
                        app_name: Some("Code".into()),
                        title_regex: None,
                        title_regex_text: None,
                    }),
                    panel: PanelSettings {
                        rows: Some(2),
                        cols: Some(2),
                        display_mode: Some(DisplayMode::IndexIcon),
                        ..Default::default()
                    },
                    buttons: vec![],
                },
            ],
            poll_interval_ms: 200,
            context_mode: ContextMode::Profile,
            tray_show_buttons: false,
        }
    }

    #[test]
    fn resolves_vscode_by_bundle_id() {
        let config = sample_config();
        let resolver = ProfileResolver::new(&config);
        let window = WindowInfo {
            app_name: "Code".into(),
            bundle_id: Some("com.microsoft.VSCode".into()),
            title: String::new(),
            process_id: 1,
            process_path: PathBuf::new(),
            page_url: None,
        };
        let (profile, panel) = resolver.resolve(&window);
        assert_eq!(profile.name, "vscode");
        assert_eq!(panel.rows, 2);
    }

    #[test]
    fn falls_back_to_default() {
        let config = sample_config();
        let resolver = ProfileResolver::new(&config);
        let window = WindowInfo {
            app_name: "Safari".into(),
            bundle_id: Some("com.apple.Safari".into()),
            title: String::new(),
            process_id: 2,
            process_path: PathBuf::new(),
            page_url: None,
        };
        let (profile, _) = resolver.resolve(&window);
        assert_eq!(profile.name, "default");
    }
}
