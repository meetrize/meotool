use std::path::{Path, PathBuf};
use std::sync::mpsc;

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::app::WakeHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    TogglePanel,
    OpenSettings,
    Quit,
    /// 面板槽位 index，与面板点击 / 热键触发一致
    TriggerButton(usize),
}

pub struct TrayManager {
    /// 必须持有，否则菜单栏图标会被释放
    tray: TrayIcon,
    /// macOS 顶部应用菜单（左侧 MeoTool 菜单），需保持存活
    #[cfg(target_os = "macos")]
    #[allow(dead_code)]
    _app_menu: Menu,
    /// 最近一次同步的按钮菜单快照；未变则跳过 rebuild
    last_button_entries: Vec<(usize, String)>,
    button_ids: Vec<(MenuId, usize)>,
    toggle_id: MenuId,
    settings_id: MenuId,
    #[cfg(target_os = "macos")]
    app_settings_id: MenuId,
    quit_id: MenuId,
    #[cfg(target_os = "macos")]
    app_quit_id: MenuId,
    event_rx: mpsc::Receiver<MenuId>,
    #[cfg_attr(not(any(feature = "ui-egui", test)), allow(dead_code))]
    icon_dirs: Vec<PathBuf>,
    #[cfg_attr(not(any(feature = "ui-egui", test)), allow(dead_code))]
    current_icon: String,
}

impl TrayManager {
    pub fn install(
        wake: WakeHandle,
        icon_dirs: &[PathBuf],
        icon_name: &str,
    ) -> Result<Self, String> {
        let icon_name = normalize_icon_name(icon_name);
        let icon = build_icon(icon_dirs, &icon_name)?;

        let built = build_tray_menu(&[])?;

        let (event_tx, event_rx) = mpsc::channel();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let _ = event_tx.send(event.id);
            wake.poke();
        }));

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(built.menu))
            .with_menu_on_left_click(true)
            .with_tooltip("MeoTool")
            .with_icon(icon)
            .build()
            .map_err(|e| format!("create tray: {e}"))?;

        #[cfg(target_os = "macos")]
        let (app_menu, app_settings_id, app_quit_id) = install_app_menu()?;

        log::info!("menu bar tray icon installed ({icon_name})");
        Ok(Self {
            tray,
            #[cfg(target_os = "macos")]
            _app_menu: app_menu,
            last_button_entries: Vec::new(),
            button_ids: built.button_ids,
            toggle_id: built.toggle_id,
            settings_id: built.settings_id,
            #[cfg(target_os = "macos")]
            app_settings_id,
            quit_id: built.quit_id,
            #[cfg(target_os = "macos")]
            app_quit_id,
            event_rx,
            icon_dirs: icon_dirs.to_vec(),
            current_icon: icon_name,
        })
    }

    /// 按条目重建托盘下拉菜单；`entries` 为空时仅保留固定三项。
    /// 内容与上次相同则跳过。
    pub fn rebuild_menu(&mut self, entries: &[(usize, String)]) {
        if entries == self.last_button_entries.as_slice() {
            return;
        }
        match build_tray_menu(entries) {
            Ok(built) => {
                self.tray.set_menu(Some(Box::new(built.menu)));
                self.toggle_id = built.toggle_id;
                self.settings_id = built.settings_id;
                self.quit_id = built.quit_id;
                self.button_ids = built.button_ids;
                self.last_button_entries = entries.to_vec();
            }
            Err(err) => log::warn!("rebuild tray menu failed: {err}"),
        }
    }

    /// 同步菜单栏图标为当前折叠桌宠图标（名称未变则跳过）
    #[cfg_attr(not(any(feature = "ui-egui", test)), allow(dead_code))]
    pub fn sync_icon(&mut self, icon_name: &str) {
        let icon_name = normalize_icon_name(icon_name);
        if icon_name == self.current_icon {
            return;
        }
        match build_icon(&self.icon_dirs, &icon_name) {
            Ok(icon) => {
                if let Err(err) = self.tray.set_icon(Some(icon)) {
                    log::warn!("update tray icon failed: {err}");
                    return;
                }
                log::info!("tray icon updated to {icon_name}");
                self.current_icon = icon_name;
            }
            Err(err) => log::warn!("rebuild tray icon failed: {err}"),
        }
    }

    pub fn poll(&self) -> Option<TrayAction> {
        while let Ok(id) = self.event_rx.try_recv() {
            if id == self.toggle_id {
                return Some(TrayAction::TogglePanel);
            }
            if id == self.settings_id {
                return Some(TrayAction::OpenSettings);
            }
            #[cfg(target_os = "macos")]
            if id == self.app_settings_id {
                return Some(TrayAction::OpenSettings);
            }
            if id == self.quit_id {
                return Some(TrayAction::Quit);
            }
            #[cfg(target_os = "macos")]
            if id == self.app_quit_id {
                return Some(TrayAction::Quit);
            }
            if let Some((_, index)) = self.button_ids.iter().find(|(mid, _)| *mid == id) {
                return Some(TrayAction::TriggerButton(*index));
            }
        }
        None
    }
}

struct BuiltTrayMenu {
    menu: Menu,
    toggle_id: MenuId,
    settings_id: MenuId,
    quit_id: MenuId,
    button_ids: Vec<(MenuId, usize)>,
}

