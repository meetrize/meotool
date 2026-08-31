//! 将 legacy `Action` 转为 MeoFlow 源码（编辑 UI 读兼容）。

use crate::config::export::keys_to_hotkey_string;
use crate::config::script_presets::{escape_meoflow_string, is_comments_only};
use crate::config::{Action, ActionScript, AppConfig};

use super::{compile, macro_to_meoflow};

/// 打开编辑 UI 时，将任意 Action 转为可编辑的 MeoFlow 文本。
pub fn action_to_meoflow(action: &Action) -> String {
    match action {
        Action::None => String::new(),
        Action::Hotkey(keys) => {
            let spec = keys_to_hotkey_string(keys);
            if spec.is_empty() {
                String::new()
            } else {
                format!("快捷键 {spec}\n")
            }
        }
        Action::OpenApp { target } => format!("打开应用 \"{}\"\n", escape_meoflow_string(target)),
        Action::Shell { command } => format!("执行命令 \"{}\"\n", escape_meoflow_string(command)),
        Action::OpenUrl { url } => format!("打开链接 \"{}\"\n", escape_meoflow_string(url)),
        Action::OpenFile { path } => format!("打开文件 \"{}\"\n", escape_meoflow_string(path)),
        Action::PasteText { text } => format!("粘贴 \"{}\"\n", escape_meoflow_string(text)),
        Action::MouseMacro(m) => macro_to_meoflow(m),
        Action::Script(s) => s.source.clone(),
    }
}

/// 是否需要在保存时从 legacy 升级为 script。
pub fn action_needs_legacy_upgrade(action: &Action) -> bool {
    !matches!(action, Action::None | Action::Script(_))
}

/// 从 Action 提取 attached_macro（Script 或 MouseMacro）。
pub fn action_attached_macro(action: &Action) -> Option<crate::config::MouseMacro> {
    match action {
        Action::Script(s) => s.attached_macro.clone(),
        Action::MouseMacro(m) => Some(m.clone()),
        _ => None,
    }
}

/// 批量将配置中 legacy 动作升级为 `Action::Script`；返回升级按钮数。
pub fn migrate_actions_in_config(config: &mut AppConfig) -> usize {
    let mut count = 0usize;
    for profile in &mut config.profiles {
        for button in &mut profile.buttons {
            if !action_needs_legacy_upgrade(&button.action) {
                continue;
            }
            let source = action_to_meoflow(&button.action);
            if source.trim().is_empty() || is_comments_only(&source) {
                continue;
            }
            let attached = action_attached_macro(&button.action);
            let Ok(program) = compile(&source, attached.as_ref()) else {
                log::warn!(
                    "skip migrate button {:?}: compile failed",
                    button.label
                );
                continue;
            };
            button.action = Action::Script(ActionScript {
                source,
                program: Some(program),
                attached_macro: attached,
            });
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Key, MouseMacro};

    #[test]
    fn hotkey_to_meoflow() {
        let action = Action::Hotkey(vec![Key::Cmd, Key::Char('s')]);
        assert_eq!(action_to_meoflow(&action).trim(), "快捷键 Cmd+s");
    }

    #[test]
    fn none_is_empty() {
        assert!(action_to_meoflow(&Action::None).is_empty());
    }

    #[test]
    fn open_app_url_file_shell() {
        assert_eq!(
            action_to_meoflow(&Action::OpenApp {
                target: "Notes".into()
            })
            .trim(),
            "打开应用 \"Notes\""
        );
        assert_eq!(
            action_to_meoflow(&Action::OpenUrl {
                url: "https://x.com".into()
            })
            .trim(),
            "打开链接 \"https://x.com\""
        );
        assert_eq!(
            action_to_meoflow(&Action::OpenFile {
                path: "~/a.pdf".into()
            })
            .trim(),
            "打开文件 \"~/a.pdf\""
        );
        assert!(action_to_meoflow(&Action::Shell {
            command: "echo hi".into()
        })
        .contains("执行命令"));
    }

    #[test]
    fn paste_with_quotes() {
        let action = Action::PasteText {
            text: "say \"hi\"".into(),
        };
        assert!(action_to_meoflow(&action).contains(r#"say \"hi\""#));
    }

    #[test]
    fn script_passthrough() {
        let src = "输入 \"x\"\n";
        let action = Action::Script(crate::config::ActionScript {
            source: src.into(),
            program: None,
            attached_macro: None,
        });
        assert_eq!(action_to_meoflow(&action), src);
    }

    #[test]
    fn needs_upgrade() {
        assert!(action_needs_legacy_upgrade(&Action::Hotkey(vec![])));
        assert!(!action_needs_legacy_upgrade(&Action::Script(
            crate::config::ActionScript {
                source: String::new(),
                program: None,
                attached_macro: None,
            }
        )));
    }

    #[test]
    fn macro_delegates() {
        let m = MouseMacro {
            steps: vec![],
            pre_delay_ms: 0,
            recorded_bundle_id: None,
        };
        let action = Action::MouseMacro(m);
        assert!(action_to_meoflow(&action).starts_with("# 由鼠标宏导出"));
    }
}
