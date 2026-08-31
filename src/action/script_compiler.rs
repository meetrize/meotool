//! MeoFlow 行式 DSL 编译器：源码 → FlowProgram + 中文行号诊断。

use crate::config::export::hotkey_string_to_keys;
use crate::config::{
    CoordSpace, FlowProgram, FlowStep, Key, MouseButtonKind, MouseMacro, SpecialKey,
};

#[derive(Debug, Clone)]
pub struct ScriptDiagnostic {
    pub line: usize,
    pub message: String,
    pub hint: Option<String>,
}

impl ScriptDiagnostic {
    pub fn display(&self) -> String {
        let mut s = format!("第 {} 行：{}", self.line, self.message);
        if let Some(hint) = &self.hint {
            s.push('\n');
            s.push_str(hint);
        }
        s
    }
}

#[derive(Debug, Clone)]
pub struct CompileError {
    pub diagnostics: Vec<ScriptDiagnostic>,
}

impl CompileError {
    pub fn primary_display(&self) -> String {
        self.diagnostics
            .first()
            .map(|d| d.display())
            .unwrap_or_else(|| "脚本编译失败".into())
    }
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.primary_display())
    }
}

/// 编译 MeoFlow 源码。`attached_macro` 供「播放宏」嵌入。
pub fn compile(
    source: &str,
    attached_macro: Option<&MouseMacro>,
) -> Result<FlowProgram, CompileError> {
    if source.len() > FlowProgram::MAX_SOURCE_BYTES {
        return Err(CompileError {
            diagnostics: vec![ScriptDiagnostic {
                line: 1,
                message: format!(
                    "源码过长（最多 {} KB）",
                    FlowProgram::MAX_SOURCE_BYTES / 1024
                ),
                hint: Some("请删减步骤或拆成多个按钮。".into()),
            }],
        });
    }

    let line_count = source.lines().count();
    if line_count > FlowProgram::MAX_LINES {
        return Err(CompileError {
            diagnostics: vec![ScriptDiagnostic {
                line: FlowProgram::MAX_LINES + 1,
                message: format!("行数过多（最多 {} 行，含空行与注释）", FlowProgram::MAX_LINES),
                hint: None,
            }],
        });
    }

    let mut steps = Vec::new();
    let mut estimated_ms = 0u64;
    let mut diagnostics = Vec::new();

    for (idx, raw_line) in source.lines().enumerate() {
        let line_no = idx + 1;
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }

        match parse_line(line, line_no, attached_macro) {
            Ok(step) => {
                estimated_ms = estimated_ms.saturating_add(step_cost_ms(&step));
                steps.push(step);
            }
            Err(d) => diagnostics.push(d),
        }
    }

    if !diagnostics.is_empty() {
        return Err(CompileError { diagnostics });
    }

    if steps.is_empty() {
        return Err(CompileError {
            diagnostics: vec![ScriptDiagnostic {
                line: 1,
                message: "脚本没有可执行步骤".into(),
                hint: Some("例如：输入 \"你好\" → 等待 100 → 按键 Enter".into()),
            }],
        });
    }

    if steps.len() > FlowProgram::MAX_STEPS {
        return Err(CompileError {
            diagnostics: vec![ScriptDiagnostic {
                line: 1,
                message: format!("步骤过多（最多 {} 步）", FlowProgram::MAX_STEPS),
                hint: None,
            }],
        });
    }

    if estimated_ms > FlowProgram::MAX_TOTAL_MS {
        return Err(CompileError {
            diagnostics: vec![ScriptDiagnostic {
                line: 1,
                message: format!(
                    "预估总时长超过 {} 秒（当前约 {:.1} 秒）",
                    FlowProgram::MAX_TOTAL_MS / 1000,
                    estimated_ms as f64 / 1000.0
                ),
                hint: Some("请缩短「等待」或减少步骤。".into()),
            }],
        });
    }

    Ok(FlowProgram {
        steps,
        estimated_ms,
    })
}

