//! 动作脚本预设：快捷添加与流程模板（Native / egui 共用）。

/// 预设参数采集方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetParamKind {
    /// 流程模板占位（由 wizard 填充）。
    FlowWizard,
    HotkeyCapture,
    Text { multiline: bool },
    Url,
    ShellCommand,
    OpenFilePanel,
    OpenAppPanel,
    ClickCapture,
    MacroRecord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetGroup {
    Quick,
    Flow,
}

#[derive(Debug, Clone, Copy)]
pub struct ScriptPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub tooltip: &'static str,
    pub param: PresetParamKind,
    /// 静态 MeoFlow 片段；`{hotkey}` `{text}` `{url}` `{path}` `{app}` `{x}` `{y}` 为占位符。
    pub template: &'static str,
    #[allow(dead_code)]
    pub group: PresetGroup,
}

pub const DEFAULT_SCRIPT_SKELETON: &str =
    "# 一行写一步。例如：输入 \"你好\" → 等待 100 → 按键 Enter\n";

pub const QUICK_PRESETS: &[ScriptPreset] = &[
    ScriptPreset {
        id: "hotkey",
        label: "快捷键",
        tooltip: "录制一组按键并插入「快捷键 Cmd+…」行。不会切换当前前台应用。",
        param: PresetParamKind::HotkeyCapture,
        template: "快捷键 {hotkey}\n",
        group: PresetGroup::Quick,
    },
    ScriptPreset {
        id: "open_app",
        label: "打开应用",
        tooltip: "选择 .app 或点选窗口，插入「打开应用 \"…\"」行。",
        param: PresetParamKind::OpenAppPanel,
        template: "打开应用 \"{app}\"\n",
        group: PresetGroup::Quick,
    },
    ScriptPreset {
        id: "open_url",
        label: "打开链接",
        tooltip: "输入网址，用默认浏览器打开。",
        param: PresetParamKind::Url,
        template: "打开链接 \"{url}\"\n",
        group: PresetGroup::Quick,
    },
    ScriptPreset {
        id: "open_file",
        label: "打开文件",
        tooltip: "选择本地文件或文件夹，用系统默认应用打开。",
        param: PresetParamKind::OpenFilePanel,
        template: "打开文件 \"{path}\"\n",
        group: PresetGroup::Quick,
    },
    ScriptPreset {
        id: "paste",
        label: "粘贴文本",
        tooltip: "将预设文字写入剪贴板并模拟粘贴（Cmd+V）。适合长文本与 emoji。",
        param: PresetParamKind::Text { multiline: true },
        template: "粘贴 \"{text}\"\n",
        group: PresetGroup::Quick,
    },
    ScriptPreset {
        id: "shell",
        label: "执行命令",
        tooltip: "运行一条 Shell 命令（与旧「执行命令」相同权限）。",
        param: PresetParamKind::ShellCommand,
        template: "执行命令 \"{text}\"\n",
        group: PresetGroup::Quick,
    },
    ScriptPreset {
        id: "click_once",
        label: "点击一次",
        tooltip: "到目标窗口点一下，插入窗口相对坐标「点击 x y」。",
        param: PresetParamKind::ClickCapture,
        template: "点击 {x} {y}\n",
        group: PresetGroup::Quick,
    },
    ScriptPreset {
        id: "record_clicks",
        label: "录制点击",
        tooltip: "连续录制多次点击、按键与等待，结束追加多行脚本。",
        param: PresetParamKind::MacroRecord,
        template: "",
        group: PresetGroup::Quick,
    },
];

pub const FLOW_PRESETS: &[ScriptPreset] = &[
    ScriptPreset {
        id: "type_enter",
        label: "输入后回车",
        tooltip: "输入短文字 → 等待 100ms → 按 Enter。适合提交表单。",
        param: PresetParamKind::FlowWizard,
        template: "输入 \"{text}\"\n等待 100\n按键 Enter\n",
        group: PresetGroup::Flow,
    },
    ScriptPreset {
        id: "paste_enter",
        label: "粘贴后回车",
        tooltip: "粘贴预设内容 → 等待 → Enter。适合邮箱、验证码槽位。",
        param: PresetParamKind::FlowWizard,
        template: "粘贴 \"{text}\"\n等待 50\n按键 Enter\n",
        group: PresetGroup::Flow,
    },
    ScriptPreset {
        id: "click_hotkey",
        label: "点击后快捷键",
        tooltip: "先点目标位置，再发送快捷键（如 Cmd+S 保存）。",
        param: PresetParamKind::FlowWizard,
        template: "点击 {x} {y}\n等待 200\n快捷键 {hotkey}\n",
        group: PresetGroup::Flow,
    },
    ScriptPreset {
        id: "open_app_type",
        label: "打开应用并输入",
        tooltip: "启动应用后等待界面，再输入一段文字。",
        param: PresetParamKind::FlowWizard,
        template: "打开应用 \"{app}\"\n等待 400\n输入 \"{text}\"\n",
        group: PresetGroup::Flow,
    },
];

pub fn preset_by_id(id: &str) -> Option<&'static ScriptPreset> {
    QUICK_PRESETS
        .iter()
        .chain(FLOW_PRESETS.iter())
        .find(|p| p.id == id)
}

/// MeoFlow 双引号字符串转义。
pub fn escape_meoflow_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

/// 渲染带占位符的模板。
pub fn render_template(template: &str, params: &PresetBuildParams) -> String {
    let mut out = template.to_string();
    if let Some(v) = &params.hotkey {
        out = out.replace("{hotkey}", v);
    }
    if let Some(v) = &params.text {
        out = out.replace("{text}", &escape_meoflow_string(v));
    }
    if let Some(v) = &params.url {
        out = out.replace("{url}", &escape_meoflow_string(v));
    }
    if let Some(v) = &params.path {
        out = out.replace("{path}", &escape_meoflow_string(v));
    }
    if let Some(v) = &params.app {
        out = out.replace("{app}", &escape_meoflow_string(v));
    }
    if let Some(v) = &params.x {
        out = out.replace("{x}", v);
    }
    if let Some(v) = &params.y {
        out = out.replace("{y}", v);
    }
    out
}

#[derive(Debug, Default, Clone)]
pub struct PresetBuildParams {
    pub hotkey: Option<String>,
    pub text: Option<String>,
    pub url: Option<String>,
    pub path: Option<String>,
    pub app: Option<String>,
    pub x: Option<String>,
    pub y: Option<String>,
}

/// 脚本是否仅含注释与空白（视为未配置）。
pub fn is_comments_only(source: &str) -> bool {
    source.lines().all(|line| {
        let t = line.trim();
        t.is_empty() || t.starts_with('#')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_quotes_and_newlines() {
        assert_eq!(escape_meoflow_string(r#"a"b"#), r#"a\"b"#);
        assert_eq!(escape_meoflow_string("a\nb"), r"a\nb");
    }

    #[test]
    fn render_hotkey_preset() {
        let out = render_template(
            "快捷键 {hotkey}\n",
            &PresetBuildParams {
                hotkey: Some("Cmd+S".into()),
                ..Default::default()
            },
        );
        assert_eq!(out, "快捷键 Cmd+S\n");
    }

    #[test]
    fn is_comments_only_detects() {
        assert!(is_comments_only("# hello\n\n"));
        assert!(!is_comments_only("# c\n输入 \"x\"\n"));
    }
}
