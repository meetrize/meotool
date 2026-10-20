//! 外观 / 脚本双窗共享的按钮编辑 draft。

use std::path::PathBuf;

use crate::config::{ButtonConfig, MouseMacro, RgbColor};

#[derive(Debug, Clone)]
pub struct ButtonEditSession {
    pub profile: usize,
    pub button: usize,
    pub draft: ButtonConfig,
    /// 打开编辑器时的快照；取消时恢复
    pub original: ButtonConfig,
    pub attached_macro: Option<MouseMacro>,
    pub draft_legacy: bool,
    pub icon_dirs: Vec<PathBuf>,
    pub theme_button_bg: RgbColor,
    pub theme_icon_color: RgbColor,
}

impl ButtonEditSession {
    pub fn new(
        profile: usize,
        button: usize,
        draft: ButtonConfig,
        icon_dirs: &[PathBuf],
        theme_button_bg: RgbColor,
        theme_icon_color: RgbColor,
    ) -> Self {
        use crate::action::{action_attached_macro, action_needs_legacy_upgrade};
        let draft_legacy = action_needs_legacy_upgrade(&draft.action);
        let attached_macro = action_attached_macro(&draft.action);
        let original = draft.clone();
        Self {
            profile,
            button,
            draft,
            original,
            attached_macro,
            draft_legacy,
            icon_dirs: icon_dirs.to_vec(),
            theme_button_bg,
            theme_icon_color,
        }
    }

    pub fn matches(&self, profile: usize, button: usize) -> bool {
        self.profile == profile && self.button == button
    }

    pub fn apply_saved(&mut self, config: ButtonConfig) {
        use crate::action::action_attached_macro;
        self.draft = config;
        self.attached_macro = action_attached_macro(&self.draft.action);
        self.draft_legacy = false;
    }
}
