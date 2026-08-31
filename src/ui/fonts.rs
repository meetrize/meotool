use std::path::PathBuf;
use std::sync::OnceLock;

use eframe::egui::{Context, FontData, FontDefinitions, FontFamily};

use crate::ui::theme::CUSTOM_FONT_KEY;

const CJK_FONT_KEY: &str = "cjk";
const BUNDLED_CJK_FONT: &str = "ui-cjk.ttf";
/// 自定义字母字体超过此大小则拒绝加载，避免再次把整包大字体塞进堆
const MAX_CUSTOM_FONT_BYTES: u64 = 2 * 1024 * 1024;
/// 系统兜底字体超过此大小则拒绝（防止 Hiragino/Songti 等整包常驻）
const MAX_FALLBACK_FONT_BYTES: u64 = 6 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontLoadSource {
    Bundled,
    SystemFallback,
    None,
}

/// 按配置加载界面字体：优先内置子集 CJK，避免整包系统字体常驻。
/// 返回字体来源，便于启动日志与监控。
pub fn setup_theme_fonts(ctx: &Context, custom_font_path: Option<&str>) -> FontLoadSource {
    // 不使用 FontDefinitions::default()，以免开启 default_fonts 时再嵌入一份大字体
    let mut fonts = FontDefinitions::empty();
    let mut source = FontLoadSource::None;

    if let Some((static_bytes, from_bundled)) = cjk_font_static() {
        source = if from_bundled {
            FontLoadSource::Bundled
        } else {
            FontLoadSource::SystemFallback
        };
        // from_static：FontDefinitions 克隆时不复制整包字体；且只 leak 一次
        fonts
            .font_data
            .insert(CJK_FONT_KEY.to_owned(), FontData::from_static(static_bytes));
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, CJK_FONT_KEY.to_owned());
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, CJK_FONT_KEY.to_owned());
    } else {
        log::error!(
            "no usable CJK font found (bundled {BUNDLED_CJK_FONT} missing or too large); Chinese text may render as boxes"
        );
    }

    if let Some(path) = custom_font_path.filter(|p| !p.trim().is_empty()) {
        match load_custom_font_bytes(path) {
            Ok(bytes) => {
                log::info!("loaded letter font: {path} ({} KB)", bytes.len() / 1024);
                // 自定义字体可能随设置更换，用 owned 避免多次 leak
                fonts
                    .font_data
                    .insert(CUSTOM_FONT_KEY.to_owned(), FontData::from_owned(bytes));
                fonts
                    .families
                    .entry(FontFamily::Name(CUSTOM_FONT_KEY.into()))
                    .or_default()
                    .push(CUSTOM_FONT_KEY.to_owned());
            }
            Err(err) => log::warn!("letter font unavailable {path}: {err}"),
        }
    }

    if source == FontLoadSource::SystemFallback {
        log::warn!(
            "using system CJK fallback font; place assets/fonts/{BUNDLED_CJK_FONT} next to the binary to reduce memory"
        );
    }

    ctx.set_fonts(fonts);
    source
}

/// 重建字体定义以清空 egui 字形图集 / galley 缓存（长跑止血）。
pub fn reload_theme_fonts(ctx: &Context, custom_font_path: Option<&str>) -> FontLoadSource {
    log::info!("reloading UI fonts to reclaim glyph atlas memory");
    setup_theme_fonts(ctx, custom_font_path)
}

/// 进程内只加载并 leak 一次，供图集重置重复引用。
fn cjk_font_static() -> Option<(&'static [u8], bool)> {
    static CJK: OnceLock<Option<(&'static [u8], bool)>> = OnceLock::new();
    CJK.get_or_init(|| {
        load_cjk_font_bytes().map(|(bytes, bundled)| {
            let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
            (leaked, bundled)
        })
    })
    .as_ref()
    .map(|&(bytes, bundled)| (bytes, bundled))
}

fn load_custom_font_bytes(path: &str) -> Result<Vec<u8>, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_CUSTOM_FONT_BYTES {
        return Err(format!(
            "font too large ({} KB > {} KB limit); use a subset font",
            meta.len() / 1024,
            MAX_CUSTOM_FONT_BYTES / 1024
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.is_empty() {
        return Err("empty font file".into());
    }
    Ok(bytes)
}

/// 返回 (bytes, is_bundled)
fn load_cjk_font_bytes() -> Option<(Vec<u8>, bool)> {
    for (path, bundled) in candidate_font_paths() {
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if !bundled && meta.len() > MAX_FALLBACK_FONT_BYTES {
            log::warn!(
                "skip oversized system font {} ({} KB > {} KB)",
                path.display(),
                meta.len() / 1024,
                MAX_FALLBACK_FONT_BYTES / 1024
            );
            continue;
        }
        match std::fs::read(&path) {
            Ok(bytes) if !bytes.is_empty() => {
                log::info!(
                    "loaded UI font: {} ({} KB, {})",
                    path.display(),
                    bytes.len() / 1024,
                    if bundled {
                        "bundled"
                    } else {
                        "system-fallback"
                    }
                );
                return Some((bytes, bundled));
            }
            Ok(_) => log::warn!("empty font file: {}", path.display()),
            Err(err) => log::debug!("font unavailable {}: {err}", path.display()),
        }
    }
    None
}

fn candidate_font_paths() -> Vec<(PathBuf, bool)> {
    let mut paths = Vec::new();

    // 1) 随应用分发的子集字体（优先，约数 MB）
    for dir in font_search_dirs() {
        paths.push((dir.join(BUNDLED_CJK_FONT), true));
    }

    // 2) 系统字体兜底：限制大小，避免整包常驻
    #[cfg(target_os = "macos")]
    {
        paths.push((
            PathBuf::from("/System/Library/Fonts/Hiragino Sans GB.ttc"),
            false,
        ));
        paths.push((
            PathBuf::from("/System/Library/Fonts/STHeiti Light.ttc"),
            false,
        ));
        paths.push((
            PathBuf::from("/System/Library/Fonts/Supplemental/Songti.ttc"),
            false,
        ));
        paths.push((
            PathBuf::from("/System/Library/Fonts/Supplemental/Arial Unicode.ttf"),
            false,
        ));
    }
    #[cfg(not(target_os = "macos"))]
    {
        paths.push((
            PathBuf::from("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
            false,
        ));
        paths.push((
            PathBuf::from("/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc"),
            false,
        ));
        paths.push((
            PathBuf::from("/usr/share/fonts/truetype/wqy/wqy-microhei.ttc"),
            false,
        ));
    }

    paths
}

fn font_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.join("assets").join("fonts"));
            if let Some(contents) = parent.parent() {
                dirs.push(contents.join("Resources").join("fonts"));
            }
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join("assets").join("fonts"));
    }

    dirs.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("fonts"),
    );

    dirs
}
