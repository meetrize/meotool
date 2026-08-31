//! 配置导入对话框（NSOpenPanel + NSAlert）。

use std::path::{Path, PathBuf};

use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertSecondButtonReturn, NSAlertStyle,
    NSAlertThirdButtonReturn, NSModalResponseOK, NSOpenPanel,
};
use objc2_foundation::{MainThreadMarker, NSString};

use crate::config::{
    app_data_dir, import_config, import_result_summary, preview_import, AppConfig, ImportMode,
    ImportResult, ProfileConflictStrategy,
};

/// 弹出导入流程；成功返回新配置（已写盘）。
pub fn run_import_flow(
    mtm: MainThreadMarker,
    current: &AppConfig,
    config_path: &Path,
) -> Option<Result<ImportResult, String>> {
    let source = pick_import_source(mtm)?;
    let preview = match preview_import(&source, current) {
        Ok(p) => p,
        Err(err) => return Some(Err(err)),
    };

    let mode = match choose_import_mode(mtm, &preview)? {
        Some(m) => m,
        None => return Some(Err("cancelled".into())),
    };

    let conflict_strategy = if mode == ImportMode::Merge && !preview.conflicting_profiles.is_empty()
    {
        match choose_conflict_strategy(mtm, &preview.conflicting_profiles)? {
            Some(s) => s,
            None => return Some(Err("cancelled".into())),
        }
    } else {
        ProfileConflictStrategy::Skip
    };

    let user_icons = app_data_dir().join("icons");
    Some(import_config(
        &source,
        config_path,
        &user_icons,
        current,
        mode,
        conflict_strategy,
    ))
}

fn pick_import_source(mtm: MainThreadMarker) -> Option<PathBuf> {
    let panel = NSOpenPanel::openPanel(mtm);
    unsafe {
        panel.setTitle(Some(&NSString::from_str("导入配置")));
        panel.setMessage(Some(&NSString::from_str(
            "选择配置文件（.toml）或完整导出包文件夹",
        )));
        panel.setPrompt(Some(&NSString::from_str("选择")));
        panel.setCanChooseFiles(true);
        panel.setCanChooseDirectories(true);
        panel.setAllowsMultipleSelection(false);
        panel.setCanCreateDirectories(false);
    }
    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    panel_path(&panel)
}

fn choose_import_mode(
    mtm: MainThreadMarker,
    preview: &crate::config::ImportPreview,
) -> Option<Option<ImportMode>> {
    let mut info = format!(
        "配置档 {} 个，按钮 {} 个",
        preview.profile_count, preview.button_count
    );
    if preview.icon_file_count > 0 {
        info.push_str(&format!("，图标 {} 个", preview.icon_file_count));
    }
    if !preview.conflicting_profiles.is_empty() {
        info.push_str(&format!(
            "\n\n与当前配置同名配置档：{}",
            preview.conflicting_profiles.join("、")
        ));
    }

    let alert = NSAlert::new(mtm);
    unsafe {
        alert.setAlertStyle(NSAlertStyle::Informational);
        alert.setMessageText(&NSString::from_str("确认导入"));
        alert.setInformativeText(&NSString::from_str(&info));
        alert.addButtonWithTitle(&NSString::from_str("替换全部"));
        alert.addButtonWithTitle(&NSString::from_str("合并配置档"));
        alert.addButtonWithTitle(&NSString::from_str("取消"));
        let resp = alert.runModal();
        if resp == NSAlertFirstButtonReturn {
            Some(Some(ImportMode::Replace))
        } else if resp == NSAlertSecondButtonReturn {
            Some(Some(ImportMode::Merge))
        } else {
            Some(None)
        }
    }
}

fn choose_conflict_strategy(
    mtm: MainThreadMarker,
    conflicts: &[String],
) -> Option<Option<ProfileConflictStrategy>> {
    let alert = NSAlert::new(mtm);
    let text = format!(
        "以下配置档与当前配置同名：\n{}\n\n请选择处理方式。",
        conflicts.join("、")
    );
    unsafe {
        alert.setAlertStyle(NSAlertStyle::Warning);
        alert.setMessageText(&NSString::from_str("同名配置档"));
        alert.setInformativeText(&NSString::from_str(&text));
        alert.addButtonWithTitle(&NSString::from_str("覆盖同名"));
        alert.addButtonWithTitle(&NSString::from_str("重命名导入项"));
        alert.addButtonWithTitle(&NSString::from_str("跳过同名"));
        alert.addButtonWithTitle(&NSString::from_str("取消"));
        let resp = alert.runModal();
        if resp == NSAlertFirstButtonReturn {
            Some(Some(ProfileConflictStrategy::Overwrite))
        } else if resp == NSAlertSecondButtonReturn {
            Some(Some(ProfileConflictStrategy::RenameImported))
        } else if resp == NSAlertThirdButtonReturn {
            Some(Some(ProfileConflictStrategy::Skip))
        } else {
            Some(None)
        }
    }
}

fn panel_path(panel: &NSOpenPanel) -> Option<PathBuf> {
    let url = panel.URLs().firstObject()?;
    let path = url.path()?.to_string();
    Some(PathBuf::from(path))
}

pub fn show_import_result(mtm: MainThreadMarker, result: Result<ImportResult, String>) {
    if matches!(&result, Err(e) if e == "cancelled") {
        return;
    }
    let alert = NSAlert::new(mtm);
    unsafe {
        match result {
            Ok(imported) => {
                alert.setAlertStyle(NSAlertStyle::Informational);
                alert.setMessageText(&NSString::from_str("导入成功"));
                alert.setInformativeText(&NSString::from_str(&import_result_summary(
                    &imported,
                )));
            }
            Err(err) => {
                alert.setAlertStyle(NSAlertStyle::Warning);
                alert.setMessageText(&NSString::from_str("导入失败"));
                alert.setInformativeText(&NSString::from_str(&err));
            }
        }
        alert.addButtonWithTitle(&NSString::from_str("好"));
        let _ = alert.runModal();
    }
}
