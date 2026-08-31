use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use egui::{Color32, ColorImage, Context, TextureHandle, TextureOptions};

use crate::config::color::parse_color;
use crate::platform::make_tintable_rgba;

/// 面板常驻图标上限（LRU）
const MAX_RESIDENT_TEXTURES: usize = 32;
/// 图标选择器临时纹理上限（LRU）；滚动时淘汰，关闭时清空
const MAX_PICKER_TEXTURES: usize = 48;
/// 解码后最长边，避免大图占满 GPU/CPU 位图
const MAX_ICON_EDGE: u32 = 48;

/// 图标纹理缓存：按需加载 + LRU，选择器浏览走独立临时缓存
pub struct IconCache {
    search_dirs: Vec<PathBuf>,
    textures: HashMap<String, TextureHandle>,
    lru: VecDeque<String>,
    picker_textures: HashMap<String, TextureHandle>,
    picker_lru: VecDeque<String>,
}

impl IconCache {
    pub fn new(search_dirs: Vec<PathBuf>) -> Self {
        Self {
            search_dirs,
            textures: HashMap::new(),
            lru: VecDeque::new(),
            picker_textures: HashMap::new(),
            picker_lru: VecDeque::new(),
        }
    }

    pub fn resident_count(&self) -> usize {
        self.textures.len()
    }

    pub fn picker_count(&self) -> usize {
        self.picker_textures.len()
    }

    pub fn get(&mut self, ctx: &Context, icon_name: &str) -> Option<TextureHandle> {
        self.get_cached(ctx, icon_name, false)
    }

    /// 加载彩色原图（不做白色蒙版转换），用于折叠桌宠等需要保留配色的场景
    pub fn get_colored(&mut self, ctx: &Context, icon_name: &str) -> Option<TextureHandle> {
        let key = format!("colored:{icon_name}");
        self.get_cached(ctx, &key, true)
    }

    /// 图标选择器专用：写入临时缓存（有 LRU），关闭窗口时可整体释放
    pub fn get_for_picker(&mut self, ctx: &Context, icon_name: &str) -> Option<TextureHandle> {
        if let Some(tex) = self.picker_textures.get(icon_name) {
            let handle = tex.clone();
            self.touch_picker_lru(icon_name);
            return Some(handle);
        }
        // 已在常驻缓存中则复用，避免重复解码
        if self.textures.contains_key(icon_name) {
            self.touch_lru(icon_name);
            return self.textures.get(icon_name).cloned();
        }

        let path = self.resolve_path(icon_name)?;
        let image = load_image(&path)?;
        let image = make_tintable_icon(image);
        let texture = ctx.load_texture(
            format!("icon-picker:{icon_name}"),
            image,
            TextureOptions::LINEAR,
        );
        self.insert_picker(icon_name.to_string(), texture.clone());
        Some(texture)
    }

    pub fn clear_picker_cache(&mut self) {
        let n = self.picker_textures.len();
        self.picker_textures.clear();
        self.picker_lru.clear();
        if n > 0 {
            log::debug!("cleared {n} icon picker textures");
        }
    }

    /// 清空全部图标纹理（字体重置等同生命周期清理时可调用）
    pub fn clear_all(&mut self) {
        self.textures.clear();
        self.lru.clear();
        self.clear_picker_cache();
    }

    fn get_cached(&mut self, ctx: &Context, key: &str, colored: bool) -> Option<TextureHandle> {
        if let Some(tex) = self.textures.get(key) {
            let handle = tex.clone();
            self.touch_lru(key);
            return Some(handle);
        }

        let icon_name = key.strip_prefix("colored:").unwrap_or(key);
        let path = self.resolve_path(icon_name)?;
        let image = load_image(&path)?;
        let image = if colored {
            image
        } else {
            make_tintable_icon(image)
        };
        let texture = ctx.load_texture(
            if colored {
                format!("icon-colored:{icon_name}")
            } else {
                format!("icon:{icon_name}")
            },
            image,
            TextureOptions::LINEAR,
        );
        log::debug!("loaded icon {key} from {}", path.display());
        self.insert_resident(key.to_string(), texture.clone());
        Some(texture)
    }

