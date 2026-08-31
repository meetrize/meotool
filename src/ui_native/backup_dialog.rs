//! 配置备份列表与恢复对话框。

use std::path::{Path, PathBuf};

use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertSecondButtonReturn, NSAlertStyle,
    NSModalResponseOK, NSOpenPanel,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSString, NSURL};

use crate::config::{
    backup_before_write, backups_dir, format_backup_entry, list_backups, parse_config_content,
    restore_from_backup, AppConfig, BackupEntry,
};

/// 备份管理流程；成功返回恢复后的配置（已写盘）。
pub fn run_backup_restore_flow(
    mtm: MainThreadMarker,
    config_path: &Path,
) -> Option<Result<AppConfig, String>> {
    let backups = match list_backups(config_path) {
        Ok(v) => v,
        Err(err) => return Some(Err(err)),
    };

    if backups.is_empty() {
        let alert = NSAlert::new(mtm);
        unsafe {
            alert.setAlertStyle(NSAlertStyle::Informational);
            alert.setMessageText(&NSString::from_str("暂无备份"));
            alert.setInformativeText(&NSString::from_str(
                "自动备份会在每次保存配置时创建，保存在 backups/ 目录。",
            ));
            alert.addButtonWithTitle(&NSString::from_str("好"));
            let _ = alert.runModal();
        }
        return None;
    }

    let action = choose_backup_action(mtm, &backups)?;
    let backup_path = match action {
        BackupAction::RestoreLatest => backups[0].path.clone(),
        BackupAction::PickFile => pick_backup_file(mtm, config_path)?,
        BackupAction::Cancel => return Some(Err("cancelled".into())),
    };

    confirm_and_restore(mtm, &backup_path, config_path)
}

#[derive(Clone, Copy)]
enum BackupAction {
    RestoreLatest,
    PickFile,
    Cancel,
}

fn choose_backup_action(
    mtm: MainThreadMarker,
    backups: &[BackupEntry],
) -> Option<BackupAction> {
    let mut lines = Vec::new();
    for (i, entry) in backups.iter().take(10).enumerate() {
        lines.push(format!("{}. {}", i + 1, format_backup_entry(entry)));
    }
    if backups.len() > 10 {
        lines.push(format!("… 共 {} 份备份", backups.len()));
    }

    let alert = NSAlert::new(mtm);
    unsafe {
        alert.setAlertStyle(NSAlertStyle::Informational);
        alert.setMessageText(&NSString::from_str("配置备份"));
        alert.setInformativeText(&NSString::from_str(&lines.join("\n")));
        alert.addButtonWithTitle(&NSString::from_str("恢复最新"));
        alert.addButtonWithTitle(&NSString::from_str("选择备份…"));
        alert.addButtonWithTitle(&NSString::from_str("取消"));
        let resp = alert.runModal();
        if resp == NSAlertFirstButtonReturn {
            Some(BackupAction::RestoreLatest)
        } else if resp == NSAlertSecondButtonReturn {
            Some(BackupAction::PickFile)
        } else {
            Some(BackupAction::Cancel)
        }
    }
}

fn pick_backup_file(mtm: MainThreadMarker, config_path: &Path) -> Option<PathBuf> {
    let panel = NSOpenPanel::openPanel(mtm);
    let dir = backups_dir(config_path);
    unsafe {
        panel.setTitle(Some(&NSString::from_str("选择备份文件")));
        panel.setMessage(Some(&NSString::from_str("从 backups/ 目录选择要恢复的 .toml")));
        panel.setPrompt(Some(&NSString::from_str("恢复")));
        panel.setCanChooseFiles(true);
        panel.setCanChooseDirectories(false);
        panel.setAllowsMultipleSelection(false);
        if dir.is_dir() {
            let url = NSURL::fileURLWithPath(&NSString::from_str(&dir.to_string_lossy()));
            panel.setDirectoryURL(Some(&url));
        }
        #[allow(deprecated)]
        {
            let toml = NSString::from_str("toml");
            panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&[toml])));
        }
    }
    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    let url = panel.URLs().firstObject()?;
    Some(PathBuf::from(url.path()?.to_string()))
}

fn confirm_and_restore(
    mtm: MainThreadMarker,
    backup_path: &Path,
    config_path: &Path,
) -> Option<Result<AppConfig, String>> {
    let label = backup_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| backup_path.display().to_string());

    let alert = NSAlert::new(mtm);
    unsafe {
        alert.setAlertStyle(NSAlertStyle::Warning);
        alert.setMessageText(&NSString::from_str("确认恢复备份？"));
        alert.setInformativeText(&NSString::from_str(&format!(
            "将用备份「{label}」替换当前配置。\n当前配置会先备份，再写入恢复内容。"
        )));
        alert.addButtonWithTitle(&NSString::from_str("恢复"));
        alert.addButtonWithTitle(&NSString::from_str("取消"));
        if alert.runModal() != NSAlertFirstButtonReturn {
            return Some(Err("cancelled".into()));
        }
    }

    Some(restore_and_load(backup_path, config_path))
}

fn restore_and_load(backup_path: &Path, config_path: &Path) -> Result<AppConfig, String> {
    if config_path.exists() {
        let _ = backup_before_write(config_path);
    }
    restore_from_backup(backup_path, config_path)?;
    let content = std::fs::read_to_string(config_path)
        .map_err(|e| format!("read restored config: {e}"))?;
    parse_config_content(&content)
}

pub fn show_backup_restore_result(mtm: MainThreadMarker, result: Result<(), String>) {
    if matches!(&result, Err(e) if e == "cancelled") {
        return;
    }
    let alert = NSAlert::new(mtm);
    unsafe {
        match result {
            Ok(_) => {
                alert.setAlertStyle(NSAlertStyle::Informational);
                alert.setMessageText(&NSString::from_str("恢复成功"));
                alert.setInformativeText(&NSString::from_str(
                    "配置已从备份恢复并立即生效。",
                ));
            }
            Err(err) => {
                alert.setAlertStyle(NSAlertStyle::Warning);
                alert.setMessageText(&NSString::from_str("恢复失败"));
                alert.setInformativeText(&NSString::from_str(&err));
            }
        }
        alert.addButtonWithTitle(&NSString::from_str("好"));
        let _ = alert.runModal();
    }
}
