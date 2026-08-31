use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::config::{Action, Key, MouseMacro};
use enigo::{
    Direction::{Click, Press, Release},
    Enigo, Key as EnigoKey, Keyboard, Settings,
};
use log::{error, info, warn};

use super::macro_player::play_mouse_macro;
use super::script_player::play_script_async;
#[cfg(feature = "ui-egui")]
use super::script_player::play_script_guarded;

pub struct ActionExecutor {
    enigo: Option<Enigo>,
    init_error: Option<String>,
}

impl ActionExecutor {
    pub fn new() -> Self {
        Self {
            enigo: None,
            init_error: None,
        }
    }

    fn ensure_enigo(&mut self) -> Result<(), String> {
        if self.enigo.is_some() {
            return Ok(());
        }

        self.init_error = None;

        let settings = Settings {
            open_prompt_to_get_permissions: false,
            ..Settings::default()
        };

        match Enigo::new(&settings) {
            Ok(enigo) => {
                self.enigo = Some(enigo);
                Ok(())
            }
            Err(err) => {
                let msg = format!(
                    "无法模拟键鼠输入（需要辅助功能权限）: {err}. \
                     请前往：系统设置 → 隐私与安全性 → 辅助功能，授权后重启应用"
                );
                warn!("{msg}");
                self.init_error = Some(msg.clone());
                Err(msg)
            }
        }
    }

    pub fn execute(&mut self, action: &Action) -> Result<(), String> {
        match action {
            Action::Hotkey(keys) => {
                self.ensure_enigo()?;
                self.send_hotkey(keys)
            }
            Action::OpenApp { target } => open_app(target),
            Action::Shell { command } => run_shell(command),
            Action::OpenUrl { url } => open_url(url),
            Action::OpenFile { path } => open_file(path),
            Action::PasteText { text } => self.paste_text(text),
            Action::MouseMacro(m) => {
                self.ensure_enigo()?;
                self.play_macro(m)
            }
            Action::Script(script) => {
                // Phase3：后台执行 + 重入忽略；编译错误仍同步返回
                play_script_async(script.clone())
            }
            Action::None => Ok(()),
        }
    }

    /// 试运行：同步执行（编辑器内反馈错误；仅 egui 路径）
    #[cfg(feature = "ui-egui")]
    pub fn try_run_script(&mut self, script: &crate::config::ActionScript) -> Result<(), String> {
        self.ensure_enigo()?;
        let enigo = self
            .enigo
            .as_mut()
            .ok_or_else(|| "enigo not initialized".to_string())?;
        play_script_guarded(enigo, script)
    }

    fn play_macro(&mut self, macro_action: &MouseMacro) -> Result<(), String> {
        let enigo = self
            .enigo
            .as_mut()
            .ok_or_else(|| "enigo not initialized".to_string())?;
        play_mouse_macro(enigo, macro_action)
    }

    fn send_hotkey(&mut self, keys: &[Key]) -> Result<(), String> {
        let enigo = self
            .enigo
            .as_mut()
            .ok_or_else(|| "enigo not initialized".to_string())?;

        let mut modifiers = Vec::new();
        let mut main_key = None;

        for key in keys {
            match key {
                Key::Cmd | Key::Ctrl | Key::Alt | Key::Shift => modifiers.push(*key),
                Key::Char(c) => {
                    if main_key.is_some() {
                        return Err("multiple main keys in hotkey".into());
                    }
                    main_key = Some(*c);
                }
            }
        }

        let main_key = main_key.ok_or_else(|| "hotkey missing main key".to_string())?;

        for modifier in &modifiers {
            press_modifier(enigo, *modifier)?;
        }

        enigo
            .key(EnigoKey::Unicode(main_key), Click)
            .map_err(|e| format!("key click failed: {e}"))?;

        for modifier in modifiers.iter().rev() {
            release_modifier(enigo, *modifier)?;
        }

        info!("executed hotkey: {:?}", keys);
        Ok(())
    }

    fn paste_text(&mut self, text: &str) -> Result<(), String> {
        if text.trim().is_empty() {
            return Err("paste text is empty".into());
        }

        set_clipboard_text(text)?;
        self.ensure_enigo()?;
        self.send_hotkey(&[Key::Cmd, Key::Char('v')])?;
        info!("pasted preset text ({} chars)", text.chars().count());
        Ok(())
    }
}

impl Default for ActionExecutor {
    fn default() -> Self {
        Self::new()
    }
}

fn set_clipboard_text(text: &str) -> Result<(), String> {
    // pbcopy 按 locale 解码 stdin；非 UTF-8（如 zh_CN.GBK）会把「继续」变成「缁х画」
    let mut child = Command::new("pbcopy")
        .env("LANG", "en_US.UTF-8")
        .env("LC_ALL", "en_US.UTF-8")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("pbcopy failed: {e}"))?;

    let stdin = child
        .stdin
        .as_mut()
        .ok_or_else(|| "pbcopy stdin unavailable".to_string())?;
    stdin
        .write_all(text.as_bytes())
        .map_err(|e| format!("write clipboard failed: {e}"))?;

    let status = child
        .wait()
        .map_err(|e| format!("pbcopy wait failed: {e}"))?;
    if !status.success() {
        return Err("pbcopy exited with error".into());
    }

    Ok(())
}

pub(crate) fn set_clipboard_text_pub(text: &str) -> Result<(), String> {
    set_clipboard_text(text)
}