fn build_tray_menu(entries: &[(usize, String)]) -> Result<BuiltTrayMenu, String> {
    let mut button_ids = Vec::with_capacity(entries.len());
    let mut button_items = Vec::with_capacity(entries.len());
    for (index, title) in entries {
        let item = MenuItem::new(title, true, None);
        button_ids.push((item.id().clone(), *index));
        button_items.push(item);
    }

    let toggle_item = MenuItem::new("显示面板", true, None);
    let settings_item = MenuItem::new("设置", true, None);
    let quit_item = MenuItem::new("退出", true, None);

    let toggle_id = toggle_item.id().clone();
    let settings_id = settings_item.id().clone();
    let quit_id = quit_item.id().clone();

    let menu = Menu::new();
    for item in &button_items {
        menu.append(item)
            .map_err(|e| format!("tray menu button: {e}"))?;
    }
    if !button_items.is_empty() {
        menu.append(&PredefinedMenuItem::separator())
            .map_err(|e| format!("tray menu: {e}"))?;
    }
    menu.append(&toggle_item)
        .map_err(|e| format!("tray menu: {e}"))?;
    menu.append(&settings_item)
        .map_err(|e| format!("tray menu: {e}"))?;
    menu.append(&PredefinedMenuItem::separator())
        .map_err(|e| format!("tray menu: {e}"))?;
    menu.append(&quit_item)
        .map_err(|e| format!("tray menu: {e}"))?;

    Ok(BuiltTrayMenu {
        menu,
        toggle_id,
        settings_id,
        quit_id,
        button_ids,
    })
}

#[cfg(target_os = "macos")]
fn install_app_menu() -> Result<(Menu, MenuId, MenuId), String> {
    let settings_item = MenuItem::new("设置...", true, None);
    let quit_item = MenuItem::new("退出 MeoTool", true, None);
    let settings_id = settings_item.id().clone();
    let quit_id = quit_item.id().clone();

    let app_submenu = Submenu::new("MeoTool", true);
    app_submenu
        .append(&settings_item)
        .map_err(|e| format!("app menu settings: {e}"))?;
    app_submenu
        .append(&PredefinedMenuItem::separator())
        .map_err(|e| format!("app menu: {e}"))?;
    app_submenu
        .append(&PredefinedMenuItem::hide(None))
        .map_err(|e| format!("app menu: {e}"))?;
    app_submenu
        .append(&PredefinedMenuItem::hide_others(None))
        .map_err(|e| format!("app menu: {e}"))?;
    app_submenu
        .append(&PredefinedMenuItem::show_all(None))
        .map_err(|e| format!("app menu: {e}"))?;
    app_submenu
        .append(&PredefinedMenuItem::separator())
        .map_err(|e| format!("app menu: {e}"))?;
    app_submenu
        .append(&quit_item)
        .map_err(|e| format!("app menu quit: {e}"))?;

    let menu = Menu::new();
    menu.append(&app_submenu)
        .map_err(|e| format!("app menu root: {e}"))?;
    menu.init_for_nsapp();

    log::info!("macOS application menu installed");
    Ok((menu, settings_id, quit_id))
}

fn normalize_icon_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        crate::config::PanelConfig::DEFAULT_COLLAPSED_ICON.to_string()
    } else {
        trimmed.to_string()
    }
}

fn resolve_icon_path(search_dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    for dir in search_dirs {
        let path = dir.join(name);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn build_icon(search_dirs: &[PathBuf], icon_name: &str) -> Result<Icon, String> {
    if let Some(path) = resolve_icon_path(search_dirs, icon_name) {
        match load_tray_icon_png(&path) {
            Ok(icon) => return Ok(icon),
            Err(err) => log::warn!(
                "tray icon load failed for {}: {err}; falling back",
                path.display()
            ),
        }
    } else {
        log::warn!("tray icon not found: {icon_name}; falling back");
    }
    build_fallback_icon().map_err(|e| format!("tray icon: {e}"))
}

fn load_tray_icon_png(path: &Path) -> Result<Icon, String> {
    // 菜单栏常用约 22pt；用 44 像素兼顾 Retina
    const SIZE: u32 = 44;
    let img = image::open(path).map_err(|e| format!("open: {e}"))?;
    let rgba = img.to_rgba8();
    let (src_w, src_h) = rgba.dimensions();
    if src_w == 0 || src_h == 0 {
        return Err("empty image".into());
    }

    let scale = (SIZE as f32 / src_w as f32).min(SIZE as f32 / src_h as f32);
    let nw = ((src_w as f32) * scale).round().max(1.0) as u32;
    let nh = ((src_h as f32) * scale).round().max(1.0) as u32;
    let resized = image::imageops::resize(&rgba, nw, nh, image::imageops::FilterType::Lanczos3);

    let mut out = vec![0u8; (SIZE * SIZE * 4) as usize];
    let ox = (SIZE - nw) / 2;
    let oy = (SIZE - nh) / 2;
    for y in 0..nh {
        for x in 0..nw {
            let px = resized.get_pixel(x, y).0;
            let dst = (((oy + y) * SIZE + (ox + x)) * 4) as usize;
            out[dst..dst + 4].copy_from_slice(&px);
        }
    }

    Icon::from_rgba(out, SIZE, SIZE).map_err(|e| format!("{e}"))
}

fn build_fallback_icon() -> Result<Icon, tray_icon::BadIcon> {
    const SIZE: u32 = 22;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let cx = SIZE as f32 / 2.0 - 0.5;
            let cy = SIZE as f32 / 2.0 - 0.5;
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist <= 8.0 {
                rgba.extend_from_slice(&[80, 160, 255, 255]);
            } else if dist <= 9.5 {
                rgba.extend_from_slice(&[40, 90, 180, 255]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    Icon::from_rgba(rgba, SIZE, SIZE)
}

/// 预留：通过 channel 转发菜单事件（与 EventLoopProxy 配合）
#[allow(dead_code)]
pub fn spawn_menu_forwarder(tx: mpsc::Sender<TrayAction>) {
    let _ = tx;
}