fn step_cost_ms(step: &FlowStep) -> u64 {
    match step {
        FlowStep::Delay { ms } => *ms,
        FlowStep::TypeText { text } => 20 + text.chars().count() as u64,
        FlowStep::PasteText { .. } => 80,
        FlowStep::Key { .. } => 40,
        FlowStep::Hotkey { .. } => 50,
        FlowStep::Click { clicks, .. } => 40 * (*clicks as u64).max(1),
        FlowStep::OpenApp { .. } | FlowStep::OpenUrl { .. } | FlowStep::OpenFile { .. } => 200,
        FlowStep::Shell { .. } => 300,
        FlowStep::ActivateApp { .. } => 150,
        FlowStep::PlayMouseMacro { macro_ } => {
            macro_.pre_delay_ms
                + macro_
                    .steps
                    .iter()
                    .map(|s| match s {
                        crate::config::MacroStep::Delay { ms } => *ms,
                        _ => 40,
                    })
                    .sum::<u64>()
        }
    }
}

fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut escaped = false;
    for (i, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '#' if !in_string => return &line[..i],
            _ => {}
        }
    }
    line
}

fn parse_line(
    line: &str,
    line_no: usize,
    attached_macro: Option<&MouseMacro>,
) -> Result<FlowStep, ScriptDiagnostic> {
    let (cmd, rest) = split_command(line);
    let cmd_key = normalize_cmd(cmd);

    match cmd_key.as_str() {
        "等待" | "wait" => parse_wait(rest, line_no),
        "输入" | "type" | "typewrite" => {
            let text = parse_quoted_string(rest, line_no, "输入")?;
            Ok(FlowStep::TypeText { text })
        }
        "粘贴" | "paste" => {
            let text = parse_quoted_string(rest, line_no, "粘贴")?;
            Ok(FlowStep::PasteText { text })
        }
        "按键" | "key" | "press" => {
            let name = rest.trim();
            if name.is_empty() {
                return Err(diag(
                    line_no,
                    "按键需要键名，例如：按键 Enter",
                    Some("常用键有 Enter、Tab、Esc、Delete、Space。"),
                ));
            }
            let key = parse_special_key(name).ok_or_else(|| {
                diag(
                    line_no,
                    &format!("按键名称无法识别「{name}」。"),
                    Some("常用键有 Enter、Tab、Esc、Delete、Space、方向键、F1–F12。"),
                )
            })?;
            Ok(FlowStep::Key { key })
        }
        "快捷键" | "hotkey" => parse_hotkey(rest, line_no),
        "点击" | "click" => parse_click(rest, line_no, 1),
        "双击" | "double_click" | "doubleclick" => parse_click(rest, line_no, 2),
        "打开应用" | "open_app" | "openapp" => {
            let target = parse_quoted_string(rest, line_no, "打开应用")?;
            Ok(FlowStep::OpenApp { target })
        }
        "打开链接" | "open_url" | "openurl" => {
            let url = parse_quoted_string(rest, line_no, "打开链接")?;
            Ok(FlowStep::OpenUrl { url })
        }
        "打开文件" | "open_file" | "openfile" => {
            let path = parse_quoted_string(rest, line_no, "打开文件")?;
            Ok(FlowStep::OpenFile { path })
        }
        "执行命令" | "shell" => {
            let command = parse_quoted_string(rest, line_no, "执行命令")?;
            Ok(FlowStep::Shell { command })
        }
        "激活应用" | "activate" | "activate_app" => {
            let bundle_id = parse_quoted_string(rest, line_no, "激活应用")?;
            Ok(FlowStep::ActivateApp { bundle_id })
        }
        "播放宏" | "play_macro" | "playmacro" => {
            if !rest.trim().is_empty() {
                return Err(diag(
                    line_no,
                    "「播放宏」暂不支持参数",
                    Some("可先把鼠标宏导出为动作脚本步骤，或使用无参「播放宏」播放已附加的宏。"),
                ));
            }
            let Some(m) = attached_macro.filter(|m| m.is_configured()) else {
                return Err(diag(
                    line_no,
                    "没有可播放的鼠标宏",
                    Some("请先从「鼠标宏」导出为脚本，或在编辑器中附加宏后再使用「播放宏」。"),
                ));
            };
            Ok(FlowStep::PlayMouseMacro { macro_: m.clone() })
        }
        other => Err(diag(
            line_no,
            &format!("无法识别的命令「{other}」"),
            Some("常用命令：等待、输入、粘贴、按键、快捷键、点击、打开应用、打开链接。"),
        )),
    }
}

