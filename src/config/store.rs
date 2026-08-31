use std::fs;
use std::path::{Path, PathBuf};

use super::app_config::AppConfig;
use super::backup::try_restore_latest;
use super::model::PanelConfig;
use super::normalize::normalize_app_config;
use super::parse::parse_config_content;
use super::resolver::ProfileResolver;

/// 应用配置目录（macOS: ~/Library/Application Support/meotool）
pub fn app_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("meotool")
}

pub fn default_config_path() -> PathBuf {
    app_data_dir().join("config.toml")
}

/// 图标搜索目录（按优先级）
pub fn icon_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    dirs.push(app_data_dir().join("icons"));

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.join("assets").join("icons"));
        }
    }

    dirs.push(PathBuf::from("assets/icons"));
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/icons"));

    dirs
}

pub struct LoadedConfig {
    pub app_config: AppConfig,
    pub source_path: PathBuf,
    pub icon_dirs: Vec<PathBuf>,
}

impl LoadedConfig {
    pub fn load(path: Option<&Path>) -> Result<Self, String> {
        let config_path = resolve_config_path(path)?;
        match Self::load_from_path(&config_path) {
            Ok(loaded) => Ok(loaded),
            Err(err) => {
                log::warn!(
                    "failed to load {}: {err}; attempting backup restore",
                    config_path.display()
                );
                match try_restore_latest(&config_path) {
                    Ok(backup) => {
                        log::warn!(
                            "restored {} from backup {}",
                            config_path.display(),
                            backup.display()
                        );
                        Self::load_from_path(&config_path).map_err(|e| {
                            format!(
                                "config still invalid after restore from {}: {e} (original: {err})",
                                backup.display()
                            )
                        })
                    }
                    Err(restore_err) => Err(format!(
                        "{err}; backup restore failed: {restore_err}"
                    )),
                }
            }
        }
    }

    fn load_from_path(config_path: &Path) -> Result<Self, String> {
        let content = fs::read_to_string(config_path)
            .map_err(|e| format!("failed to read {}: {e}", config_path.display()))?;

        let mut app_config = parse_config_content(&content)?;
        normalize_app_config(&mut app_config);
        let icon_dirs = icon_search_dirs();

        log::info!("loaded config from {}", config_path.display());
        Ok(Self {
            app_config,
            source_path: config_path.to_path_buf(),
            icon_dirs,
        })
    }

    pub fn initial_panel(&self) -> PanelConfig {
        let idx = self.app_config.default_profile_index();
        self.app_config.build_panel(&self.app_config.profiles[idx])
    }

    pub fn resolver(&self) -> ProfileResolver<'_> {
        ProfileResolver::new(&self.app_config)
    }
}

fn bundled_config_path() -> Option<PathBuf> {
    bundled_config_candidates()
        .into_iter()
        .find(|path| path.exists())
}

fn bundled_config_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            paths.push(parent.join("configs/config.toml"));
            paths.push(parent.join("../Resources/configs/config.toml"));
        }
    }

    paths.push(PathBuf::from("configs/config.toml"));
    paths.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("configs/config.toml"));
    paths
}

fn resolve_config_path(cli_path: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = cli_path {
        return Ok(path.to_path_buf());
    }

    // 无 CLI 路径时始终使用用户目录；首次运行从 bundled 复制默认配置。
    ensure_user_config()
}

/// 将示例配置复制到用户目录（首次运行）
pub fn ensure_user_config() -> Result<PathBuf, String> {
    let dest = default_config_path();
    if dest.exists() {
        return Ok(dest);
    }

    let example = bundled_config_path().ok_or_else(|| {
        "bundled configs/config.toml not found (expected next to executable)".to_string()
    })?;

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create config dir: {e}"))?;
    }
    fs::copy(&example, &dest).map_err(|e| format!("copy default config: {e}"))?;
    log::info!("created default config at {}", dest.display());
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::export::save_app_config;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "meotool-store-test-{}-{}-{}",
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

    fn minimal_toml() -> &'static str {
        r#"
[[profiles]]
name = "default"

[[profiles.buttons]]
label = "A"
enabled = true

[profiles.buttons.action]
type = "hotkey"
keys = ["Cmd", "c"]
"#
    }

    #[test]
    fn load_recovers_from_backup_when_corrupt() {
        let dir = temp_dir();
        let path = dir.join("config.toml");
        fs::write(&path, minimal_toml()).unwrap();

        let loaded = LoadedConfig::load(Some(&path)).expect("initial load");
        save_app_config(&path, &loaded.app_config).expect("save creates backup");

        fs::write(&path, "{{{{ not valid toml").unwrap();

        let recovered = LoadedConfig::load(Some(&path)).expect("recover from backup");
        assert_eq!(recovered.app_config.profiles[0].name, "default");
        assert_eq!(recovered.app_config.profiles[0].buttons[0].label, "A");
        let mut corrupt = path.as_os_str().to_owned();
        corrupt.push(".corrupt");
        assert!(PathBuf::from(corrupt).exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_fails_clearly_without_backup() {
        let dir = temp_dir();
        let path = dir.join("config.toml");
        fs::write(&path, "not-toml").unwrap();
        let err = match LoadedConfig::load(Some(&path)) {
            Ok(_) => panic!("expected load to fail"),
            Err(e) => e,
        };
        assert!(
            err.contains("backup restore failed") || err.contains("no backups"),
            "unexpected err: {err}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