fn open_app(target: &str) -> Result<(), String> {
    let target = target.trim();
    if target.is_empty() {
        return Err("open app target is empty".into());
    }

    let mut cmd = Command::new("open");
    if target.ends_with(".app") || target.starts_with('/') || target.starts_with('~') {
        cmd.arg(target);
    } else if target.contains('.') && !target.contains(' ') {
        // 形如 com.microsoft.VSCode 的 Bundle ID
        cmd.args(["-b", target]);
    } else {
        cmd.args(["-a", target]);
    }

    cmd.spawn()
        .map_err(|e| format!("open app '{target}' failed: {e}"))?;
    info!("opened app: {target}");
    Ok(())
}

fn run_shell(command: &str) -> Result<(), String> {
    let command = command.trim();
    if command.is_empty() {
        return Err("shell command is empty".into());
    }

    Command::new("sh")
        .arg("-c")
        .arg(command)
        .spawn()
        .map_err(|e| format!("run shell failed: {e}"))?;
    info!("spawned shell: {command}");
    Ok(())
}

fn open_url(url: &str) -> Result<(), String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("url is empty".into());
    }

    Command::new("open")
        .arg(url)
        .spawn()
        .map_err(|e| format!("open url '{url}' failed: {e}"))?;
    info!("opened url: {url}");
    Ok(())
}

fn open_file(path: &str) -> Result<(), String> {
    let path = normalize_open_path(path)?;
    Command::new("open")
        .arg(&path)
        .spawn()
        .map_err(|e| format!("open file '{}' failed: {e}", path.display()))?;
    info!("opened file: {}", path.display());
    Ok(())
}

/// 规范化「打开文件」路径：去引号、解码 file://、展开 ~，并要求绝对路径。
pub(crate) fn normalize_open_path(raw: &str) -> Result<PathBuf, String> {
    let mut path = raw.trim();
    if path.len() >= 2
        && ((path.starts_with('"') && path.ends_with('"'))
            || (path.starts_with('\'') && path.ends_with('\'')))
    {
        path = path[1..path.len() - 1].trim();
    }
    if path.is_empty() {
        return Err("file path is empty".into());
    }

    let decoded = strip_file_url(path);
    let path = decoded.as_deref().unwrap_or(path);
    let expanded = expand_tilde(path)?;
    if !expanded.is_absolute() {
        return Err(format!(
            "file path must be absolute or start with ~: {path}"
        ));
    }
    Ok(expanded)
}

fn strip_file_url(s: &str) -> Option<String> {
    let rest = if s.len() >= 7 && s[..7].eq_ignore_ascii_case("file://") {
        &s[7..]
    } else {
        return None;
    };
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    Some(percent_decode(rest))
}

fn expand_tilde(path: &str) -> Result<PathBuf, String> {
    if path == "~" {
        return dirs::home_dir().ok_or_else(|| "cannot resolve home directory".to_string());
    }
    if let Some(rest) = path.strip_prefix("~/") {
        let home = dirs::home_dir().ok_or_else(|| "cannot resolve home directory".to_string())?;
        return Ok(home.join(rest));
    }
    Ok(PathBuf::from(path))
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn press_modifier(enigo: &mut Enigo, key: Key) -> Result<(), String> {
    let enigo_key = modifier_to_enigo(key)?;
    enigo
        .key(enigo_key, Press)
        .map_err(|e| format!("modifier press failed: {e}"))
}

fn release_modifier(enigo: &mut Enigo, key: Key) -> Result<(), String> {
    let enigo_key = modifier_to_enigo(key)?;
    enigo
        .key(enigo_key, Release)
        .map_err(|e| format!("modifier release failed: {e}"))
}

fn modifier_to_enigo(key: Key) -> Result<EnigoKey, String> {
    match key {
        Key::Cmd => Ok(EnigoKey::Meta),
        Key::Ctrl => Ok(EnigoKey::Control),
        Key::Alt => Ok(EnigoKey::Alt),
        Key::Shift => Ok(EnigoKey::Shift),
        Key::Char(_) => Err("char is not a modifier".into()),
    }
}

pub fn log_action_error(err: &str) {
    error!("{err}");
}

#[cfg(test)]
mod tests {
    use super::normalize_open_path;
    use std::path::{Path, PathBuf};

    #[test]
    fn rejects_empty_path() {
        assert!(normalize_open_path("  ").is_err());
        assert!(normalize_open_path("\"\"").is_err());
    }

    #[test]
    fn rejects_relative_path() {
        let err = normalize_open_path("Documents/a.pdf").unwrap_err();
        assert!(err.contains("absolute"));
    }

    #[test]
    fn expands_tilde() {
        let home = dirs::home_dir().expect("home dir");
        assert_eq!(
            normalize_open_path("~/Documents/a.pdf").unwrap(),
            home.join("Documents/a.pdf")
        );
        assert_eq!(normalize_open_path("~").unwrap(), home);
    }

    #[test]
    fn accepts_absolute_and_quoted_paths() {
        let path = PathBuf::from("/Users/me/My Notes.pdf");
        assert_eq!(normalize_open_path("/Users/me/My Notes.pdf").unwrap(), path);
        assert_eq!(
            normalize_open_path("\"/Users/me/My Notes.pdf\"").unwrap(),
            path
        );
    }

    #[test]
    fn decodes_file_url() {
        assert_eq!(
            normalize_open_path("file:///Users/me/My%20Doc.pdf").unwrap(),
            Path::new("/Users/me/My Doc.pdf")
        );
        assert_eq!(
            normalize_open_path("file://localhost/Users/me/a.pdf").unwrap(),
            Path::new("/Users/me/a.pdf")
        );
        let home = dirs::home_dir().expect("home dir");
        assert_eq!(
            normalize_open_path("file://~/Documents/a.pdf").unwrap(),
            home.join("Documents/a.pdf")
        );
    }
}
