//! 配置备份、原子写入与损坏恢复。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// 自动备份保留份数
pub const MAX_BACKUPS: usize = 20;

#[derive(Debug, Clone)]
#[allow(dead_code)] // size 供后续备份管理 UI 使用
pub struct BackupEntry {
    pub path: PathBuf,
    pub modified: SystemTime,
    pub size: u64,
}

/// 配置文件旁的 backups 目录：`…/meotool/backups/`
pub fn backups_dir(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("backups")
}

/// 最近一次写入前的单文件备份：`config.toml.bak`
pub fn sidecar_bak_path(config_path: &Path) -> PathBuf {
    let mut bak = config_path.as_os_str().to_owned();
    bak.push(".bak");
    PathBuf::from(bak)
}

/// 损坏文件暂存：`config.toml.corrupt`
fn corrupt_path(config_path: &Path) -> PathBuf {
    let mut p = config_path.as_os_str().to_owned();
    p.push(".corrupt");
    PathBuf::from(p)
}

/// 原子写入：先写同目录临时文件，再 rename 覆盖目标。
pub fn atomic_write(path: &Path, content: impl AsRef<[u8]>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| format!("create config dir: {e}"))?;
        }
    }

    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("config.toml");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_name = format!(".{file_name}.{nanos}.tmp");
    let tmp_path = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(tmp_name);

    fs::write(&tmp_path, content.as_ref())
        .map_err(|e| format!("write temp config {}: {e}", tmp_path.display()))?;

    fs::rename(&tmp_path, path).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        format!("replace config {}: {e}", path.display())
    })?;

    Ok(())
}

fn backup_filename() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    // 毫秒时间戳，可按字典序排序；同毫秒追加 nanos 后缀避免碰撞
    format!("{:013}_{:06}.toml", dur.as_millis(), dur.subsec_nanos() % 1_000_000)
}

/// 写入前备份当前配置（若不存在则跳过）。
/// 失败只记日志由调用方决定是否继续；本函数返回 Err 供调用方 warn。
pub fn backup_before_write(config_path: &Path) -> Result<(), String> {
    if !config_path.exists() {
        return Ok(());
    }

    let dir = backups_dir(config_path);
    fs::create_dir_all(&dir).map_err(|e| format!("create backups dir: {e}"))?;

    let dest = dir.join(backup_filename());
    fs::copy(config_path, &dest)
        .map_err(|e| format!("backup to {}: {e}", dest.display()))?;

    let bak = sidecar_bak_path(config_path);
    if let Err(e) = fs::copy(config_path, &bak) {
        log::warn!("sidecar bak failed ({}): {e}", bak.display());
    }

    prune_backups(&dir, MAX_BACKUPS)?;
    log::info!("backed up config to {}", dest.display());
    Ok(())
}

fn prune_backups(dir: &Path, keep: usize) -> Result<(), String> {
    let mut entries = list_backup_files(dir)?;
    if entries.len() <= keep {
        return Ok(());
    }
    // 最新在前；删除多余的旧文件
    entries.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| b.path.cmp(&a.path)));
    for entry in entries.into_iter().skip(keep) {
        if let Err(e) = fs::remove_file(&entry.path) {
            log::warn!("prune backup {}: {e}", entry.path.display());
        }
    }
    Ok(())
}

fn list_backup_files(dir: &Path) -> Result<Vec<BackupEntry>, String> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let rd = fs::read_dir(dir).map_err(|e| format!("read backups dir: {e}"))?;
    for entry in rd {
        let entry = entry.map_err(|e| format!("read backup entry: {e}"))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !name.ends_with(".toml") {
            continue;
        }
        let meta = entry
            .metadata()
            .map_err(|e| format!("metadata {}: {e}", path.display()))?;
        out.push(BackupEntry {
            path,
            modified: meta.modified().unwrap_or(UNIX_EPOCH),
            size: meta.len(),
        });
    }
    Ok(out)
}

/// 列出 backups/ 下的备份（最新在前）
pub fn list_backups(config_path: &Path) -> Result<Vec<BackupEntry>, String> {
    let mut entries = list_backup_files(&backups_dir(config_path))?;
    entries.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| b.path.cmp(&a.path)));
    Ok(entries)
}

/// 最新一份 backups/ 备份路径
pub fn latest_backup(config_path: &Path) -> Option<PathBuf> {
    list_backups(config_path)
        .ok()?
        .into_iter()
        .next()
        .map(|e| e.path)
}