fn split_command(line: &str) -> (&str, &str) {
    let mut chars = line.char_indices();
    let Some((_, first)) = chars.next() else {
        return ("", "");
    };
    // ASCII 命令：吃到空白
    if first.is_ascii_alphabetic() {
        let end = line
            .char_indices()
            .find(|(_, c)| c.is_whitespace())
            .map(|(i, _)| i)
            .unwrap_or(line.len());
        return (&line[..end], line[end..].trim_start());
    }
    // 中文命令：已知多字命令优先匹配
    for cmd in [
        "打开应用",
        "打开链接",
        "打开文件",
        "执行命令",
        "激活应用",
        "播放宏",
        "快捷键",
        "双击",
        "等待",
        "输入",
        "粘贴",
        "按键",
        "点击",
    ] {
        if line.starts_with(cmd) {
            return (cmd, line[cmd.len()..].trim_start());
        }
    }
    let end = line
        .char_indices()
        .find(|(_, c)| c.is_whitespace())
        .map(|(i, _)| i)
        .unwrap_or(line.len());
    (&line[..end], line[end..].trim_start())
}

fn normalize_cmd(cmd: &str) -> String {
    cmd.trim().to_lowercase()
}

fn parse_wait(rest: &str, line_no: usize) -> Result<FlowStep, ScriptDiagnostic> {
    let token = rest.trim();
    if token.is_empty() {
        return Err(diag(
            line_no,
            "等待需要毫秒数，例如：等待 100",
            None,
        ));
    }
    let ms = parse_duration_ms(token).ok_or_else(|| {
        diag(
            line_no,
            &format!("无法解析等待时间「{token}」"),
            Some("请写成：等待 100 或 等待 100ms"),
        )
    })?;
    if ms > FlowProgram::MAX_WAIT_MS {
        return Err(diag(
            line_no,
            &format!("单次等待不能超过 {} 毫秒", FlowProgram::MAX_WAIT_MS),
            Some("请改小等待时间，或拆成多步。"),
        ));
    }
    Ok(FlowStep::Delay { ms })
}

fn parse_duration_ms(token: &str) -> Option<u64> {
    let t = token.trim().to_lowercase();
    let t = t.strip_suffix("ms").unwrap_or(&t).trim();
    t.parse::<u64>().ok()
}

fn parse_hotkey(rest: &str, line_no: usize) -> Result<FlowStep, ScriptDiagnostic> {
    let spec = rest.trim().trim_matches('"');
    if spec.is_empty() {
        return Err(diag(
            line_no,
            "快捷键不能为空，例如：快捷键 Cmd+S",
            None,
        ));
    }
    let keys = hotkey_string_to_keys(spec);
    if keys.is_empty() {
        return Err(diag(
            line_no,
            &format!("无法解析快捷键「{spec}」"),
            Some("格式示例：Cmd+S、Cmd+Shift+P"),
        ));
    }
    if !keys.iter().any(|k| matches!(k, Key::Char(_))) {
        return Err(diag(
            line_no,
            "快捷键缺少主键（字母或数字）",
            Some("格式示例：Cmd+S"),
        ));
    }
    Ok(FlowStep::Hotkey { keys })
}

