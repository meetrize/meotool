//! 配置导入 / 导出（单文件 TOML / 含图标的完整包）。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::app_config::{AppConfig, Profile};
use super::backup::atomic_write;
use super::export::{app_config_to_file, save_app_config};

pub const EXPORT_FORMAT: &str = "meotool-export";
pub const EXPORT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct ExportBundleResult {
    pub path: PathBuf,
    pub icons_copied: Vec<String>,
    pub icons_missing: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ImportSource {
    pub config_path: PathBuf,
    pub icons_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ImportPreview {
    pub source: ImportSource,
    pub app_config: AppConfig,
    pub profile_count: usize,
    pub button_count: usize,
    pub icon_file_count: usize,
    pub conflicting_profiles: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    Replace,
    Merge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileConflictStrategy {
    Overwrite,
    RenameImported,
    Skip,
}

#[derive(Debug, Clone)]
pub struct ImportResult {
    pub app_config: AppConfig,
    pub mode: ImportMode,
    pub icons_copied: Vec<String>,
    pub profiles_added: usize,
    pub profiles_overwritten: usize,
    pub profiles_renamed: usize,
    pub profiles_skipped: usize,
}

/// 解析并校验 TOML 文本，返回友好错误（含大致行号）。
pub fn validate_config_str(content: &str) -> Result<AppConfig, String> {
    super::parse::parse_config_content(content)
}

/// 校验配置文件路径。
pub fn validate_config_file(path: &Path) -> Result<AppConfig, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("无法读取 {}: {e}", path.display()))?;
    validate_config_str(&content)
}

/// 识别导入源：单个 `.toml` 或完整包目录（含 `config.toml`）。
pub fn resolve_import_source(path: &Path) -> Result<ImportSource, String> {
    if path.is_file() {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext != "toml" {
            return Err(format!(
                "不支持的文件类型（需 .toml）: {}",
                path.display()
            ));
        }
        return Ok(ImportSource {
            config_path: path.to_path_buf(),
            icons_dir: None,
        });
    }

    if !path.is_dir() {
        return Err(format!("路径不存在: {}", path.display()));
    }

    let config_path = path.join("config.toml");
    if !config_path.is_file() {
        return Err(format!(
            "目录中未找到 config.toml: {}",
            path.display()
        ));
    }

    let icons_dir = path.join("icons");
    Ok(ImportSource {
        config_path,
        icons_dir: icons_dir.is_dir().then_some(icons_dir),
    })
}

/// 预览导入内容（不写盘）。
pub fn preview_import(path: &Path, current: &AppConfig) -> Result<ImportPreview, String> {
    let source = resolve_import_source(path)?;
    let app_config = validate_config_file(&source.config_path)?;
    let profile_count = app_config.profiles.len();
    let button_count: usize = app_config
        .profiles
        .iter()
        .map(|p| p.buttons.len())
        .sum();
    let icon_file_count = source
        .icons_dir
        .as_ref()
        .map(|dir| count_icon_files(dir.as_path()))
        .transpose()?
        .unwrap_or(0);
    let conflicting_profiles = find_profile_conflicts(current, &app_config);

    Ok(ImportPreview {
        source,
        app_config,
        profile_count,
        button_count,
        icon_file_count,
        conflicting_profiles,
    })
}

fn count_icon_files(dir: &Path) -> Result<usize, String> {
    let mut n = 0;
    for entry in fs::read_dir(dir).map_err(|e| format!("读取 icons 目录: {e}"))? {
        let entry = entry.map_err(|e| format!("读取 icons 条目: {e}"))?;
        if entry.path().is_file() {
            n += 1;
        }
    }
    Ok(n)
}

/// 与当前配置同名的 profile 列表。
pub fn find_profile_conflicts(current: &AppConfig, imported: &AppConfig) -> Vec<String> {
    let current_names: BTreeSet<_> = current.profiles.iter().map(|p| p.name.as_str()).collect();
    imported
        .profiles
        .iter()
        .filter(|p| current_names.contains(p.name.as_str()))
        .map(|p| p.name.clone())
        .collect()
}

/// 合并 profile 列表；全局面板 / 热键等保留 `current`。
pub fn merge_profiles(
    current: &AppConfig,
    imported: &AppConfig,
    strategy: ProfileConflictStrategy,
) -> (AppConfig, usize, usize, usize, usize) {
    let mut out = current.clone();
    let mut added = 0usize;
    let mut overwritten = 0usize;
    let mut renamed = 0usize;
    let mut skipped = 0usize;

    for mut profile in imported.profiles.clone() {
        if let Some(idx) = out.profiles.iter().position(|p| p.name == profile.name) {
            match strategy {
                ProfileConflictStrategy::Overwrite => {
                    out.profiles[idx] = profile;
                    overwritten += 1;
                }
                ProfileConflictStrategy::RenameImported => {
                    profile.name = unique_profile_name(&out.profiles, &profile.name);
                    out.profiles.push(profile);
                    renamed += 1;
                }
                ProfileConflictStrategy::Skip => {
                    skipped += 1;
                }
            }
        } else {
            out.profiles.push(profile);
            added += 1;
        }
    }

    (out, added, overwritten, renamed, skipped)
}

fn unique_profile_name(existing: &[Profile], base: &str) -> String {
    let first = format!("{base} (导入)");
    if !existing.iter().any(|p| p.name == first) {
        return first;
    }
    for i in 2..100 {
        let name = format!("{base} (导入 {i})");
        if !existing.iter().any(|p| p.name == name) {
            return name;
        }
    }
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{base} (导入 {secs})")
}

/// 将导入包中的 icons/ 复制到用户目录。
pub fn copy_import_icons(icons_src: &Path, user_icons_dir: &Path) -> Result<Vec<String>, String> {
    if !icons_src.is_dir() {
        return Ok(Vec::new());
    }
    fs::create_dir_all(user_icons_dir).map_err(|e| format!("创建 icons 目录: {e}"))?;
    let mut copied = Vec::new();
    for entry in fs::read_dir(icons_src).map_err(|e| format!("读取 icons 目录: {e}"))? {
        let entry = entry.map_err(|e| format!("读取 icons 条目: {e}"))?;
        let src = entry.path();
        if !src.is_file() {
            continue;
        }
        let Some(name) = src.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let dest = user_icons_dir.join(name);
        fs::copy(&src, &dest)
            .map_err(|e| format!("复制图标 {} -> {}: {e}", src.display(), dest.display()))?;
        copied.push(name.to_string());
    }
    copied.sort();
    Ok(copied)
}

/// 执行导入：校验 → 合并/替换 → 备份 → 写盘 → 复制图标。
pub fn import_config(
    source_path: &Path,
    config_path: &Path,
    user_icons_dir: &Path,
    current: &AppConfig,
    mode: ImportMode,
    conflict_strategy: ProfileConflictStrategy,
) -> Result<ImportResult, String> {
    let preview = preview_import(source_path, current)?;
    let imported = preview.app_config;

    let (app_config, added, overwritten, renamed, skipped) = match mode {
        ImportMode::Replace => (imported, 0, preview.profile_count, 0, 0),
        ImportMode::Merge => merge_profiles(current, &imported, conflict_strategy),
    };

    save_app_config(config_path, &app_config)?;

    let icons_copied = preview
        .source
        .icons_dir
        .as_ref()
        .map(|dir| copy_import_icons(dir, user_icons_dir))
        .transpose()?
        .unwrap_or_default();

    log::info!(
        "imported config from {} mode={mode:?} icons={}",
        source_path.display(),
        icons_copied.len()
    );

    Ok(ImportResult {
        app_config,
        mode,
        icons_copied,
        profiles_added: added,
        profiles_overwritten: overwritten,
        profiles_renamed: renamed,
        profiles_skipped: skipped,
    })
}

pub fn import_result_summary(result: &ImportResult) -> String {
    let mut lines = vec![format!(
        "已{}配置。",
        if result.mode == ImportMode::Replace {
            "替换"
        } else {
            "合并"
        }
    )];
    if result.mode == ImportMode::Merge {
        let mut parts = Vec::new();
        if result.profiles_added > 0 {
            parts.push(format!("新增 {} 个配置档", result.profiles_added));
        }
        if result.profiles_overwritten > 0 {
            parts.push(format!("覆盖 {} 个", result.profiles_overwritten));
        }
        if result.profiles_renamed > 0 {
            parts.push(format!("重命名导入 {} 个", result.profiles_renamed));
        }
        if result.profiles_skipped > 0 {
            parts.push(format!("跳过 {} 个", result.profiles_skipped));
        }
        if !parts.is_empty() {
            lines.push(parts.join("；"));
        }
    }
    if !result.icons_copied.is_empty() {
        lines.push(format!("已复制 {} 个图标。", result.icons_copied.len()));
    }
    lines.join("\n")
}

/// 导出为单个 TOML 文件（不触发用户配置备份）。
pub fn export_config_toml(app: &AppConfig, path: &Path) -> Result<(), String> {
    let file = app_config_to_file(app);
    let content = toml::to_string_pretty(&file).map_err(|e| format!("serialize config: {e}"))?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| format!("create export dir: {e}"))?;
        }
    }
    atomic_write(path, content)?;
    log::info!("exported config toml to {}", path.display());
    Ok(())
}

