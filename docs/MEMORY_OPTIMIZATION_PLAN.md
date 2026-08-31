# MeoTool 内存优化开发计划

> 目标：长跑 48h 后 phys_footprint 稳定，隐藏稳态 ≤ **35MB**，折叠悬停不触发整面板重建。  
> 架构基线：[`NATIVE_UI_ARCHITECTURE.md`](./NATIVE_UI_ARCHITECTURE.md)（默认 `ui-native`）  
> 更新日期：2026-08-27

## 构建与度量

```bash
# 默认：AppKit 原生（低内存）
cargo build --release

# 内存基线采样
./scripts/memory-baseline.sh --bin target/release/meotool --seconds 300 --interval 30

# 48h 长跑（验收）
./scripts/memory-baseline.sh --seconds 172800 --interval 300
```

| 路径 | phys_footprint（隐藏稳态） | RSS |
|------|---------------------------|-----|
| egui App | ≈ 99–101MB | ≈ 78–80MB |
| **ui-native Release** | **≈ 20–40MB** | ≈ 72–79MB |

**主口径**：`phys_footprint`（`footprint -p` 或进程内 `task_vm_info`）。  
应用每 **120s** 输出 `MemoryStats` 日志（`RUST_LOG=info`）。

---

## 目标指标

| 场景 | phys_footprint | 48h 增长 |
|------|----------------|----------|
| 隐藏稳态 | ≤ 35MB | ≤ 5MB |
| 面板显示 | ≤ 50MB | ≤ 10MB |
| 折叠桌宠 + 悬停 | ≤ 45MB | ≤ 10MB |
| 关闭设置/图标选择器后 30s | 回到隐藏基线 ±3MB | — |

---

## 阶段总览

| 阶段 | 内容 | 状态 | 预估 |
|------|------|------|------|
| **0** | 原生 MemoryStats + phys/RSS + 计数器 | **已完成** | 1–2 天 |
| **1.1** | 折叠悬停增量更新（不整树 rebuild） | **已完成** | 2 天 |
| **1.2** | 原生 `NativeIconCache`（LRU cap 32） | **已完成** | 1 天 |
| **1.3** | `sync_panel` 降频、去掉热路径 clone | **已完成** | 0.5 天 |
| **2** | 隐藏 idle 回收（icon cache / mimalloc purge） | **已完成** | 1–2 天 |
| **3** | 窗口轮询与 ButtonFilter 去抖 | **已完成** | 1 天 |
| **4** | 48h soak 验收 + 删 egui（可选） | 待做 | 持续 |

---

## 阶段 0 — 可观测性（已完成）

### 交付

- `src/app/memory.rs`：`read_rss_mb()`、`read_phys_mb()`、`NativeMemoryCounters`
- 原生运行时每 120s 日志：
  ```
  MemoryStats ui=native rss_mb=… phys_mb=… panel_visible=… collapsed=…
  rebuilds=… fp_changes=… collapsed_toggles=… icon_loads=…
  ```
- 启动时打印 `startup memory ui=native rss_mb=… phys_mb=…`

### 计数器含义

| 字段 | 说明 |
|------|------|
| `rebuilds` | 整面板 `setContentView` 重建次数 |
| `fp_changes` | 配置指纹变化次数 |
| `collapsed_toggles` | 折叠悬停展开/收起（增量）次数 |
| `icon_loads` | PNG 解码次数（应随 rebuild 下降） |

### 验收

- Release 启动后 2 分钟内可见 `MemoryStats` 与 `startup memory` 日志。

---

## 阶段 1.1 — 折叠悬停增量更新（已完成）

### 问题

`fingerprint` 曾包含 `collapsed_expanded`，鼠标移入/移出桌宠每 50ms tick 触发**整树重建** + 全量 PNG 解码 → 长跑内存阶梯上涨。

### 方案

1. 从 `fingerprint` **移除** `collapsed_expanded`
2. `rebuild_collapsed` 始终构建 `grid_container` + `launcher`，初始 `setHidden(!expanded)`
3. 仅展开状态变化时调用 `apply_collapsed_expanded`：`setHidden` + 背景绘制切换
4. 保留 `CollapsedShell` 引用，避免重复 `setContentView`

### 验收

- 折叠模式悬停 100 次：`collapsed_toggles` 增加，`rebuilds` 不增加
- `icon_loads` 仅在配置变更后增加

---

## 阶段 1.2 — 原生图标 LRU（已完成）

### 交付

- `src/ui_native/icon_cache.rs`：`NativeIconCache`，常驻上限 **32**
- 面板 `load_nsimage` 改走 cache；`icon_loads` 仅在首次解码时增加
- `MemoryStats` 新增 `icons_resident=…`

### 验收

- 同一配置反复 rebuild：`icon_loads` 不随 rebuild 次数线性增长
- `icons_resident` ≤ 32

---

## 阶段 1.3 — sync 降频（已完成）

### 交付

- 面板**隐藏**且状态未变 → 跳过 `sync_panel`（不再每 50ms clone 配置）
- `sync_panel` 使用 `&[ButtonConfig]` / `&PanelConfig` 引用，去掉 `to_vec` / `clone`
- 折叠悬停 `expanded` 变化时设置 `panel_sync_dirty`

---

## 阶段 2 — Idle 回收（已完成）

### 交付

| 触发 | 动作 |
|------|------|
| 面板隐藏 | 立即 `clear_icon_cache()` |
| 隐藏 > **90s** | `mi_collect(true)` 归还 mimalloc 空闲页（每 30min 最多一次） |

日志：`idle memory purge (panel hidden Ns)`

---

## 阶段 3 — 轮询与 ButtonFilter 去抖（已完成）

1. **`WindowWatcherControl`**：面板隐藏时间隔 ×4（最低 800ms）
2. 主线程去掉冗余 `fetch_active_window()`，仅 drain `window_rx`
3. ButtonFilter：仅 `resolve_visibility` 结果变化才 `RefreshPanel`

---

## 阶段 4 — 验收（待做）

```bash
# 折叠模式 + 正常使用 48h
./scripts/memory-baseline.sh --seconds 172800 --interval 300

# 对照：仅隐藏
./scripts/memory-baseline.sh --seconds 172800 --interval 300
```

通过标准：phys 曲线无持续单调上升；`rebuilds` / `icon_loads` 与操作次数成正比而非与时间成正比。

---

## 日志排查速查

```bash
# 实时看内存统计
RUST_LOG=info ./target/release/meotool 2>&1 | grep MemoryStats

# 若 collapsed_toggles 涨而 rebuilds 不涨 → 1.1 生效
# 若 icon_loads 随时间线性涨 → 查 rebuild 或缺 LRU（1.2）
# 若 phys 涨但 rebuilds 稳定 → 阶段 2 purge / AppKit 缓存
```

---

## 与历史文档

- egui 路径止血记录仍见上文「实测对照」；**新目标以本计划为准**。
- 领域行为见 `MOUSE_MACRO.md` / `BUTTON_APP_VISIBILITY.md` / `DESIGN.md`。
