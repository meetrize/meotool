//! UI 无关的 RGBA 颜色（配置层）；egui 转换见 `ui::egui_color`。

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl RgbColor {
    pub const TRANSPARENT: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    #[cfg(any(feature = "ui-egui", test))]
    pub fn to_srgba_unmultiplied(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

/// 解析颜色字符串，支持 `transparent`、`#RGB`、`#RGBA`、`#RRGGBB`、`#RRGGBBAA`、`rgb()`、`rgba()`
pub fn parse_color(s: &str) -> Option<RgbColor> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    if lower == "transparent" || lower == "none" {
        return Some(RgbColor::TRANSPARENT);
    }
    if s.starts_with('#') {
        return parse_hex_color(s);
    }
    if lower.starts_with("rgba(") && s.ends_with(')') {
        return parse_rgba_fn(&s[5..s.len() - 1]);
    }
    if lower.starts_with("rgb(") && s.ends_with(')') {
        return parse_rgb_fn(&s[4..s.len() - 1]);
    }
    None
}

pub fn color_to_string(c: RgbColor) -> String {
    if c.a == 0 {
        "transparent".into()
    } else if c.a == 255 {
        format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
    } else {
        format!("rgba({}, {}, {}, {:.3})", c.r, c.g, c.b, c.a as f32 / 255.0)
    }
}

fn parse_hex_color(s: &str) -> Option<RgbColor> {
    let hex = s[1..].trim();
    match hex.len() {
        3 => {
            let r = hex_digit(hex.chars().nth(0)?)? * 17;
            let g = hex_digit(hex.chars().nth(1)?)? * 17;
            let b = hex_digit(hex.chars().nth(2)?)? * 17;
            Some(RgbColor::rgb(r, g, b))
        }
        4 => {
            let r = hex_digit(hex.chars().nth(0)?)? * 17;
            let g = hex_digit(hex.chars().nth(1)?)? * 17;
            let b = hex_digit(hex.chars().nth(2)?)? * 17;
            let a = hex_digit(hex.chars().nth(3)?)? * 17;
            Some(RgbColor::rgba(r, g, b, a))
        }
        6 => {
            let r = parse_hex_byte(hex, 0)?;
            let g = parse_hex_byte(hex, 2)?;
            let b = parse_hex_byte(hex, 4)?;
            Some(RgbColor::rgb(r, g, b))
        }
        8 => {
            let r = parse_hex_byte(hex, 0)?;
            let g = parse_hex_byte(hex, 2)?;
            let b = parse_hex_byte(hex, 4)?;
            let a = parse_hex_byte(hex, 6)?;
            Some(RgbColor::rgba(r, g, b, a))
        }
        _ => None,
    }
}

fn parse_rgb_fn(inner: &str) -> Option<RgbColor> {
    let parts: Vec<f32> = inner
        .split(',')
        .map(|p| p.trim().parse())
        .collect::<Result<_, _>>()
        .ok()?;
    if parts.len() != 3 {
        return None;
    }
    Some(RgbColor::rgb(
        clamp_u8(parts[0]),
        clamp_u8(parts[1]),
        clamp_u8(parts[2]),
    ))
}

fn parse_rgba_fn(inner: &str) -> Option<RgbColor> {
    let parts: Vec<f32> = inner
        .split(',')
        .map(|p| p.trim().parse())
        .collect::<Result<_, _>>()
        .ok()?;
    if parts.len() != 4 {
        return None;
    }
    let alpha = if parts[3] <= 1.0 {
        parts[3] * 255.0
    } else {
        parts[3]
    };
    Some(RgbColor::rgba(
        clamp_u8(parts[0]),
        clamp_u8(parts[1]),
        clamp_u8(parts[2]),
        clamp_u8(alpha),
    ))
}

fn hex_digit(c: char) -> Option<u8> {
    c.to_digit(16).map(|d| d as u8)
}

fn parse_hex_byte(hex: &str, start: usize) -> Option<u8> {
    let slice = hex.get(start..start + 2)?;
    u8::from_str_radix(slice, 16).ok()
}

fn clamp_u8(v: f32) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_transparent() {
        assert_eq!(parse_color("transparent"), Some(RgbColor::TRANSPARENT));
    }

    #[test]
    fn parse_rgba() {
        let c = parse_color("rgba(40, 40, 40, 0.85)").unwrap();
        assert!((c.r as i32 - 40).abs() <= 1);
        assert!((c.g as i32 - 40).abs() <= 1);
        assert!((c.b as i32 - 40).abs() <= 1);
        assert_eq!(c.a, 217);
    }

    #[test]
    fn parse_hex() {
        let c = parse_color("#E0E0E0").unwrap();
        assert_eq!(c, RgbColor::rgb(224, 224, 224));
    }
}
