use std::fs;
use std::path::Path;

use toml_edit::{DocumentMut, Item};

use super::backup::atomic_write;

/// 将面板位置写回配置文件（仅更新 global.panel.position）。
/// 拖拽会较频繁调用，故不做自动备份，仅原子写入。
pub fn save_panel_position(path: &Path, x: f32, y: f32) -> Result<(), String> {
    let content = fs::read_to_string(path).map_err(|e| format!("read config for save: {e}"))?;
    let mut doc = content
        .parse::<DocumentMut>()
        .map_err(|e| format!("parse config for save: {e}"))?;

    let global = doc
        .entry("global")
        .or_insert(Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or("global is not a table")?;

    let panel = global
        .entry("panel")
        .or_insert(Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or("global.panel is not a table")?;

    let mut pos = toml_edit::Table::new();
    pos.insert("x", toml_edit::value(x as f64));
    pos.insert("y", toml_edit::value(y as f64));
    panel.insert("position", Item::Table(pos));

    atomic_write(path, doc.to_string())?;
    log::info!("saved panel position ({x}, {y}) to {}", path.display());
    Ok(())
}
