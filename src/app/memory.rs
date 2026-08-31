//! 运行时内存与缓存占用观测（Release / Debug 均可）。

use std::time::{Duration, Instant};

/// MemoryStats 日志间隔
pub const STATS_LOG_INTERVAL: Duration = Duration::from_secs(120);

#[cfg(feature = "ui-egui")]
pub const FONT_ATLAS_RESET_RATIO: f32 = 0.45;
#[cfg(feature = "ui-egui")]
pub const FONT_RESET_MIN_INTERVAL: Duration = Duration::from_secs(2 * 60);
#[cfg(feature = "ui-egui")]
pub const HIDDEN_FONT_RESET_AFTER: Duration = Duration::from_secs(90);
/// 面板隐藏超过此时长后尝试归还堆内存给 OS
pub const HIDDEN_IDLE_PURGE_AFTER: Duration = Duration::from_secs(90);
/// 两次 purge 之间的最小间隔
pub const PURGE_MIN_INTERVAL: Duration = Duration::from_secs(30 * 60);

/// 原生 UI 路径的运行时计数（累计值，用于长跑泄漏定位）。
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeMemoryCounters {
    pub panel_rebuilds: u32,
    pub fingerprint_changes: u32,
    pub collapsed_toggles: u32,
    pub icon_loads: u32,
}

impl NativeMemoryCounters {
    pub fn record_icon_load(&mut self) {
        self.icon_loads = self.icon_loads.saturating_add(1);
    }

    pub fn record_panel_rebuild(&mut self) {
        self.panel_rebuilds = self.panel_rebuilds.saturating_add(1);
    }

    pub fn record_fingerprint_change(&mut self) {
        self.fingerprint_changes = self.fingerprint_changes.saturating_add(1);
    }

    pub fn record_collapsed_toggle(&mut self) {
        self.collapsed_toggles = self.collapsed_toggles.saturating_add(1);
    }
}

pub struct MemoryMonitor {
    last_stats_log: Instant,
    #[cfg(feature = "ui-egui")]
    last_font_reset: Instant,
    #[cfg(feature = "ui-egui")]
    pub font_resets: u32,
}

impl Default for MemoryMonitor {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            last_stats_log: now,
            #[cfg(feature = "ui-egui")]
            last_font_reset: now,
            #[cfg(feature = "ui-egui")]
            font_resets: 0,
        }
    }
}

impl MemoryMonitor {
    /// 原生路径：定期打印 RSS / phys 与面板计数。
    pub fn maybe_log_native(
        &mut self,
        counters: &NativeMemoryCounters,
        panel_visible: bool,
        collapsed: bool,
        icons_resident: usize,
    ) {
        if self.last_stats_log.elapsed() < STATS_LOG_INTERVAL {
            return;
        }
        self.last_stats_log = Instant::now();

        let rss = read_rss_mb();
        let phys = read_phys_mb();

        log::info!(
            "MemoryStats ui=native rss_mb={} phys_mb={} panel_visible={} collapsed={} \
             rebuilds={} fp_changes={} collapsed_toggles={} icon_loads={} icons_resident={}",
            fmt_mb(rss),
            fmt_mb(phys),
            panel_visible,
            collapsed,
            counters.panel_rebuilds,
            counters.fingerprint_changes,
            counters.collapsed_toggles,
            counters.icon_loads,
            icons_resident,
        );
    }

    #[cfg(feature = "ui-egui")]
    pub fn should_reset_fonts_ex(&self, ctx: &egui::Context, force_idle: bool) -> bool {
        if self.last_font_reset.elapsed() < FONT_RESET_MIN_INTERVAL {
            return false;
        }
        if force_idle {
            return true;
        }
        let ratio = ctx.fonts(|f| f.font_atlas_fill_ratio());
        ratio >= FONT_ATLAS_RESET_RATIO
    }

    #[cfg(feature = "ui-egui")]
    pub fn mark_font_reset(&mut self) {
        self.last_font_reset = Instant::now();
        self.font_resets = self.font_resets.saturating_add(1);
    }

