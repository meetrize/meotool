//! 原生面板图标 LRU 缓存（对齐 egui `IconCache` 常驻上限）。

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use objc2::rc::Retained;
use objc2_app_kit::NSImage;
use objc2_foundation::{MainThreadMarker, NSData};

use crate::app::NativeMemoryCounters;
use crate::platform::{make_tintable_rgba, resolve_icon_path};

/// 面板常驻图标上限（与 egui 路径一致）
const MAX_RESIDENT_ICONS: usize = 32;
const MAX_ICON_EDGE: u32 = 48;
const TEMPLATE_PREFIX: &str = "tpl:";

pub struct NativeIconCache {
    images: HashMap<String, Retained<NSImage>>,
    lru: VecDeque<String>,
}

impl Default for NativeIconCache {
    fn default() -> Self {
        Self {
            images: HashMap::new(),
            lru: VecDeque::new(),
        }
    }
}

impl NativeIconCache {
    pub fn resident_count(&self) -> usize {
        self.images.len()
    }

    pub fn clear(&mut self) {
        let n = self.images.len();
        self.images.clear();
        self.lru.clear();
        if n > 0 {
            log::debug!("cleared {n} native icon cache entries");
        }
    }

    pub fn load(
        &mut self,
        mtm: MainThreadMarker,
        dirs: &[PathBuf],
        name: &str,
        counters: &mut NativeMemoryCounters,
    ) -> Option<Retained<NSImage>> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        if let Some(img) = self.images.get(name) {
            let handle = img.clone();
            self.touch_lru(name);
            return Some(handle);
        }

        let path = resolve_icon_path(dirs, name)?;
        let bytes = std::fs::read(&path).ok()?;
        let data = NSData::with_bytes(&bytes);
        let img = unsafe { NSImage::initWithData(mtm.alloc::<NSImage>(), &data) }?;
        counters.record_icon_load();
        log::debug!("loaded icon {name} from {}", path.display());
        self.insert(name.to_string(), img.clone());
        Some(img)
    }

    /// 加载可模板着色的图标（白色蒙版），用于面板按钮 `contentTintColor`。
    pub fn load_template(
        &mut self,
        mtm: MainThreadMarker,
        dirs: &[PathBuf],
        name: &str,
        counters: &mut NativeMemoryCounters,
    ) -> Option<Retained<NSImage>> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let key = format!("{TEMPLATE_PREFIX}{name}");
        if let Some(img) = self.images.get(&key) {
            let handle = img.clone();
            self.touch_lru(&key);
            return Some(handle);
        }

        let path = resolve_icon_path(dirs, name)?;
        let bytes = std::fs::read(&path).ok()?;
        let img = decode_template_image(mtm, &bytes)?;
        counters.record_icon_load();
        log::debug!("loaded template icon {name} from {}", path.display());
        self.insert(key, img.clone());
        Some(img)
    }

    fn insert(&mut self, key: String, image: Retained<NSImage>) {
        if self.images.contains_key(&key) {
            self.touch_lru(&key);
            return;
        }
        self.images.insert(key.clone(), image);
        self.lru.push_back(key);
        while self.images.len() > MAX_RESIDENT_ICONS {
            if let Some(old) = self.lru.pop_front() {
                self.images.remove(&old);
            } else {
                break;
            }
        }
    }

    fn touch_lru(&mut self, key: &str) {
        if let Some(pos) = self.lru.iter().position(|k| k == key) {
            self.lru.remove(pos);
            self.lru.push_back(key.to_string());
        }
    }
}

/// 由 PNG 等原始字节解码为模板图标（供编辑器预览等一次性场景）。
pub(crate) fn decode_template_image(mtm: MainThreadMarker, bytes: &[u8]) -> Option<Retained<NSImage>> {
    use std::io::Cursor;

    let img = image::load_from_memory(bytes).ok()?;
    let img = if img.width() > MAX_ICON_EDGE || img.height() > MAX_ICON_EDGE {
        img.thumbnail(MAX_ICON_EDGE, MAX_ICON_EDGE)
    } else {
        img
    };
    let mut rgba = img.to_rgba8();
    make_tintable_rgba(rgba.as_mut());
    let mut buf = Vec::new();
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
        .ok()?;
    let data = NSData::with_bytes(&buf);
    let ns_img = unsafe { NSImage::initWithData(mtm.alloc::<NSImage>(), &data)? };
    unsafe { ns_img.setTemplate(true) };
    Some(ns_img)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_key_prefix() {
        assert!(TEMPLATE_PREFIX.starts_with("tpl:"));
    }
}