    fn insert_resident(&mut self, key: String, texture: TextureHandle) {
        if self.textures.contains_key(&key) {
            self.touch_lru(&key);
            return;
        }
        self.textures.insert(key.clone(), texture);
        self.lru.push_back(key);
        while self.textures.len() > MAX_RESIDENT_TEXTURES {
            if let Some(old) = self.lru.pop_front() {
                self.textures.remove(&old);
            } else {
                break;
            }
        }
    }

    fn insert_picker(&mut self, key: String, texture: TextureHandle) {
        if self.picker_textures.contains_key(&key) {
            self.touch_picker_lru(&key);
            return;
        }
        self.picker_textures.insert(key.clone(), texture);
        self.picker_lru.push_back(key);
        while self.picker_textures.len() > MAX_PICKER_TEXTURES {
            if let Some(old) = self.picker_lru.pop_front() {
                self.picker_textures.remove(&old);
            } else {
                break;
            }
        }
    }

    fn touch_lru(&mut self, key: &str) {
        if let Some(pos) = self.lru.iter().position(|k| k == key) {
            if let Some(k) = self.lru.remove(pos) {
                self.lru.push_back(k);
            }
        }
    }

    fn touch_picker_lru(&mut self, key: &str) {
        if let Some(pos) = self.picker_lru.iter().position(|k| k == key) {
            if let Some(k) = self.picker_lru.remove(pos) {
                self.picker_lru.push_back(k);
            }
        }
    }

    fn resolve_path(&self, icon_name: &str) -> Option<PathBuf> {
        for dir in &self.search_dirs {
            let path = dir.join(icon_name);
            if path.exists() {
                return Some(path);
            }
        }
        log::warn!("icon not found: {icon_name}");
        None
    }

    pub fn search_dirs(&self) -> &[PathBuf] {
        &self.search_dirs
    }
}

use crate::platform::icons_fs::list_available_icons as list_icons_fs;

/// 扫描图标目录，返回去重后的 PNG 文件名（按字母序）
pub fn list_available_icons(search_dirs: &[PathBuf]) -> Vec<String> {
    list_icons_fs(search_dirs)
}

/// 解析按钮图标着色；未配置时使用 `fallback`（通常为全局文字色）
pub fn resolve_icon_tint(icon_color: Option<&str>, fallback: Color32, active: bool) -> Color32 {
    let base = icon_color
        .and_then(parse_color)
        .map(crate::ui::egui_color::to_egui)
        .unwrap_or(fallback);
    if active {
        base
    } else {
        base.gamma_multiply(0.5)
    }
}

fn load_image(path: &Path) -> Option<ColorImage> {
    let img = image::open(path).ok()?;
    let img = if img.width() > MAX_ICON_EDGE || img.height() > MAX_ICON_EDGE {
        img.thumbnail(MAX_ICON_EDGE, MAX_ICON_EDGE)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, &rgba))
}

/// 将黑色/深色剪影 PNG 转为白色蒙版，便于 egui tint 正确着色
fn make_tintable_icon(mut image: ColorImage) -> ColorImage {
    let mut rgba = Vec::with_capacity(image.pixels.len() * 4);
    for px in &image.pixels {
        rgba.extend_from_slice(&px.to_array());
    }
    make_tintable_rgba(&mut rgba);
    for (px, chunk) in image.pixels.iter_mut().zip(rgba.chunks_exact(4)) {
        *px = Color32::from_rgba_unmultiplied(chunk[0], chunk[1], chunk[2], chunk[3]);
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_icon_becomes_white_mask() {
        let mut image = ColorImage::new([1, 1], Color32::TRANSPARENT);
        image.pixels[0] = Color32::from_rgba_unmultiplied(0, 0, 0, 255);
        let tinted = make_tintable_icon(image);
        assert_eq!(tinted.pixels[0], Color32::from_white_alpha(255));
    }

    #[test]
    fn transparent_pixel_stays_transparent() {
        let image = ColorImage::new([1, 1], Color32::TRANSPARENT);
        let tinted = make_tintable_icon(image);
        assert_eq!(tinted.pixels[0], Color32::TRANSPARENT);
    }
}