fn parse_click(rest: &str, line_no: usize, clicks: u8) -> Result<FlowStep, ScriptDiagnostic> {
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    if tokens.len() < 2 {
        return Err(diag(
            line_no,
            "点击需要两个数字坐标，例如：点击 0.5 0.3",
            None,
        ));
    }
    let x: f32 = tokens[0].parse().map_err(|_| {
        diag(
            line_no,
            &format!("横坐标无法识别「{}」", tokens[0]),
            Some("窗口相对坐标请用 0~1，例如：点击 0.42 0.18"),
        )
    })?;
    let y: f32 = tokens[1].parse().map_err(|_| {
        diag(
            line_no,
            &format!("纵坐标无法识别「{}」", tokens[1]),
            Some("窗口相对坐标请用 0~1，例如：点击 0.42 0.18"),
        )
    })?;

    let mut button = MouseButtonKind::Left;
    let mut coord_space = CoordSpace::WindowNormalized;

    for tok in &tokens[2..] {
        let t = tok.to_lowercase();
        match t.as_str() {
            "左" | "left" => button = MouseButtonKind::Left,
            "右" | "right" => button = MouseButtonKind::Right,
            "中" | "middle" => button = MouseButtonKind::Middle,
            "窗口" | "window" | "window_normalized" => {
                coord_space = CoordSpace::WindowNormalized
            }
            "屏幕" | "screen" | "screen_abs" | "absolute" => {
                coord_space = CoordSpace::ScreenAbs
            }
            other => {
                return Err(diag(
                    line_no,
                    &format!("无法识别的点击参数「{other}」"),
                    Some("可选：左/右/中、窗口/屏幕"),
                ));
            }
        }
    }

    if coord_space == CoordSpace::WindowNormalized && !(0.0..=1.0).contains(&x) {
        return Err(diag(
            line_no,
            "窗口相对横坐标应在 0~1 之间",
            Some("若要用屏幕像素坐标，请加「屏幕」，例如：点击 1200 800 屏幕"),
        ));
    }
    if coord_space == CoordSpace::WindowNormalized && !(0.0..=1.0).contains(&y) {
        return Err(diag(
            line_no,
            "窗口相对纵坐标应在 0~1 之间",
            Some("若要用屏幕像素坐标，请加「屏幕」，例如：点击 1200 800 屏幕"),
        ));
    }

    Ok(FlowStep::Click {
        button,
        x,
        y,
        coord_space,
        clicks,
    })
}

fn parse_quoted_string(rest: &str, line_no: usize, cmd: &str) -> Result<String, ScriptDiagnostic> {
    let s = rest.trim();
    if s.is_empty() {
        return Err(diag(
            line_no,
            &format!("{cmd}需要引号字符串，例如：{cmd} \"内容\""),
            None,
        ));
    }
    if !s.starts_with('"') {
        return Err(diag(
            line_no,
            &format!("{cmd}的参数请用双引号包裹，例如：{cmd} \"内容\""),
            None,
        ));
    }
    let mut out = String::new();
    let mut chars = s.chars().skip(1).peekable();
    let mut closed = false;
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
            continue;
        }
        if ch == '"' {
            closed = true;
            break;
        }
        out.push(ch);
    }
    if !closed {
        return Err(diag(
            line_no,
            "字符串缺少结尾双引号",
            Some("示例：输入 \"你好\""),
        ));
    }
    let trailing = chars.collect::<String>();
    if !trailing.trim().is_empty() {
        return Err(diag(
            line_no,
            &format!("引号后还有多余内容「{}」", trailing.trim()),
            None,
        ));
    }
    Ok(out)
}