/// 规范化图标文件名（补全 `.png`）。
pub fn normalize_icon_name(name: &str) -> Option<String> {
    let t = name.trim();
    if t.is_empty() {
        return None;
    }
    if t.ends_with(".png") || t.ends_with(".PNG") {
        Some(t.to_string())
    } else {
        Some(format!("{t}.png"))
    }
}

/// 收集配置中引用到的图标名（去重、排序）。
pub fn referenced_icon_names(app: &AppConfig) -> Vec<String> {
    let mut set = BTreeSet::new();
    if let Some(name) = app
        .global_panel
        .settings
        .collapsed_icon
        .as_deref()
        .and_then(normalize_icon_name)
    {
        set.insert(name);
    }
    for profile in &app.profiles {
        if let Some(name) = profile
            .panel
            .collapsed_icon
            .as_deref()
            .and_then(normalize_icon_name)
        {
            set.insert(name);
        }
        for button in &profile.buttons {
            if let Some(name) = button.icon.as_deref().and_then(normalize_icon_name) {
                set.insert(name);
            }
        }
    }
    set.into_iter().collect()
}

/// 按 `icon_dirs` 优先级查找图标文件。
pub fn find_icon_file(name: &str, icon_dirs: &[PathBuf]) -> Option<PathBuf> {
    let name = normalize_icon_name(name)?;
    for dir in icon_dirs {
        let path = dir.join(&name);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// 默认完整包目录名：`meotool-export-<unix_secs>`。
pub fn default_bundle_dirname() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("meotool-export-{secs}")
}

/// 导出完整包到目标目录：
/// ```text
/// dest/
///   config.toml
///   icons/
///   manifest.json
/// ```
pub fn export_bundle(
    app: &AppConfig,
    icon_dirs: &[PathBuf],
    dest_dir: &Path,
) -> Result<ExportBundleResult, String> {
    if dest_dir.exists() {
        if dest_dir.is_file() {
            return Err(format!(
                "export path is a file, expected directory: {}",
                dest_dir.display()
            ));
        }
        let non_empty = fs::read_dir(dest_dir)
            .map_err(|e| format!("read export dir: {e}"))?
            .next()
            .is_some();
        if non_empty {
            return Err(format!(
                "export directory is not empty: {}",
                dest_dir.display()
            ));
        }
    } else {
        fs::create_dir_all(dest_dir).map_err(|e| format!("create export dir: {e}"))?;
    }

    let icons_dir = dest_dir.join("icons");
    fs::create_dir_all(&icons_dir).map_err(|e| format!("create icons dir: {e}"))?;

    export_config_toml(app, &dest_dir.join("config.toml"))?;

    let names = referenced_icon_names(app);
    let mut icons_copied = Vec::new();
    let mut icons_missing = Vec::new();
    for name in names {
        match find_icon_file(&name, icon_dirs) {
            Some(src) => {
                let dest = icons_dir.join(&name);
                fs::copy(&src, &dest).map_err(|e| {
                    format!("copy icon {} -> {}: {e}", src.display(), dest.display())
                })?;
                icons_copied.push(name);
            }
            None => icons_missing.push(name),
        }
    }

    write_manifest(dest_dir, &icons_copied, &icons_missing)?;

    log::info!(
        "exported config bundle to {} (icons={} missing={})",
        dest_dir.display(),
        icons_copied.len(),
        icons_missing.len()
    );

    Ok(ExportBundleResult {
        path: dest_dir.to_path_buf(),
        icons_copied,
        icons_missing,
    })
}

fn write_manifest(
    dest_dir: &Path,
    icons_copied: &[String],
    icons_missing: &[String],
) -> Result<(), String> {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!("  \"format\": \"{EXPORT_FORMAT}\",\n"));
    out.push_str(&format!("  \"version\": {EXPORT_FORMAT_VERSION},\n"));
    out.push_str(&format!("  \"exported_at_unix_ms\": {ms},\n"));
    out.push_str("  \"icons\": ");
    out.push_str(&json_string_array(icons_copied));
    out.push_str(",\n");
    out.push_str("  \"icons_missing\": ");
    out.push_str(&json_string_array(icons_missing));
    out.push('\n');
    out.push_str("}\n");

    let path = dest_dir.join("manifest.json");
    fs::write(&path, out).map_err(|e| format!("write manifest: {e}"))?;
    Ok(())
}

