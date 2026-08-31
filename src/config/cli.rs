//! 命令行导入 / 导出（无 UI）。

use std::env;
use std::path::{Path, PathBuf};

use super::app_config::AppConfig;
use super::import_export::{
    export_bundle, export_config_toml, import_config, ImportMode, ProfileConflictStrategy,
};
use super::store::{app_data_dir, LoadedConfig};

#[derive(Debug, Clone)]
pub struct CliArgs {
    pub config_path: Option<PathBuf>,
    pub command: Option<CliCommand>,
    pub show_help: bool,
}

#[derive(Debug, Clone)]
pub enum CliCommand {
    Export {
        dest: PathBuf,
        bundle: bool,
    },
    Import {
        source: PathBuf,
        mode: ImportMode,
        conflict: ProfileConflictStrategy,
    },
    MigrateActions,
}

const HELP: &str = "\
MeoTool — 窗口感知浮动快捷面板

用法:
  meotool [选项]
  meotool --export <路径> [--bundle]
  meotool --import <路径> [--replace | --merge] [--overwrite | --rename | --skip]
  meotool --migrate-actions   将配置中旧动作类型批量升级为动作脚本

选项:
  --config <路径>    指定配置文件（默认：Application Support/meotool/config.toml）
  --export <路径>    导出为 TOML 文件
  --export-bundle <目录>  导出完整包（config.toml + icons/ + manifest.json）
  --import <路径>    导入 TOML 或完整包目录
  --replace          导入时替换全部（默认）
  --merge            导入时合并配置档
  --overwrite        合并时覆盖同名配置档
  --rename           合并时重命名导入项（默认）
  --skip             合并时跳过同名配置档
  --migrate-actions  将旧动作类型批量升级为动作脚本并写回配置
  --help, -h         显示帮助
";

pub fn parse_cli_args() -> CliArgs {
    let mut config_path = None;
    let mut show_help = false;
    let mut import_mode = ImportMode::Replace;
    let mut conflict = ProfileConflictStrategy::RenameImported;
    let mut export_dest: Option<(PathBuf, bool)> = None;
    let mut import_source: Option<PathBuf> = None;
    let mut migrate_actions = false;

    let args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--help" | "-h" => show_help = true,
            "--config" => {
                i += 1;
                config_path = args.get(i).map(PathBuf::from);
            }
            "--replace" => import_mode = ImportMode::Replace,
            "--merge" => import_mode = ImportMode::Merge,
            "--overwrite" => conflict = ProfileConflictStrategy::Overwrite,
            "--rename" => conflict = ProfileConflictStrategy::RenameImported,
            "--skip" => conflict = ProfileConflictStrategy::Skip,
            "--export" => {
                i += 1;
                if let Some(dest) = args.get(i) {
                    export_dest = Some((PathBuf::from(dest), false));
                }
            }
            "--export-bundle" => {
                i += 1;
                if let Some(dest) = args.get(i) {
                    export_dest = Some((PathBuf::from(dest), true));
                }
            }
            "--import" => {
                i += 1;
                if let Some(source) = args.get(i) {
                    import_source = Some(PathBuf::from(source));
                }
            }
            "--migrate-actions" => {
                migrate_actions = true;
            }
            other if let Some(path) = other.strip_prefix("--config=") => {
                config_path = Some(PathBuf::from(path));
            }
            _ => {}
        }
        i += 1;
    }

    let command = if migrate_actions {
        Some(CliCommand::MigrateActions)
    } else if let Some((dest, bundle)) = export_dest {
        Some(CliCommand::Export { dest, bundle })
    } else if let Some(source) = import_source {
        Some(CliCommand::Import {
            source,
            mode: import_mode,
            conflict,
        })
    } else {
        None
    };

    CliArgs {
        config_path,
        command,
        show_help,
    }
}