pub fn parse_special_key(name: &str) -> Option<SpecialKey> {
    let n = name.trim();
    let lower = n.to_lowercase();
    match lower.as_str() {
        "enter" | "return" | "回车" => Some(SpecialKey::Enter),
        "tab" => Some(SpecialKey::Tab),
        "esc" | "escape" => Some(SpecialKey::Escape),
        "space" | "空格" => Some(SpecialKey::Space),
        "delete" | "del" => Some(SpecialKey::Delete),
        "backspace" | "退格" => Some(SpecialKey::Backspace),
        "up" | "↑" | "上" => Some(SpecialKey::Up),
        "down" | "↓" | "下" => Some(SpecialKey::Down),
        "left" | "←" | "左" => Some(SpecialKey::Left),
        "right" | "→" | "右" => Some(SpecialKey::Right),
        "home" => Some(SpecialKey::Home),
        "end" => Some(SpecialKey::End),
        "pageup" | "page_up" | "pgup" => Some(SpecialKey::PageUp),
        "pagedown" | "page_down" | "pgdn" => Some(SpecialKey::PageDown),
        "f1" => Some(SpecialKey::F1),
        "f2" => Some(SpecialKey::F2),
        "f3" => Some(SpecialKey::F3),
        "f4" => Some(SpecialKey::F4),
        "f5" => Some(SpecialKey::F5),
        "f6" => Some(SpecialKey::F6),
        "f7" => Some(SpecialKey::F7),
        "f8" => Some(SpecialKey::F8),
        "f9" => Some(SpecialKey::F9),
        "f10" => Some(SpecialKey::F10),
        "f11" => Some(SpecialKey::F11),
        "f12" => Some(SpecialKey::F12),
        _ => None,
    }
}

fn diag(line: usize, message: &str, hint: Option<&str>) -> ScriptDiagnostic {
    ScriptDiagnostic {
        line,
        message: message.to_string(),
        hint: hint.map(str::to_string),
    }
}

/// 将鼠标宏步骤导出为 MeoFlow 源码文本。
pub fn macro_to_meoflow(macro_: &MouseMacro) -> String {
    let mut lines = Vec::new();
    lines.push("# 由鼠标宏导出".to_string());
    if macro_.pre_delay_ms > 0 {
        lines.push(format!("等待 {}", macro_.pre_delay_ms));
    }
    for step in &macro_.steps {
        match step {
            crate::config::MacroStep::Delay { ms } => lines.push(format!("等待 {ms}")),
            crate::config::MacroStep::ActivateApp { bundle_id, .. } => {
                lines.push(format!("激活应用 \"{}\"", escape_quotes(bundle_id)));
            }
            crate::config::MacroStep::Hotkey { keys, .. } => {
                let spec = crate::config::export::keys_to_hotkey_string(keys);
                lines.push(format!("快捷键 {spec}"));
            }
            crate::config::MacroStep::Click {
                button,
                x,
                y,
                coord_space,
                ..
            } => {
                let mut parts = vec![
                    "点击".to_string(),
                    format_coord(*x),
                    format_coord(*y),
                ];
                match button {
                    MouseButtonKind::Left => {}
                    MouseButtonKind::Right => parts.push("右".into()),
                    MouseButtonKind::Middle => parts.push("中".into()),
                }
                if *coord_space == CoordSpace::ScreenAbs {
                    parts.push("屏幕".into());
                }
                lines.push(parts.join(" "));
            }
        }
    }
    lines.join("\n")
}

