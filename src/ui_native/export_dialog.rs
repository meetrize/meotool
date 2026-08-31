//! 配置导出对话框（NSAlert + NSSavePanel）。

use std::path::PathBuf;

use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertSecondButtonReturn, NSAlertStyle, NSModalResponseOK,
    NSSavePanel,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSString};

use crate::config::{
    default_bundle_dirname, export_bundle, export_config_toml, AppConfig,
};

/// 弹出导出方式选择，并执行导出。返回用户可见摘要；取消时返回 `None`。
pub fn run_export_flow(
    mtm: MainThreadMarker,
    app: &AppConfig,
    icon_dirs: &[PathBuf],
) -> Option<Result<String, String>> {
    let mode = choose_export_mode(mtm)?;
    Some(match mode {
        ExportMode::Toml => export_toml_with_panel(mtm, app),
        ExportMode::Bundle => export_bundle_with_panel(mtm, app, icon_dirs),
    })
}

#[derive(Clone, Copy)]
enum ExportMode {
    Toml,
    Bundle,
}

fn choose_export_mode(mtm: MainThreadMarker) -> Option<ExportMode> {
    let alert = NSAlert::new(mtm);
    unsafe {
        alert.setAlertStyle(NSAlertStyle::Informational);
        alert.setMessageText(&NSString::from_str("导出配置"));
        alert.setInformativeText(&NSString::from_str(
            "「仅配置文件」导出单个 TOML。\n「完整包」额外包含引用到的图标与 manifest.json。",
        ));
        alert.addButtonWithTitle(&NSString::from_str("仅配置文件"));
        alert.addButtonWithTitle(&NSString::from_str("完整包"));
        alert.addButtonWithTitle(&NSString::from_str("取消"));
        let resp = alert.runModal();
        if resp == NSAlertFirstButtonReturn {
            Some(ExportMode::Toml)
        } else if resp == NSAlertSecondButtonReturn {
            Some(ExportMode::Bundle)
        } else {
            None
        }
    }
}

fn export_toml_with_panel(mtm: MainThreadMarker, app: &AppConfig) -> Result<String, String> {
    let panel = NSSavePanel::savePanel(mtm);
    unsafe {
        panel.setTitle(Some(&NSString::from_str("导出配置文件")));
        panel.setMessage(Some(&NSString::from_str("保存为 TOML 配置文件")));
        panel.setPrompt(Some(&NSString::from_str("导出")));
        panel.setNameFieldStringValue(&NSString::from_str("meotool-config.toml"));
        #[allow(deprecated)]
        {
            let toml_ext = NSString::from_str("toml");
            let txt_ext = NSString::from_str("txt");
            panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&[
                toml_ext, txt_ext,
            ])));
        }
        panel.setCanCreateDirectories(true);
        panel.setExtensionHidden(false);
    }
    if panel.runModal() != NSModalResponseOK {
        return Err("cancelled".into());
    }
    let path = panel_path(&panel).ok_or_else(|| "未选择保存路径".to_string())?;
    export_config_toml(app, &path)?;
    Ok(format!("已导出配置文件：\n{}", path.display()))
}

fn export_bundle_with_panel(
    mtm: MainThreadMarker,
    app: &AppConfig,
    icon_dirs: &[PathBuf],
) -> Result<String, String> {
    let panel = NSSavePanel::savePanel(mtm);
    let default_name = default_bundle_dirname();
    unsafe {
        panel.setTitle(Some(&NSString::from_str("导出完整配置包")));
        panel.setMessage(Some(&NSString::from_str(
            "将创建一个文件夹，内含 config.toml、icons/ 与 manifest.json",
        )));
        panel.setPrompt(Some(&NSString::from_str("导出")));
        panel.setNameFieldStringValue(&NSString::from_str(&default_name));
        panel.setCanCreateDirectories(true);
        panel.setExtensionHidden(true);
    }
    if panel.runModal() != NSModalResponseOK {
        return Err("cancelled".into());
    }
    let path = panel_path(&panel).ok_or_else(|| "未选择保存路径".to_string())?;
    let result = export_bundle(app, icon_dirs, &path)?;
    let mut msg = format!(
        "已导出完整包：\n{}\n\n图标：已复制 {} 个",
        result.path.display(),
        result.icons_copied.len()
    );
    if !result.icons_missing.is_empty() {
        msg.push_str(&format!(
            "；未找到 {} 个（{}",
            result.icons_missing.len(),
            result.icons_missing.join(", ")
        ));
        msg.push(')');
    }
    Ok(msg)
}

fn panel_path(panel: &NSSavePanel) -> Option<PathBuf> {
    let url = panel.URL()?;
    let path = url.path()?.to_string();
    Some(PathBuf::from(path))
}

pub fn show_export_result(mtm: MainThreadMarker, result: Result<String, String>) {
    if matches!(&result, Err(e) if e == "cancelled") {
        return;
    }
    let alert = NSAlert::new(mtm);
    unsafe {
        match result {
            Ok(msg) => {
                alert.setAlertStyle(NSAlertStyle::Informational);
                alert.setMessageText(&NSString::from_str("导出成功"));
                alert.setInformativeText(&NSString::from_str(&msg));
            }
            Err(err) => {
                alert.setAlertStyle(NSAlertStyle::Warning);
                alert.setMessageText(&NSString::from_str("导出失败"));
                alert.setInformativeText(&NSString::from_str(&err));
            }
        }
        alert.addButtonWithTitle(&NSString::from_str("好"));
        let _ = alert.runModal();
    }
}