/// 将备份恢复到配置路径（原子替换）；恢复前会先备份当前文件。
pub fn restore_from_backup(backup: &Path, config_path: &Path) -> Result<(), String> {
    if !backup.exists() {
        return Err(format!("backup not found: {}", backup.display()));
    }
    if config_path.exists() {
        let _ = backup_before_write(config_path);
    }
    let content =
        fs::read(backup).map_err(|e| format!("read backup {}: {e}", backup.display()))?;

    if config_path.exists() {
        let corrupt = corrupt_path(config_path);
        if let Err(e) = fs::rename(config_path, &corrupt) {
            log::warn!(
                "could not quarantine corrupt config to {}: {e}",
                corrupt.display()
            );
        } else {
            log::warn!("quarantined corrupt config to {}", corrupt.display());
        }
    }

    atomic_write(config_path, content)?;
    log::info!(
        "restored config from {} -> {}",
        backup.display(),
        config_path.display()
    );
    Ok(())
}

/// 尝试用最新备份恢复；成功返回所用备份路径
pub fn try_restore_latest(config_path: &Path) -> Result<PathBuf, String> {
    let backup = latest_backup(config_path).ok_or_else(|| {
        format!(
            "no backups available under {}",
            backups_dir(config_path).display()
        )
    })?;
    restore_from_backup(&backup, config_path)?;
    Ok(backup)
}

/// 格式化备份条目供 UI 显示。
pub fn format_backup_entry(entry: &BackupEntry) -> String {
    let size_kb = (entry.size + 512) / 1024;
    if let Some(stem) = entry.path.file_stem().and_then(|s| s.to_str()) {
        if let Some(ms_part) = stem.split('_').next() {
            if let Ok(ms) = ms_part.parse::<u64>() {
                return format!("{} · {} KB", format_unix_ms(ms), size_kb);
            }
        }
    }
    let name = entry
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "?".into());
    format!("{name} · {size_kb} KB")
}

fn format_unix_ms(ms: u64) -> String {
    let secs = ms / 1000;
    let days = secs / 86400;
    let time = secs % 86400;
    let h = time / 3600;
    let m = (time % 3600) / 60;
    let s = time % 60;
    let (y, mo, d) = civil_from_days(days as i64);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02}")
}

/// 由 Unix 日数计算公历日期（UTC，Howard Hinnant 算法）
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100 + yoe / 400);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = mp + if mp < 10 { 3 } else { -9 };
    let y = y + if mo <= 2 { 1 } else { 0 };
    (y, mo, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "meotool-backup-test-{}-{}-{}",
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

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn format_backup_entry_parses_filename() {
        let dir = temp_dir();
        let path = dir.join("1756234567890_123456.toml");
        fs::write(&path, b"x").unwrap();
        let meta = fs::metadata(&path).unwrap();
        let entry = BackupEntry {
            path: path.clone(),
            modified: meta.modified().unwrap_or(UNIX_EPOCH),
            size: meta.len(),
        };
        let label = format_backup_entry(&entry);
        assert!(label.contains("KB"), "{label}");
        assert!(label.contains("2025") || label.contains("2026"), "{label}");
        cleanup(&dir);
    }

    #[test]
    fn atomic_write_creates_and_replaces() {
        let dir = temp_dir();
        let path = dir.join("config.toml");
        atomic_write(&path, b"hello").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
        atomic_write(&path, b"world").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "world");
        // 不应残留 .tmp
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .contains(".tmp")
            })
            .collect();
        assert!(leftovers.is_empty(), "tmp leftovers: {leftovers:?}");
        cleanup(&dir);
    }

    #[test]
    fn backup_before_write_and_rotate() {
        let dir = temp_dir();
        let path = dir.join("config.toml");
        fs::write(&path, "v0").unwrap();

        for i in 1..=25 {
            fs::write(&path, format!("v{i}")).unwrap();
            backup_before_write(&path).unwrap();
            // 文件名按毫秒，连续写入可能同毫秒碰撞；稍等保证可区分
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

        let backups = list_backups(&path).unwrap();
        assert!(
            backups.len() <= MAX_BACKUPS,
            "expected <= {MAX_BACKUPS}, got {}",
            backups.len()
        );
        assert!(sidecar_bak_path(&path).exists());
        cleanup(&dir);
    }

    #[test]
    fn restore_from_latest_backup() {
        let dir = temp_dir();
        let path = dir.join("config.toml");
        fs::write(&path, "good").unwrap();
        backup_before_write(&path).unwrap();

        fs::write(&path, "not toml {{{").unwrap();
        let used = try_restore_latest(&path).unwrap();
        assert!(used.exists());
        assert_eq!(fs::read_to_string(&path).unwrap(), "good");
        assert!(corrupt_path(&path).exists());
        cleanup(&dir);
    }

    #[test]
    fn backup_skips_missing_file() {
        let dir = temp_dir();
        let path = dir.join("missing.toml");
        backup_before_write(&path).unwrap();
        assert!(!backups_dir(&path).exists() || list_backups(&path).unwrap().is_empty());
        cleanup(&dir);
    }
}
