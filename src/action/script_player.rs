//! MeoFlow 脚本执行器：顺序执行 FlowProgram。

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use enigo::{
    Button, Coordinate,
    Direction::{Click, Press, Release},
    Enigo, Key as EnigoKey, Keyboard, Mouse,
};
use log::{info, warn};

use crate::config::{
    ActionScript, CoordSpace, FlowProgram, FlowStep, Key, MouseButtonKind, SpecialKey,
};
use crate::platform::window_bounds::{activate_bundle_id, target_to_screen};

use super::executor::{normalize_open_path, set_clipboard_text_pub};
use super::macro_player::play_mouse_macro;
use super::script_compiler::compile;

static SCRIPT_RUNNING: AtomicBool = AtomicBool::new(false);

pub fn is_script_running() -> bool {
    SCRIPT_RUNNING.load(Ordering::SeqCst)
}

fn try_begin_script() -> bool {
    SCRIPT_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

fn end_script() {
    SCRIPT_RUNNING.store(false, Ordering::SeqCst);
}

/// 同步执行动作脚本（已持有 / 不检查重入锁）。
#[cfg(feature = "ui-egui")]
pub fn play_script_sync(enigo: &mut Enigo, script: &ActionScript) -> Result<(), String> {
    let program = match &script.program {
        Some(p) if !p.steps.is_empty() => p.clone(),
        _ => compile(&script.source, script.attached_macro.as_ref())
            .map_err(|e| e.primary_display())?,
    };
    play_program(enigo, &program)
}

/// 带重入保护的执行：若已有脚本在跑则忽略。
#[cfg(feature = "ui-egui")]
pub fn play_script_guarded(enigo: &mut Enigo, script: &ActionScript) -> Result<(), String> {
    if !try_begin_script() {
        info!("script already running, ignore re-entrant trigger");
        return Ok(());
    }
    let result = play_script_sync(enigo, script);
    end_script();
    result
}

/// 后台线程执行；立即返回。重入时忽略。
pub fn play_script_async(script: ActionScript) -> Result<(), String> {
    if !try_begin_script() {
        info!("script already running, ignore re-entrant trigger");
        return Ok(());
    }

    // 先编译，避免线程里才发现语法错误却无反馈
    let program = match &script.program {
        Some(p) if !p.steps.is_empty() => p.clone(),
        _ => match compile(&script.source, script.attached_macro.as_ref()) {
            Ok(p) => p,
            Err(e) => {
                end_script();
                return Err(e.primary_display());
            }
        },
    };

    thread::spawn(move || {
        // macOS：Enigo/HIToolbox 必须在主线程，否则会 SIGTRAP 崩退
        let result = play_program_from_any_thread(&program);
        if let Err(err) = result {
            warn!("script async play failed: {err}");
        }
        end_script();
    });
    Ok(())
}

/// 试运行：先编译，后台执行，完成后回调（任意线程）。重入时返回 Err。
pub fn play_script_try_run(
    script: ActionScript,
    on_done: impl FnOnce(Result<(), String>) + Send + 'static,
) -> Result<(), String> {
    if !try_begin_script() {
        return Err("已有脚本在运行，请稍后再试".into());
    }

    let program = match &script.program {
        Some(p) if !p.steps.is_empty() => p.clone(),
        _ => match compile(&script.source, script.attached_macro.as_ref()) {
            Ok(p) => p,
            Err(e) => {
                end_script();
                return Err(e.primary_display());
            }
        },
    };

    thread::spawn(move || {
        let result = play_program_from_any_thread(&program);
        end_script();
        on_done(result);
    });
    Ok(())
}

fn create_enigo() -> Result<Enigo, String> {
    let settings = enigo::Settings {
        open_prompt_to_get_permissions: false,
        ..enigo::Settings::default()
    };
    Enigo::new(&settings).map_err(|err| {
        format!(
            "无法模拟键鼠输入（需要辅助功能权限）: {err}. \
             请前往：系统设置 → 隐私与安全性 → 辅助功能，授权后重启应用"
        )
    })
}

#[cfg(target_os = "macos")]
fn is_main_thread() -> bool {
    extern "C" {
        fn pthread_main_np() -> i32;
    }
    unsafe { pthread_main_np() != 0 }
}

/// 在主线程执行闭包。已在主线程则直接跑，避免 GCD `dispatch_sync` 自死锁。
fn run_on_main<F, T>(f: F) -> T
where
    F: Send + FnOnce() -> T,
    T: Send,
{
    #[cfg(target_os = "macos")]
    {
        if is_main_thread() {
            f()
        } else {
            dispatch::Queue::main().exec_sync(f)
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        f()
    }
}

fn step_needs_enigo(step: &FlowStep) -> bool {
    !matches!(
        step,
        FlowStep::Delay { .. }
            | FlowStep::Shell { .. }
            | FlowStep::OpenApp { .. }
            | FlowStep::OpenUrl { .. }
            | FlowStep::OpenFile { .. }
            | FlowStep::ActivateApp { .. }
    )
}

/// 任意线程可调用：等待/打开类步骤留在当前线程，键鼠步骤切到主线程执行。
fn play_program_from_any_thread(program: &FlowProgram) -> Result<(), String> {
    validate_program(program)?;

    let restore_pos = run_on_main(|| -> Result<Option<(i32, i32)>, String> {
        let enigo = create_enigo()?;
        Ok(enigo.location().ok())
    })?;

    let exclude_pid = Some(std::process::id());
    let mut preferred_bundle: Option<String> = None;
    let mut spent_ms = 0u64;

    let result = (|| {
        for (index, step) in program.steps.iter().enumerate() {
            let step_no = index + 1;
            let step_result = if step_needs_enigo(step) {
                run_on_main(|| {
                    let mut enigo = create_enigo()?;
                    execute_step(
                        &mut enigo,
                        step,
                        exclude_pid,
                        &mut preferred_bundle,
                        &mut spent_ms,
                    )
                })
            } else {
                execute_step_no_enigo(step, &mut preferred_bundle, &mut spent_ms)
            };
            match step_result {
                Ok(()) => {}
                Err(err) => {
                    return Err(format!("脚本第 {step_no} 步失败：{err}"));
                }
            }
            if spent_ms > FlowProgram::MAX_TOTAL_MS {
                return Err(format!("脚本第 {step_no} 步后总时长超限"));
            }
        }
        Ok(())
    })();

    if let Some((x, y)) = restore_pos {
        if let Err(err) = run_on_main(|| {
            let mut enigo = create_enigo()?;
            enigo
                .move_mouse(x, y, Coordinate::Abs)
                .map_err(|e| format!("{e}"))
        }) {
            warn!("restore cursor after script failed: {err}");
        }
    }

    result?;
    info!(
        "executed script: {} steps, ~{}ms",
        program.steps.len(),
        program.estimated_ms
    );
    Ok(())
}

fn validate_program(program: &FlowProgram) -> Result<(), String> {
    if program.steps.is_empty() {
        return Err("脚本没有可执行步骤".into());
    }
    if program.steps.len() > FlowProgram::MAX_STEPS {
        return Err(format!("脚本步骤过多（最多 {} 步）", FlowProgram::MAX_STEPS));
    }
    Ok(())
}

#[cfg(feature = "ui-egui")]
fn play_program(enigo: &mut Enigo, program: &FlowProgram) -> Result<(), String> {
    validate_program(program)?;

    let restore_pos = enigo.location().ok();
    let exclude_pid = Some(std::process::id());
    let mut preferred_bundle: Option<String> = None;
    let mut spent_ms = 0u64;

    let result = (|| {
        for (index, step) in program.steps.iter().enumerate() {
            let step_no = index + 1;
            match execute_step(
                enigo,
                step,
                exclude_pid,
                &mut preferred_bundle,
                &mut spent_ms,
            ) {
                Ok(()) => {}
                Err(err) => {
                    return Err(format!("脚本第 {step_no} 步失败：{err}"));
                }
            }
            if spent_ms > FlowProgram::MAX_TOTAL_MS {
                return Err(format!("脚本第 {step_no} 步后总时长超限"));
            }
        }
        Ok(())
    })();

    if let Some((x, y)) = restore_pos {
        if let Err(err) = enigo.move_mouse(x, y, Coordinate::Abs) {
            warn!("restore cursor after script failed: {err}");
        }
    }

    result?;
    info!(
        "executed script: {} steps, ~{}ms",
        program.steps.len(),
        program.estimated_ms
    );
    Ok(())
}

fn execute_step_no_enigo(
    step: &FlowStep,
    preferred_bundle: &mut Option<String>,
    spent_ms: &mut u64,
) -> Result<(), String> {
    match step {
        FlowStep::Delay { ms } => {
            let ms = (*ms).min(FlowProgram::MAX_WAIT_MS);
            *spent_ms = spent_ms.saturating_add(ms);
            if ms > 0 {
                thread::sleep(Duration::from_millis(ms));
            }
            Ok(())
        }
        FlowStep::OpenApp { target } => {
            open_app(target)?;
            *spent_ms = spent_ms.saturating_add(200);
            Ok(())
        }
        FlowStep::OpenUrl { url } => {
            open_url(url)?;
            *spent_ms = spent_ms.saturating_add(200);
            Ok(())
        }
        FlowStep::OpenFile { path } => {
            open_file(path)?;
            *spent_ms = spent_ms.saturating_add(200);
            Ok(())
        }
        FlowStep::Shell { command } => {
            run_shell_timeout(command, FlowProgram::SHELL_TIMEOUT_MS)?;
            *spent_ms = spent_ms.saturating_add(300);
            Ok(())
        }
        FlowStep::ActivateApp { bundle_id } => {
            activate_bundle_id(bundle_id)?;
            *preferred_bundle = Some(bundle_id.clone());
            *spent_ms = spent_ms.saturating_add(150);
            thread::sleep(Duration::from_millis(150));
            Ok(())
        }
        _ => Err("内部错误：该步骤需要键鼠模拟".into()),
    }
}

fn execute_step(
    enigo: &mut Enigo,
    step: &FlowStep,
    exclude_pid: Option<u32>,
    preferred_bundle: &mut Option<String>,
    spent_ms: &mut u64,
) -> Result<(), String> {
    match step {
        FlowStep::Delay { ms } => {
            let ms = (*ms).min(FlowProgram::MAX_WAIT_MS);
            *spent_ms = spent_ms.saturating_add(ms);
            if ms > 0 {
                thread::sleep(Duration::from_millis(ms));
            }
            Ok(())
        }
        FlowStep::TypeText { text } => {
            if text.is_empty() {
                return Err("输入文本为空".into());
            }
            enigo
                .text(text)
                .map_err(|e| format!("输入文本失败: {e}"))?;
            *spent_ms = spent_ms.saturating_add(20 + text.chars().count() as u64);
            Ok(())
        }
        FlowStep::PasteText { text } => {
            if text.trim().is_empty() {
                return Err("粘贴文本为空".into());
            }
            set_clipboard_text_pub(text)?;
            play_hotkey(enigo, &[Key::Cmd, Key::Char('v')])?;
            *spent_ms = spent_ms.saturating_add(80);
            Ok(())
        }
        FlowStep::Key { key } => {
            let enigo_key = special_to_enigo(*key)?;
            enigo
                .key(enigo_key, Click)
                .map_err(|e| format!("按键失败: {e}"))?;
            *spent_ms = spent_ms.saturating_add(40);
            thread::sleep(Duration::from_millis(20));
            Ok(())
        }
        FlowStep::Hotkey { keys } => {
            play_hotkey(enigo, keys)?;
            *spent_ms = spent_ms.saturating_add(50);
            thread::sleep(Duration::from_millis(20));
            Ok(())
        }
        FlowStep::Click {
            button,
            x,
            y,
            coord_space,
            clicks,
        } => {
            play_click(
                enigo,
                *button,
                *x,
                *y,
                *coord_space,
                *clicks,
                exclude_pid,
                preferred_bundle.as_deref(),
            )?;
            *spent_ms = spent_ms.saturating_add(40 * (*clicks as u64).max(1));
            Ok(())
        }
        FlowStep::OpenApp { target } => {
            open_app(target)?;
            *spent_ms = spent_ms.saturating_add(200);
            Ok(())
        }
        FlowStep::OpenUrl { url } => {
            open_url(url)?;
            *spent_ms = spent_ms.saturating_add(200);
            Ok(())
        }
        FlowStep::OpenFile { path } => {
            open_file(path)?;
            *spent_ms = spent_ms.saturating_add(200);
            Ok(())
        }
        FlowStep::Shell { command } => {
            run_shell_timeout(command, FlowProgram::SHELL_TIMEOUT_MS)?;
            *spent_ms = spent_ms.saturating_add(300);
            Ok(())
        }
        FlowStep::ActivateApp { bundle_id } => {
            activate_bundle_id(bundle_id)?;
            *preferred_bundle = Some(bundle_id.clone());
            *spent_ms = spent_ms.saturating_add(150);
            thread::sleep(Duration::from_millis(150));
            Ok(())
        }
        FlowStep::PlayMouseMacro { macro_ } => {
            play_mouse_macro(enigo, macro_)?;
            *spent_ms = spent_ms.saturating_add(macro_.pre_delay_ms + 100);
            Ok(())
        }
    }
}

fn play_click(
    enigo: &mut Enigo,
    button: MouseButtonKind,
    x: f32,
    y: f32,
    coord_space: CoordSpace,
    clicks: u8,
    exclude_pid: Option<u32>,
    preferred_bundle_id: Option<&str>,
) -> Result<(), String> {
    let (sx, sy) = target_to_screen(x, y, coord_space, exclude_pid, preferred_bundle_id)?;
    enigo
        .move_mouse(sx, sy, Coordinate::Abs)
        .map_err(|e| format!("移动鼠标失败: {e}"))?;
    thread::sleep(Duration::from_millis(16));
    let times = clicks.max(1);
    for i in 0..times {
        enigo
            .button(to_enigo_button(button), Click)
            .map_err(|e| format!("鼠标点击失败: {e}"))?;
        if i + 1 < times {
            thread::sleep(Duration::from_millis(80));
        }
    }
    Ok(())
}

fn play_hotkey(enigo: &mut Enigo, keys: &[Key]) -> Result<(), String> {
    let mut modifiers = Vec::new();
    let mut main_key = None;

    for key in keys {
        match key {
            Key::Cmd | Key::Ctrl | Key::Alt | Key::Shift => modifiers.push(*key),
            Key::Char(c) => {
                if main_key.is_some() {
                    return Err("快捷键包含多个主键".into());
                }
                main_key = Some(*c);
            }
        }
    }

    let main_key = main_key.ok_or_else(|| "快捷键缺少主键".to_string())?;

    for modifier in &modifiers {
        press_modifier(enigo, *modifier)?;
    }

    enigo
        .key(EnigoKey::Unicode(main_key), Click)
        .map_err(|e| format!("key click failed: {e}"))?;

    for modifier in modifiers.iter().rev() {
        release_modifier(enigo, *modifier)?;
    }
    Ok(())
}

fn press_modifier(enigo: &mut Enigo, key: Key) -> Result<(), String> {
    let enigo_key = match key {
        Key::Cmd => EnigoKey::Meta,
        Key::Ctrl => EnigoKey::Control,
        Key::Alt => EnigoKey::Alt,
        Key::Shift => EnigoKey::Shift,
        Key::Char(_) => return Ok(()),
    };
    enigo
        .key(enigo_key, Press)
        .map_err(|e| format!("modifier press failed: {e}"))
}

fn release_modifier(enigo: &mut Enigo, key: Key) -> Result<(), String> {
    let enigo_key = match key {
        Key::Cmd => EnigoKey::Meta,
        Key::Ctrl => EnigoKey::Control,
        Key::Alt => EnigoKey::Alt,
        Key::Shift => EnigoKey::Shift,
        Key::Char(_) => return Ok(()),
    };
    enigo
        .key(enigo_key, Release)
        .map_err(|e| format!("modifier release failed: {e}"))
}

fn special_to_enigo(key: SpecialKey) -> Result<EnigoKey, String> {
    Ok(match key {
        SpecialKey::Enter => EnigoKey::Return,
        SpecialKey::Tab => EnigoKey::Tab,
        SpecialKey::Escape => EnigoKey::Escape,
        SpecialKey::Space => EnigoKey::Space,
        SpecialKey::Delete => EnigoKey::Delete,
        SpecialKey::Backspace => EnigoKey::Backspace,
        SpecialKey::Up => EnigoKey::UpArrow,
        SpecialKey::Down => EnigoKey::DownArrow,
        SpecialKey::Left => EnigoKey::LeftArrow,
        SpecialKey::Right => EnigoKey::RightArrow,
        SpecialKey::Home => EnigoKey::Home,
        SpecialKey::End => EnigoKey::End,
        SpecialKey::PageUp => EnigoKey::PageUp,
        SpecialKey::PageDown => EnigoKey::PageDown,
        SpecialKey::F1 => EnigoKey::F1,
        SpecialKey::F2 => EnigoKey::F2,
        SpecialKey::F3 => EnigoKey::F3,
        SpecialKey::F4 => EnigoKey::F4,
        SpecialKey::F5 => EnigoKey::F5,
        SpecialKey::F6 => EnigoKey::F6,
        SpecialKey::F7 => EnigoKey::F7,
        SpecialKey::F8 => EnigoKey::F8,
        SpecialKey::F9 => EnigoKey::F9,
        SpecialKey::F10 => EnigoKey::F10,
        SpecialKey::F11 => EnigoKey::F11,
        SpecialKey::F12 => EnigoKey::F12,
    })
}

fn to_enigo_button(button: MouseButtonKind) -> Button {
    match button {
        MouseButtonKind::Left => Button::Left,
        MouseButtonKind::Right => Button::Right,
        MouseButtonKind::Middle => Button::Middle,
    }
}

fn open_app(target: &str) -> Result<(), String> {
    let target = target.trim();
    if target.is_empty() {
        return Err("打开应用目标为空".into());
    }
    let mut cmd = Command::new("open");
    if target.ends_with(".app") || target.starts_with('/') || target.starts_with('~') {
        cmd.arg(target);
    } else if target.contains('.') && !target.contains(' ') {
        cmd.args(["-b", target]);
    } else {
        cmd.args(["-a", target]);
    }
    cmd.spawn()
        .map_err(|e| format!("打开应用 '{target}' 失败: {e}"))?;
    Ok(())
}

fn open_url(url: &str) -> Result<(), String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("链接为空".into());
    }
    Command::new("open")
        .arg(url)
        .spawn()
        .map_err(|e| format!("打开链接失败: {e}"))?;
    Ok(())
}

fn open_file(path: &str) -> Result<(), String> {
    let path = normalize_open_path(path)?;
    Command::new("open")
        .arg(&path)
        .spawn()
        .map_err(|e| format!("打开文件失败: {e}"))?;
    Ok(())
}

fn run_shell_timeout(command: &str, timeout_ms: u64) -> Result<(), String> {
    let command = command.trim();
    if command.is_empty() {
        return Err("命令为空".into());
    }

    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("执行命令失败: {e}"))?;

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    return Ok(());
                }
                return Err(format!("命令退出码非零: {status}"));
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("命令超时（>{timeout_ms}ms）"));
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(format!("等待命令失败: {e}")),
        }
    }
}