fn escape_quotes(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn format_coord(v: f32) -> String {
    if (v.fract()).abs() < 1e-6 {
        format!("{v:.0}")
    } else {
        format!("{v:.4}").trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// 步骤预览一行文案
pub fn step_preview_label(step: &FlowStep) -> String {
    match step {
        FlowStep::Delay { ms } => format!("等待 {ms}ms"),
        FlowStep::TypeText { text } => format!("输入 「{}」", truncate(text, 24)),
        FlowStep::PasteText { text } => format!("粘贴 「{}」", truncate(text, 24)),
        FlowStep::Key { key } => format!("按键 {key:?}").replace("Enter", "Enter"),
        FlowStep::Hotkey { keys } => {
            format!("快捷键 {}", crate::config::export::keys_to_hotkey_string(keys))
        }
        FlowStep::Click {
            x,
            y,
            clicks,
            button,
            coord_space,
            ..
        } => {
            let kind = if *clicks >= 2 { "双击" } else { "点击" };
            let btn = match button {
                MouseButtonKind::Left => "",
                MouseButtonKind::Right => "右键",
                MouseButtonKind::Middle => "中键",
            };
            let space = match coord_space {
                CoordSpace::WindowNormalized => "窗口",
                CoordSpace::ScreenAbs => "屏幕",
            };
            format!("{kind}{btn} ({x:.2}, {y:.2}) {space}")
        }
        FlowStep::OpenApp { target } => format!("打开应用 {target}"),
        FlowStep::OpenUrl { url } => format!("打开链接 {}", truncate(url, 32)),
        FlowStep::OpenFile { path } => format!("打开文件 {}", truncate(path, 32)),
        FlowStep::Shell { command } => format!("执行命令 {}", truncate(command, 32)),
        FlowStep::ActivateApp { bundle_id } => format!("激活应用 {bundle_id}"),
        FlowStep::PlayMouseMacro { macro_ } => format!("播放宏（{} 步）", macro_.steps.len()),
    }
}

fn truncate(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_type_wait_enter() {
        let prog = compile(
            r#"
输入 "hello"
等待 100
按键 Enter
"#,
            None,
        )
        .expect("compile");
        assert_eq!(prog.steps.len(), 3);
        assert!(matches!(prog.steps[0], FlowStep::TypeText { .. }));
        assert!(matches!(prog.steps[1], FlowStep::Delay { ms: 100 }));
        assert!(matches!(
            prog.steps[2],
            FlowStep::Key {
                key: SpecialKey::Enter
            }
        ));
        assert!(prog.estimated_ms >= 100);
    }

    #[test]
    fn reject_unknown_command_with_line() {
        let err = compile("等待 10\n飞天\n按键 Enter", None).unwrap_err();
        assert_eq!(err.diagnostics[0].line, 2);
        assert!(err.primary_display().contains("第 2 行"));
    }

    #[test]
    fn reject_huge_wait() {
        let err = compile("等待 999999", None).unwrap_err();
        assert!(err.primary_display().contains("10000") || err.primary_display().contains("超过"));
    }

    #[test]
    fn english_aliases() {
        let prog = compile("type \"hi\"\nwait 50\nkey Enter", None).unwrap();
        assert_eq!(prog.steps.len(), 3);
    }

    #[test]
    fn comment_and_escape() {
        let prog = compile(
            r#"
# 注释
输入 "a\"b"
"#,
            None,
        )
        .unwrap();
        match &prog.steps[0] {
            FlowStep::TypeText { text } => assert_eq!(text, "a\"b"),
            _ => panic!(),
        }
    }

    #[test]
    fn click_coords() {
        let prog = compile("点击 0.42 0.18 右", None).unwrap();
        match &prog.steps[0] {
            FlowStep::Click {
                x,
                y,
                button,
                clicks,
                coord_space,
                ..
            } => {
                assert!((*x - 0.42).abs() < 1e-5);
                assert!((*y - 0.18).abs() < 1e-5);
                assert_eq!(*button, MouseButtonKind::Right);
                assert_eq!(*clicks, 1);
                assert_eq!(*coord_space, CoordSpace::WindowNormalized);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn macro_export_roundtrip_shape() {
        let m = MouseMacro {
            steps: vec![
                crate::config::MacroStep::Click {
                    button: MouseButtonKind::Left,
                    x: 0.5,
                    y: 0.3,
                    coord_space: CoordSpace::WindowNormalized,
                    note: None,
                },
                crate::config::MacroStep::Delay { ms: 100 },
                crate::config::MacroStep::Hotkey {
                    keys: vec![Key::Cmd, Key::Char('s')],
                    note: None,
                },
            ],
            pre_delay_ms: 80,
            recorded_bundle_id: None,
        };
        let src = macro_to_meoflow(&m);
        let prog = compile(&src, None).unwrap();
        assert!(prog.steps.len() >= 3);
    }
}
