//! 原生 UI → 运行时的事件通道。

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};

use crate::config::{AppConfig, ButtonConfig};

#[derive(Debug, Clone)]
pub enum IconPickTarget {
    CollapsedIcon,
    Button { profile: usize, button: usize },
}

/// 对齐 egui `ButtonMenuAction`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonMenuAction {
    /// 编辑脚本（菜单「编辑…」「编辑脚本…」）
    Edit = 1,
    Enable = 2,
    Disable = 3,
    ClearAction = 4,
    Delete = 5,
    Settings = 6,
    Quit = 7,
    EditAppearance = 8,
}

impl ButtonMenuAction {
    pub fn from_tag_action(code: isize) -> Option<Self> {
        match code {
            1 => Some(Self::Edit),
            2 => Some(Self::Enable),
            3 => Some(Self::Disable),
            4 => Some(Self::ClearAction),
            5 => Some(Self::Delete),
            6 => Some(Self::Settings),
            7 => Some(Self::Quit),
            8 => Some(Self::EditAppearance),
            _ => None,
        }
    }

    pub fn tag_code(self) -> isize {
        self as isize
    }
}

/// 对齐 egui `LauncherMenuAction`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherMenuAction {
    TogglePin = 1,
    ExpandToFull = 2,
    Settings = 3,
    Quit = 4,
}

impl LauncherMenuAction {
    pub fn from_code(code: isize) -> Option<Self> {
        match code {
            1 => Some(Self::TogglePin),
            2 => Some(Self::ExpandToFull),
            3 => Some(Self::Settings),
            4 => Some(Self::Quit),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum NativeUiEvent {
    /// 设置变更已校验：立即 apply，并调度防抖写盘
    ApplySettings(AppConfig),
    /// 关闭设置窗（无回滚、无确认）
    CloseSettings,
    OpenConfigFile,
    /// 导出配置（当前运行时配置）
    ExportConfig(AppConfig),
    /// 导入配置（基于磁盘配置合并/替换）
    ImportConfig,
    /// 查看并恢复自动备份
    ViewBackups,
    #[allow(dead_code)]
    OpenPermissions,
    OpenButtonEditor {
        profile: usize,
        button: usize,
        /// true = 脚本窗，false = 外观窗
        script: bool,
    },
    SaveButton {
        profile: usize,
        button: usize,
        config: ButtonConfig,
    },
    CancelButtonEditor,
    /// 编辑器试运行动作脚本（不写盘）
    TryRunScript {
        source: String,
        attached_macro: Option<crate::config::MouseMacro>,
    },
    /// 开始捕获一次点击，坐标插入脚本
    StartScriptClickCapture,
    /// 取消动作脚本点击捕获
    CancelScriptClickCapture,
    /// 开始录制点击序列（追加到脚本）
    StartScriptMacroRecord,
    /// 结束录制并追加
    FinishScriptMacroRecord,
    StartAppPick,
    CancelAppPick,
    OpenIconPicker(IconPickTarget),
    IconPicked {
        target: IconPickTarget,
        name: String,
    },
    CancelIconPicker,
    RelaunchForPermissions,
    ButtonMenu {
        index: usize,
        action: ButtonMenuAction,
    },
    /// 标题栏猫头：进入折叠模式
    EnterCollapsedMode,
    /// 桌宠双击 / 右键「恢复完整模式」
    ExitCollapsedMode,
    LauncherMenu(LauncherMenuAction),
    /// 悬停桌宠（由 tracking / 点击唤醒展开）
    LauncherInteract,
}

static UI_TX: OnceLock<Mutex<Option<Sender<NativeUiEvent>>>> = OnceLock::new();

fn ui_tx() -> &'static Mutex<Option<Sender<NativeUiEvent>>> {
    UI_TX.get_or_init(|| Mutex::new(None))
}

pub fn install_ui_channel() -> Receiver<NativeUiEvent> {
    let (tx, rx) = mpsc::channel();
    *ui_tx().lock().unwrap() = Some(tx);
    rx
}

pub fn emit(event: NativeUiEvent) {
    if let Ok(guard) = ui_tx().lock() {
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(event);
        }
    }
}

pub fn drain(rx: &Receiver<NativeUiEvent>) -> Vec<NativeUiEvent> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        out.push(ev);
    }
    out
}

/// menu item tag = (action << 16) | button_index
pub fn encode_menu_tag(index: usize, action: ButtonMenuAction) -> isize {
    (action.tag_code() << 16) | (index as isize & 0xFFFF)
}

pub fn decode_menu_tag(tag: isize) -> Option<(usize, ButtonMenuAction)> {
    let index = (tag & 0xFFFF) as usize;
    let action = ButtonMenuAction::from_tag_action((tag >> 16) & 0xFF)?;
    Some((index, action))
}
