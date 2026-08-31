//! 图标模板化与着色（native / egui 共用像素逻辑）。

use crate::config::color::{parse_color, RgbColor};

/// 将 RGBA 像素转为白色蒙版，便于模板着色。
pub fn make_tintable_rgba(rgba: &mut [u8]) {
    for px in rgba.chunks_mut(4) {
        let [r, g, b, a] = [px[0], px[1], px[2], px[3]];

        if a == 0 && r == 0 && g == 0 && b == 0 {
            px[3] = 0;
            continue;
        }

        let mask = if r == 0 && g == 0 && b == 0 {
            a
        } else {
            let lum = r.max(g).max(b);
            ((lum as u16 * a as u16 + 127) / 255) as u8
        };

        if mask == 0 {
            px[0] = 0;
            px[1] = 0;
            px[2] = 0;
            px[3] = 0;
        } else {
            px[0] = 255;
            px[1] = 255;
            px[2] = 255;
            px[3] = mask;
        }
    }
}

/// 解析按钮图标着色；未配置时使用 `fallback`（通常为全局文字色）。
pub fn resolve_native_icon_tint(
    icon_color: Option<&str>,
    fallback: RgbColor,
    enabled: bool,
) -> RgbColor {
    let base = icon_color.and_then(parse_color).unwrap_or(fallback);
    if enabled {
        base
    } else {
        RgbColor {
            r: ((base.r as f32) * 0.5).round().clamp(0.0, 255.0) as u8,
            g: ((base.g as f32) * 0.5).round().clamp(0.0, 255.0) as u8,
            b: ((base.b as f32) * 0.5).round().clamp(0.0, 255.0) as u8,
            a: base.a,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_pixel_becomes_white_mask() {
        let mut px = [0u8, 0, 0, 255];
        make_tintable_rgba(&mut px);
        assert_eq!(px, [255, 255, 255, 255]);
    }

    #[test]
    fn transparent_pixel_stays_transparent() {
        let mut px = [0u8, 0, 0, 0];
        make_tintable_rgba(&mut px);
        assert_eq!(px[3], 0);
    }

    #[test]
    fn disabled_halves_rgb() {
        let tint = resolve_native_icon_tint(Some("#FF8040"), RgbColor::rgb(200, 200, 200), false);
        assert_eq!(tint.r, 128);
        assert_eq!(tint.g, 64);
        assert_eq!(tint.b, 32);
    }
}
