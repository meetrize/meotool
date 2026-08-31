//! 配置 schema 版本与迁移。

use super::toml_model::ConfigFile;

/// 当前磁盘配置 schema 版本
pub const CURRENT_CONFIG_VERSION: u32 = 1;

/// 加载后、转 AppConfig 前执行迁移链。
pub fn migrate_config_file(file: &mut ConfigFile) -> Result<(), String> {
    if file.version > CURRENT_CONFIG_VERSION {
        return Err(format!(
            "配置文件版本 {} 高于本程序支持的 {}，请升级 MeoTool",
            file.version, CURRENT_CONFIG_VERSION
        ));
    }

    while file.version < CURRENT_CONFIG_VERSION {
        match file.version {
            0 => migrate_v0_to_v1(file)?,
            v => return Err(format!("缺少版本 {v} → {} 的迁移逻辑", v + 1)),
        }
    }
    Ok(())
}

/// v0：无 version 字段的旧配置，结构与 v1 兼容。
fn migrate_v0_to_v1(file: &mut ConfigFile) -> Result<(), String> {
    file.version = 1;
    if file.updated_at.is_none() {
        file.updated_at = Some(now_timestamp());
    }
    log::info!("migrated config schema v0 → v1");
    Ok(())
}

/// 保存时写入的时间戳（Unix 秒，UTC）。
pub fn now_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::toml_model::ConfigFile;

    #[test]
    fn migrates_v0_to_v1() {
        let mut file = ConfigFile {
            version: 0,
            updated_at: None,
            ..ConfigFile::default()
        };
        migrate_config_file(&mut file).unwrap();
        assert_eq!(file.version, CURRENT_CONFIG_VERSION);
        assert!(file.updated_at.is_some());
    }

    #[test]
    fn rejects_future_version() {
        let mut file = ConfigFile {
            version: CURRENT_CONFIG_VERSION + 1,
            ..ConfigFile::default()
        };
        assert!(migrate_config_file(&mut file).is_err());
    }
}
