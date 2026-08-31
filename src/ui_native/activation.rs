//! Accessory 应用打开设置/编辑等窗时，临时前台激活，避免窗体沉在其它应用后面。

use std::sync::atomic::{AtomicUsize, Ordering};

use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSEventModifierFlags, NSFloatingWindowLevel,
    NSMenu, NSMenuItem, NSNormalWindowLevel, NSWindow, NSWindowLevel,
};
use objc2_foundation::{MainThreadMarker, NSString};

static ELEVATED: AtomicUsize = AtomicUsize::new(0);

/// 安装最小主菜单（含「编辑」），作为文本快捷键的兜底路由。
///
/// Accessory / LSUIElement 应用默认无 Edit 菜单；编辑按钮窗还会额外装本地 KeyDown 监视。
pub fn install_standard_main_menu(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);

    unsafe {
        let main_menu = NSMenu::new(mtm);

        // 应用菜单（菜单栏左侧应用名）
        let app_item = NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str(""),
            None,
            &NSString::from_str(""),
        );
        let app_menu = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str("MeoTool"));
        let quit = NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str("退出 MeoTool"),
            Some(objc2::sel!(terminate:)),
            &NSString::from_str("q"),
        );
        app_menu.addItem(&quit);
        app_item.setSubmenu(Some(&app_menu));
        main_menu.addItem(&app_item);

        // 编辑菜单：标准 first-responder actions
        let edit_item = NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str("编辑"),
            None,
            &NSString::from_str(""),
        );
        let edit_menu = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str("编辑"));
        add_edit_item(mtm, &edit_menu, "撤销", objc2::sel!(undo:), "z", false);
        add_edit_item(mtm, &edit_menu, "重做", objc2::sel!(redo:), "z", true);
        edit_menu.addItem(&NSMenuItem::separatorItem(mtm));
        add_edit_item(mtm, &edit_menu, "剪切", objc2::sel!(cut:), "x", false);
        add_edit_item(mtm, &edit_menu, "复制", objc2::sel!(copy:), "c", false);
        add_edit_item(mtm, &edit_menu, "粘贴", objc2::sel!(paste:), "v", false);
        add_edit_item(mtm, &edit_menu, "全选", objc2::sel!(selectAll:), "a", false);
        edit_item.setSubmenu(Some(&edit_menu));
        main_menu.addItem(&edit_item);

        app.setMainMenu(Some(&main_menu));
    }
}

fn add_edit_item(
    mtm: MainThreadMarker,
    menu: &NSMenu,
    title: &str,
    action: objc2::runtime::Sel,
    key: &str,
    shift: bool,
) {
    unsafe {
        let item = NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str(title),
            Some(action),
            &NSString::from_str(key),
        );
        // target 为空：走 responder chain（NSTextView / field editor）
        let mods = if shift {
            NSEventModifierFlags::Command | NSEventModifierFlags::Shift
        } else {
            NSEventModifierFlags::Command
        };
        item.setKeyEquivalentModifierMask(mods);
        menu.addItem(&item);
    }
}

/// `held`：该窗是否已占用一次 elevation（防重复 show/hide 打乱计数）。
pub fn present_utility_window(mtm: MainThreadMarker, window: &NSWindow, held: &mut bool) {
    present_utility_window_at_level(mtm, window, held, NSFloatingWindowLevel);
}

/// 设置窗用普通层级，避免盖住浮层面板，便于边改边看。
pub fn present_settings_window(mtm: MainThreadMarker, window: &NSWindow, held: &mut bool) {
    present_utility_window_at_level(mtm, window, held, NSNormalWindowLevel);
}

fn present_utility_window_at_level(
    mtm: MainThreadMarker,
    window: &NSWindow,
    held: &mut bool,
    level: NSWindowLevel,
) {
    let app = NSApplication::sharedApplication(mtm);
    if !*held {
        if ELEVATED.fetch_add(1, Ordering::SeqCst) == 0 {
            unsafe {
                app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
            }
            // 切到 Regular 后再装一次菜单，确保菜单栏键等价可用
            install_standard_main_menu(mtm);
        }
        *held = true;
    }
    unsafe {
        app.activate();
        window.setLevel(level);
        window.center();
        window.makeKeyAndOrderFront(None);
        window.orderFrontRegardless();
    }
}

pub fn dismiss_utility_window(mtm: MainThreadMarker, window: &NSWindow, held: &mut bool) {
    unsafe {
        window.orderOut(None);
        window.setLevel(NSNormalWindowLevel);
    }
    if !*held {
        return;
    }
    *held = false;
    let prev = ELEVATED.fetch_sub(1, Ordering::SeqCst);
    if prev <= 1 {
        ELEVATED.store(0, Ordering::SeqCst);
        let app = NSApplication::sharedApplication(mtm);
        unsafe {
            app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        }
    }
}
