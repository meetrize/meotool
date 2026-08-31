use std::path::PathBuf;
use std::sync::mpsc;
#[cfg(any(feature = "ui-egui", test))]
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

const DEBOUNCE_MS: u64 = 300;

/// 监听配置文件变更，通过 channel 通知主线程
pub fn spawn_config_watcher(
    config_path: PathBuf,
    tx: mpsc::Sender<()>,
) -> Result<RecommendedWatcher, String> {
    let watch_path = config_path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| config_path.clone());

    let config_name = config_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config.toml")
        .to_string();

    let mut watcher = RecommendedWatcher::new(
        move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                let relevant = matches!(
                    event.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Any
                ) && event.paths.iter().any(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n == config_name)
                        .unwrap_or(false)
                });
                if relevant {
                    let _ = tx.send(());
                }
            }
        },
        notify::Config::default(),
    )
    .map_err(|e| format!("config watcher: {e}"))?;

    watcher
        .watch(&watch_path, RecursiveMode::NonRecursive)
        .map_err(|e| format!("watch {}: {e}", watch_path.display()))?;

    log::info!(
        "watching config changes in {} (debounce {}ms)",
        watch_path.display(),
        DEBOUNCE_MS
    );
    Ok(watcher)
}

#[cfg(any(feature = "ui-egui", test))]
pub fn debounce_duration() -> Duration {
    Duration::from_millis(DEBOUNCE_MS)
}