fn json_string_array(items: &[String]) -> String {
    let mut s = String::from("[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        s.push('"');
        s.push_str(&escape_json_string(item));
        s.push('"');
    }
    s.push(']');
    s
}

fn escape_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::{empty_slot_button, Action};
    use crate::config::toml_model::ConfigFile;
    use crate::config::LoadedConfig;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "meotool-import-export-test-{}-{}-{}",
            std::process::id(),
            n,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn minimal_toml(name: &str, label: &str) -> String {
        format!(
            r#"
[[profiles]]
name = "{name}"

[[profiles.buttons]]
label = "{label}"
enabled = true

[profiles.buttons.action]
type = "none"
"#
        )
    }

    fn app_from_toml(toml: &str) -> AppConfig {
        let file: ConfigFile = toml::from_str(toml).unwrap();
        file.into_app_config().unwrap()
    }

    #[test]
    fn validate_config_reports_line_on_error() {
        let err = validate_config_str("[[profiles]]\nname = \n").unwrap_err();
        assert!(err.contains("TOML") || err.contains("解析"), "{err}");
    }

    #[test]
    fn resolve_import_source_file_and_bundle() {
        let dir = temp_dir();
        let toml = dir.join("solo.toml");
        fs::write(&toml, minimal_toml("default", "A")).unwrap();
        let src = resolve_import_source(&toml).unwrap();
        assert_eq!(src.config_path, toml);
        assert!(src.icons_dir.is_none());

        let bundle = dir.join("bundle");
        fs::create_dir_all(bundle.join("icons")).unwrap();
        fs::write(bundle.join("config.toml"), minimal_toml("default", "B")).unwrap();
        fs::write(bundle.join("icons/x.png"), b"x").unwrap();
        let src = resolve_import_source(&bundle).unwrap();
        assert!(src.icons_dir.is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_profiles_rename_and_add() {
        let current = app_from_toml(&minimal_toml("default", "old"));
        let imported = app_from_toml(&minimal_toml("default", "new"));
        let (merged, added, overwritten, renamed, skipped) =
            merge_profiles(&current, &imported, ProfileConflictStrategy::RenameImported);
        assert_eq!(merged.profiles.len(), 2);
        assert_eq!(renamed, 1);
        assert_eq!(added, 0);
        assert!(merged.profiles.iter().any(|p| p.name.contains("导入")));

        let imported_extra = app_from_toml(&minimal_toml("extra", "x"));
        let (merged2, added2, _, _, _) =
            merge_profiles(&current, &imported_extra, ProfileConflictStrategy::Skip);
        assert_eq!(added2, 1);
        assert_eq!(merged2.profiles.len(), 2);
        let _ = added;
        let _ = overwritten;
        let _ = skipped;
    }

    #[test]
    fn import_replace_and_copy_icons() {
        let dir = temp_dir();
        let config_path = dir.join("config.toml");
        let icons_dest = dir.join("user_icons");
        let current = app_from_toml(&minimal_toml("default", "keep"));

        let bundle = dir.join("bundle");
        fs::create_dir_all(bundle.join("icons")).unwrap();
        fs::write(
            bundle.join("config.toml"),
            minimal_toml("default", "imported"),
        )
        .unwrap();
        fs::write(bundle.join("icons/copy.png"), b"png").unwrap();

        save_app_config(&config_path, &current).unwrap();
        let result = import_config(
            &bundle,
            &config_path,
            &icons_dest,
            &current,
            ImportMode::Replace,
            ProfileConflictStrategy::Skip,
        )
        .unwrap();
        assert_eq!(
            result.app_config.profiles[0].buttons[0].label,
            "imported"
        );
        assert!(icons_dest.join("copy.png").is_file());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_writes_version_field() {
        use crate::config::export::save_app_config;
        use crate::config::migrate::CURRENT_CONFIG_VERSION;
        use crate::config::toml_model::ConfigFile;
        let dir = temp_dir();
        let path = dir.join("config.toml");
        let app = app_from_toml(&minimal_toml("default", "A"));
        save_app_config(&path, &app).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("version"));
        let file: ConfigFile = toml::from_str(&content).unwrap();
        assert_eq!(file.version, CURRENT_CONFIG_VERSION);
        assert!(file.updated_at.is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    fn sample_app(icon: &str) -> AppConfig {
        let toml = format!(
            r#"
[[profiles]]
name = "default"

[[profiles.buttons]]
label = "A"
icon = "{icon}"
enabled = true

[profiles.buttons.action]
type = "none"
"#
        );
        let file: ConfigFile = toml::from_str(&toml).unwrap();
        file.into_app_config().unwrap()
    }

    #[test]
    fn normalize_and_collect_icons() {
        assert_eq!(normalize_icon_name("copy").as_deref(), Some("copy.png"));
        assert_eq!(normalize_icon_name("x.png").as_deref(), Some("x.png"));
        assert_eq!(normalize_icon_name("  ").as_deref(), None);

        let mut app = sample_app("pet-cat");
        app.global_panel.settings.collapsed_icon = Some("pet-cat.png".into());
        let names = referenced_icon_names(&app);
        assert_eq!(names, vec!["pet-cat.png".to_string()]);
    }

    #[test]
    fn export_toml_roundtrip() {
        let dir = temp_dir();
        let path = dir.join("out.toml");
        let app = sample_app("copy.png");
        export_config_toml(&app, &path).unwrap();
        let loaded = LoadedConfig::load(Some(&path)).unwrap();
        assert_eq!(loaded.app_config.profiles[0].buttons[0].label, "A");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_bundle_copies_icons_and_writes_manifest() {
        let dir = temp_dir();
        let icons_src = dir.join("icon_src");
        fs::create_dir_all(&icons_src).unwrap();
        fs::write(icons_src.join("copy.png"), b"png").unwrap();

        let app = sample_app("copy.png");
        let mut missing_app = app;
        let mut btn = empty_slot_button();
        btn.label = "B".into();
        btn.enabled = true;
        btn.icon = Some("nope.png".into());
        btn.action = Action::None;
        missing_app.profiles[0].buttons.push(btn);

        let dest = dir.join("bundle");
        let result = export_bundle(&missing_app, &[icons_src], &dest).unwrap();
        assert!(dest.join("config.toml").is_file());
        assert!(dest.join("icons/copy.png").is_file());
        assert!(!dest.join("icons/nope.png").exists());
        assert_eq!(result.icons_copied, vec!["copy.png".to_string()]);
        assert_eq!(result.icons_missing, vec!["nope.png".to_string()]);

        let manifest = fs::read_to_string(dest.join("manifest.json")).unwrap();
        assert!(manifest.contains("\"format\": \"meotool-export\""));
        assert!(manifest.contains("copy.png"));
        assert!(manifest.contains("nope.png"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_bundle_rejects_non_empty_dest() {
        let dir = temp_dir();
        let dest = dir.join("bundle");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("keep.txt"), b"x").unwrap();
        let app = sample_app("copy.png");
        let err = export_bundle(&app, &[], &dest).unwrap_err();
        assert!(err.contains("not empty"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_icon_respects_dir_order() {
        let dir = temp_dir();
        let a = dir.join("a");
        let b = dir.join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("x.png"), b"a").unwrap();
        fs::write(b.join("x.png"), b"b").unwrap();
        let found = find_icon_file("x.png", &[a.clone(), b.clone()]).unwrap();
        assert_eq!(found, a.join("x.png"));
        let _ = fs::remove_dir_all(&dir);
    }
}
