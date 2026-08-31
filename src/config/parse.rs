//! 配置 TOML 解析（含 schema 迁移）。

use super::app_config::AppConfig;
use super::migrate::migrate_config_file;
use super::toml_model::ConfigFile;

/// 解析 TOML 文本 → 迁移 → AppConfig。
pub fn parse_config_content(content: &str) -> Result<AppConfig, String> {
    let mut file: ConfigFile =
        toml::from_str(content).map_err(|e| format_toml_error(content, &e))?;
    migrate_config_file(&mut file)?;
    file.into_app_config()
}

fn format_toml_error(content: &str, err: &toml::de::Error) -> String {
    let line = err.span().map(|span| {
        content[..span.start]
            .chars()
            .filter(|&c| c == '\n')
            .count()
            + 1
    });
    match line {
        Some(n) => format!("TOML 解析错误（约第 {n} 行）: {err}"),
        None => format!("TOML 解析错误: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_legacy_without_version() {
        let toml = r#"
[[profiles]]
name = "default"

[[profiles.buttons]]
label = "A"
enabled = true

[profiles.buttons.action]
type = "none"
"#;
        let app = parse_config_content(toml).unwrap();
        assert_eq!(app.profiles[0].name, "default");
    }
}