pub fn run_cli(args: &CliArgs) -> Result<(), String> {
    if args.show_help {
        print!("{HELP}");
        return Ok(());
    }
    let Some(cmd) = args.command.clone() else {
        return Ok(());
    };

    let loaded = LoadedConfig::load(args.config_path.as_deref())?;
    let config_path = loaded.source_path.clone();
    let icon_dirs = loaded.icon_dirs.clone();
    let app = loaded.app_config;

    match cmd {
        CliCommand::Export { dest, bundle: true } => {
            export_bundle(&app, &icon_dirs, &dest)?;
            eprintln!("exported bundle to {}", dest.display());
        }
        CliCommand::Export { dest, bundle: false } => {
            export_config_toml(&app, &dest)?;
            eprintln!("exported config to {}", dest.display());
        }
        CliCommand::Import {
            source,
            mode,
            conflict,
        } => {
            let user_icons = app_data_dir().join("icons");
            let result = import_config(
                &source,
                &config_path,
                &user_icons,
                &app,
                mode,
                conflict,
            )?;
            eprintln!(
                "imported from {} (profiles={})",
                source.display(),
                result.app_config.profiles.len()
            );
        }
        CliCommand::MigrateActions => {
            use crate::action::legacy_convert::migrate_actions_in_config;
            use super::export::save_app_config;

            let mut app = app;
            let count = migrate_actions_in_config(&mut app);
            save_app_config(&config_path, &app)?;
            eprintln!("migrated {count} button action(s) to script in {}", config_path.display());
        }
    }
    Ok(())
}

#[allow(dead_code)]
pub fn load_for_cli(config_path: Option<&Path>) -> Result<(PathBuf, AppConfig, Vec<PathBuf>), String> {
    let loaded = LoadedConfig::load(config_path)?;
    Ok((loaded.source_path, loaded.app_config, loaded.icon_dirs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_export_and_import_flags() {
        let args = parse_cli_from(&["--export", "/tmp/out.toml"]);
        assert!(matches!(
            args.command,
            Some(CliCommand::Export { bundle: false, .. })
        ));

        let args = parse_cli_from(&[
            "--import",
            "/tmp/in",
            "--merge",
            "--overwrite",
        ]);
        match args.command {
            Some(CliCommand::Import { mode, conflict, .. }) => {
                assert_eq!(mode, ImportMode::Merge);
                assert_eq!(conflict, ProfileConflictStrategy::Overwrite);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    /// 与 `parse_cli_args` 相同的两阶段解析，供单元测试使用。
    fn parse_cli_from(argv: &[&str]) -> CliArgs {
        let mut import_mode = ImportMode::Replace;
        let mut conflict = ProfileConflictStrategy::RenameImported;
        let mut export_dest: Option<(PathBuf, bool)> = None;
        let mut import_source: Option<PathBuf> = None;

        let mut i = 0;
        while i < argv.len() {
            match argv[i] {
                "--replace" => import_mode = ImportMode::Replace,
                "--merge" => import_mode = ImportMode::Merge,
                "--overwrite" => conflict = ProfileConflictStrategy::Overwrite,
                "--rename" => conflict = ProfileConflictStrategy::RenameImported,
                "--skip" => conflict = ProfileConflictStrategy::Skip,
                "--export" => {
                    i += 1;
                    export_dest = Some((PathBuf::from(argv[i]), false));
                }
                "--export-bundle" => {
                    i += 1;
                    export_dest = Some((PathBuf::from(argv[i]), true));
                }
                "--import" => {
                    i += 1;
                    import_source = Some(PathBuf::from(argv[i]));
                }
                _ => {}
            }
            i += 1;
        }

        let command = if let Some((dest, bundle)) = export_dest {
            Some(CliCommand::Export { dest, bundle })
        } else if let Some(source) = import_source {
            Some(CliCommand::Import {
                source,
                mode: import_mode,
                conflict,
            })
        } else {
            None
        };

        CliArgs {
            config_path: None,
            command,
            show_help: false,
        }
    }
}
