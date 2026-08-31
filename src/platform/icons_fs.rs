//! 图标目录扫描（不依赖 egui）。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// 扫描图标目录，返回去重后的 PNG 文件名（按字母序）。
pub fn list_available_icons(search_dirs: &[PathBuf]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut names = Vec::new();

    for dir in search_dirs {
        let Ok(read_dir) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("png") {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if seen.insert(name.to_string()) {
                names.push(name.to_string());
            }
        }
    }

    names.sort_by(|a, b| a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()));
    names
}

pub fn resolve_icon_path(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    for dir in dirs {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
        if !name.ends_with(".png") {
            let p = dir.join(format!("{name}.png"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let fallback = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("icons")
        .join(if name.ends_with(".png") {
            name.to_string()
        } else {
            format!("{name}.png")
        });
    fallback.is_file().then_some(fallback)
}