    #[cfg(feature = "ui-egui")]
    pub fn maybe_log(
        &mut self,
        ctx: &egui::Context,
        icons: &crate::ui::IconCache,
        panel_visible: bool,
    ) {
        if self.last_stats_log.elapsed() < STATS_LOG_INTERVAL {
            return;
        }
        self.last_stats_log = Instant::now();

        let (atlas_ratio, galleys) =
            ctx.fonts(|f| (f.font_atlas_fill_ratio(), f.num_galleys_in_cache()));
        let resident = icons.resident_count();
        let picker = icons.picker_count();
        let rss = read_rss_mb();
        let phys = read_phys_mb();

        log::info!(
            "MemoryStats ui=egui rss_mb={} phys_mb={} atlas_fill={:.2} galleys={} \
             icons_resident={} icons_picker={} font_resets={} panel_visible={}",
            fmt_mb(rss),
            fmt_mb(phys),
            atlas_ratio,
            galleys,
            resident,
            picker,
            self.font_resets,
            panel_visible
        );
    }
}

fn fmt_mb(v: Option<f64>) -> String {
    v.map(|n| format!("{n:.1}"))
        .unwrap_or_else(|| "?".into())
}

/// 隐藏稳态：尝试将 mimalloc 空闲页归还 OS（best-effort）。
pub fn purge_allocator() {
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn mi_collect(force: bool) -> usize;
        }
        unsafe {
            let _ = mi_collect(true);
        }
    }
}

/// 读取本进程 RSS（MiB）。失败返回 None。
pub fn read_rss_mb() -> Option<f64> {
    #[cfg(target_os = "macos")]
    {
        macos_rss_bytes().map(|b| b as f64 / (1024.0 * 1024.0))
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// 读取本进程 phys_footprint（MiB）。失败返回 None。
pub fn read_phys_mb() -> Option<f64> {
    #[cfg(target_os = "macos")]
    {
        macos_phys_footprint_bytes().map(|b| b as f64 / (1024.0 * 1024.0))
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(target_os = "macos")]
fn macos_rss_bytes() -> Option<u64> {
    use std::mem::MaybeUninit;

    #[repr(C)]
    struct MachTaskBasicInfo {
        virtual_size: u64,
        resident_size: u64,
        resident_size_max: u64,
        user_time: [u32; 2],
        system_time: [u32; 2],
        policy: i32,
        suspend_count: i32,
    }

    const MACH_TASK_BASIC_INFO: u32 = 20;
    const MACH_TASK_BASIC_INFO_COUNT: u32 =
        (std::mem::size_of::<MachTaskBasicInfo>() / std::mem::size_of::<u32>()) as u32;

    extern "C" {
        fn mach_task_self() -> u32;
        fn task_info(
            target_task: u32,
            flavor: u32,
            task_info_out: *mut u32,
            task_info_outCnt: *mut u32,
        ) -> i32;
    }

    unsafe {
        let mut info = MaybeUninit::<MachTaskBasicInfo>::uninit();
        let mut count = MACH_TASK_BASIC_INFO_COUNT;
        let kr = task_info(
            mach_task_self(),
            MACH_TASK_BASIC_INFO,
            info.as_mut_ptr() as *mut u32,
            &mut count,
        );
        if kr != 0 {
            return None;
        }
        Some(info.assume_init().resident_size)
    }
}

#[cfg(target_os = "macos")]
fn macos_phys_footprint_bytes() -> Option<u64> {
    use std::mem::MaybeUninit;

    // task_vm_info_data_t（macOS 11+）；phys_footprint 字段偏移稳定。
    #[repr(C)]
    struct TaskVmInfo {
        virtual_size: u64,
        region_count: u32,
        page_size: u32,
        _pad0: u32,
        _dyld_pager: u64,
        _object_id: u64,
        phys_footprint: u64,
    }

    const TASK_VM_INFO: u32 = 22;
    const TASK_VM_INFO_COUNT: u32 =
        (std::mem::size_of::<TaskVmInfo>() / std::mem::size_of::<u32>()) as u32;

    extern "C" {
        fn mach_task_self() -> u32;
        fn task_info(
            target_task: u32,
            flavor: u32,
            task_info_out: *mut u32,
            task_info_outCnt: *mut u32,
        ) -> i32;
    }

    unsafe {
        let mut info = MaybeUninit::<TaskVmInfo>::uninit();
        let mut count = TASK_VM_INFO_COUNT;
        let kr = task_info(
            mach_task_self(),
            TASK_VM_INFO,
            info.as_mut_ptr() as *mut u32,
            &mut count,
        );
        if kr != 0 {
            return None;
        }
        Some(info.assume_init().phys_footprint)
    }
}
