# MeoTool 换架构方案：最低内存优先

> 目标：隐藏稳态 **phys_footprint ≤ 35MB**，面板显示 ≤ 50MB；活动监视器「内存」尽量 ≤ 45MB。  
> 前提：当前 egui + glow 实测地板约 **~100MB phys / ~80MB RSS**（2026-08-14），继续拧缓存无法到 50MB。  
> 状态：阶段 1 done；阶段 2 MVP 已合入（默认 `ui-native`）。  
> 日期：2026-08-14

---

## 1. 结论（先看这个）

**选定架构：Rust 领域核心 + AppKit 原生 UI（无 OpenGL / 无 egui / 无常驻 GPU 上下文）**

| 方案 | 隐藏稳态预期 | 取舍 | 决议 |
|------|--------------|------|------|
| A. 继续 egui 调参 | ~80–100MB | 已实测触顶 | **否** |
| B. egui 隐藏时销毁 GL 上下文 | 不确定，唤起慢/闪 | 工程脏、仍可能 ≥60MB | **否** |
| C. WebView / Tauri | 往往更高 | 内存更差 | **否** |
| D. iced / slint / wgpu | 仍偏高 | 换皮不换骨 | **否** |
| **E. AppKit `NSPanel` + Rust 核心** | **~20–40MB** | 重写 UI ~5k LOC | **采用** |
| F. 双进程（agent + UI） | 可再低一点 | IPC 复杂、调试成本高 | 二期可选 |

内存杀手不是配置/热键/宏，而是 **透明 AlwaysOnTop 的常驻 OpenGL 根窗口**（`vmmap` 中 `IOAccelerator` 等）。去掉这条路径，才有机会压到 50MB 以下。

---

## 2. 目标架构

```
┌─────────────────────────────────────────────────────────┐
│  MeoTool.app（单进程，macOS only 先保内存）              │
│                                                         │
│  ┌──────────────────┐     wake / command      ┌───────┐ │
│  │  Rust Core       │◄───────────────────────►│ AppKit│ │
│  │  config/action/  │     NSPanel / NSWindow  │ Chrome│ │
│  │  hotkey/gesture/ │     NSImage 图标        │       │ │
│  │  macro/tray/AX   │     RunLoop 驱动        │       │ │
│  └──────────────────┘                         └───────┘ │
│           │                                             │
│           ▼                                             │
│  CGEventTap / global-hotkey / enigo（已有，保留）        │
└─────────────────────────────────────────────────────────┘
```

### 2.1 进程与事件循环

- **单一进程**：避免双进程常驻翻倍；用 `CFRunLoop` / AppKit 主线程驱动。
- **无帧渲染循环**：面板隐藏时不 tick UI；仅热键/手势/`mpsc` wake 唤醒。
- **面板**：`NSPanel`（`NSWindowStyleMask.nonactivatingPanel` + `floating` / `statusBar` 级），按需 `orderFront` / `orderOut`；隐藏时 **不保留 GL 后备缓冲**（AppKit 图层可释放）。
- **设置 / 编辑器 / 权限 / 图标选择**：普通 `NSWindow` 或 `NSPanel`，用完关闭并释放；不与主面板抢 AlwaysOnTop。

### 2.2 模块切分（目标树）

| 层 | 职责 | 现状 → 目标 |
|----|------|-------------|
| `core/` | 配置、动作、宏、窗口解析、无 UI 类型 | 从 `config`/`action`/`window` 抽出；去掉 `egui::Color32` |
| `platform/` | AX、EventTap、tray、bounds | **基本保留**；`wake_ctx` 改为 `WakeHandle` |
| `ui_native/` | AppKit 面板/设置/编辑器（`objc2`） | **新建**，替代 `ui/` |
| `bridge/` | Rust ↔ AppKit 命令与状态快照 | **新建**（消息枚举，禁止 UI 直接摸 config 文件） |
| `main` | 组装 RunLoop，不再 `eframe::run_native` | 重写入口 |

现有 `ui/`（~4.7k LOC）与 `main.rs` 中 egui 编排 **整层退役**，不做成双 UI 长期并存（可用 feature 短期对照，见阶段 0）。

### 2.3 唤醒通道（替代 `egui::Context::request_repaint`）

统一抽象：

```text
WakeHandle::poke()  →  主线程 drain 事件
事件源：热键 / 手势 / tray / 配置热更新 / 宏录制步进
```

实现：`mpsc` + `CFRunLoopSource`（或 AppKit `performSelectorOnMainThread`）。  
热键、手势、tray、宏录制只依赖 `WakeHandle`，不依赖任何 UI 框架。

### 2.4 绘制与字体

- **面板按钮**：`NSButton` / 自定义 `NSView` + `NSImage`（PNG 图标，边长 ≤48，LRU 或按配置预载当前 profile）。
- **文字**：系统字体（`NSFont.systemFont`），**不再内嵌 ui-cjk.ttf**，也不建 egui 字形图集 → 再省 ~1MB 堆 + 图集峰值。
- **主题色**：存 `#RRGGBB` / `u32`，映射到 `NSColor`；配置层禁止依赖 egui。

### 2.5 功能对等（必须保住）

| 功能 | 原生实现要点 |
|------|----------------|
| 浮层快捷面板 / 折叠宠物模式 | `NSPanel` + 透明内容视图；passthrough 用 `ignoresMouseEvents` |
| 全局热键 / 数字键 | 现有 `global-hotkey` |
| 右键拖拽手势 | 现有懒启动 `CGEventTap` |
| 宏录制回放 | 现有 tap + `enigo` |
| 设置 / 按钮编辑 / 图标选择 | AppKit 窗口；图标列表用 `NSCollectionView` 虚拟化 |
| 权限引导 | 现有 TCC 探测 + System Settings URL |
| 托盘与菜单 | 现有 `tray-icon` 或改为 `NSStatusItem` |

可接受降级（换内存）：复杂圆角阴影动画、egui 级即时主题预览帧率、超大图标库一次解码。

---

## 3. 内存预算（设计目标）

| 部件 | 预算（隐藏稳态） |
|------|------------------|
| Rust 堆（config + 状态） | ≤ 8MB |
| 系统库 mapped（分摊） | ≤ 15MB |
| AppKit / 托盘 / 无面板 | ≤ 8MB |
| EventTap 线程 | ≤ 3MB |
| 余量 | ≤ 5MB |
| **合计 phys** | **≤ 35MB** |

面板显示时：多一块小 `NSPanel` 图层与当前页图标，目标 **≤ 50MB phys**。  
打开设置/图标库允许临时冲高，关闭后 30s 内回到隐藏预算。

度量：继续用 `vmmap` Physical footprint + `scripts/memory-baseline.sh`（入口改为新二进制）。

---

## 4. 技术选型细节

| 项 | 选择 | 理由 |
|----|------|------|
| UI 绑定 | `objc2` + `objc2-app-kit` | 与现有 tray/依赖生态一致，纯 Rust 仓库可维护 |
| 不引入 Swift | 首期 | 避免双语言构建；二期若写复杂 Preference 再评估 |
| 渲染 | AppKit / Core Animation | 无常驻 GL/Metal 应用上下文 |
| 分配器 | 保留 `mimalloc` | 对长跑碎片仍有利 |
| 平台 | 先 **macOS only** 保证内存目标 | 其它 OS 仍可暂时停在 egui feature |

Feature 建议：

```toml
default = ["ui-native"]
ui-native = []          # AppKit 路径（内存优先）
ui-egui = ["eframe", "egui", "renderer-glow"]  # 对照 / 回退，最终删除
```

---

## 5. 分阶段落地

### 阶段 0 — 解耦（不降内存，但必须做）≈ 3–5 天 — **进行中 / 骨架已合入**

1. ~~引入 `WakeHandle`，替换 `hotkey` / `mouse_monitor` / `macro_recorder` / `tray` 中的 `egui::Context`。~~ **done**（`src/app/wake.rs`）
2. ~~配置颜色改为 `RgbColor`，与 egui 的转换仅留在 `ui/egui_color.rs`。~~ **done**
3. ~~抽出 `AppCore`：`handle(Command)` 骨架。~~ **done**（`src/app/core.rs`；热键已先走 `handle` 再执行 UI）
4. ~~Feature 对照：`ui-egui` / `ui-native`。~~ **done**（现默认已切 `ui-native`，见阶段 2）

**验收：** 现有 egui 行为不变；`cargo test` / Release 可编译。

### 阶段 1 — 原生主面板 MVP ≈ 1.5–2.5 周 — **done**

1. ~~`ui_native`：可显示/隐藏的 `NSPanel` 网格~~ **done**（`src/ui_native/`）
2. ~~入口切到 AppKit RunLoop；隐藏时不跑 GL~~ **done**
3. ~~托盘 / 热键 / 手势接通 `AppCore`~~ **done**
4. ~~设置页暂缓~~ → 阶段 2 已补齐原生设置

**验收：** Release 原生隐藏 `phys_footprint ≈ 20MB`；热键唤起；按钮可执行。

### 阶段 2 — 设置与编辑器原生化 — **MVP 已合入（2026-08-14）**

1. ~~设置窗口（全局 / 热键 / 配置档要点）~~ **done**（`settings.rs`；细外观仍可「打开配置文件」）
2. ~~按钮编辑~~ **done**（`button_editor.rs`；鼠标宏步骤仍建议配置文件编辑，保存保留原宏）
3. ~~图标选择~~ **done**（`icon_picker.rs`：弹出列表 MVP；虚拟化列表可后续加强）
4. ~~权限引导窗口~~ **done**（`permissions.rs`）
5. ~~默认切 `ui-native`~~ **done**；`ui-egui` 仍可用作对照，**尚未删除** eframe

**构建：**

```bash
# 默认：原生 AppKit（低内存）
cargo build --release

# egui 对照 / 回归测试
cargo build --release --no-default-features --features ui-egui
cargo test --release --no-default-features --features ui-egui
```

**验收（进行中）：** 设置/编辑/权限可打开并写回配置；隐藏稳态仍以 phys 为准；完整删 egui 留待确认对等后。

### 阶段 3 — 极致（可选）≈ 1 周

1. 托盘改为纯 `NSStatusItem`，去掉多余 crate。
2. 评估「设置独立 helper 进程」（仅打开设置时拉起）——若隐藏态仍 >35MB 再做。
3. 折叠宠物模式与 mouse passthrough 打磨。

---

## 6. 风险与对策

| 风险 | 对策 |
|------|------|
| AppKit 绑定工作量大 | 面板先 NSButton 网格，视觉可先朴素；设置用标准控件 |
| 透明面板点击穿透 | `ignoresMouseEvents` + 命中测试，对标现 `MousePassthrough` |
| AlwaysOnTop vs 权限密码框 | 打开系统设置前 `orderOut` 主面板（沿用现逻辑） |
| 双 UI 分叉过久 | 阶段 1 设 deadline；阶段 2 结束必须删 egui |
| 内存仍不达标 | 查是否误留 Metal/GL；设置改 helper 进程 |

---

## 7. 明确不做

- 不在 egui 上继续「冲 50MB」。
- 不上 WebView / Electron / Tauri。
- 不把「换 wgpu」当架构方案（实测不是主因）。
- 不维护两套完整 UI 超过阶段 2。

---

## 8. 与现有文档关系

- `MEMORY_OPTIMIZATION_PLAN.md`：egui 路径的止血与实测记录；**后续内存目标以本文为准**。
- 领域行为（宏、按钮可见性等）仍以 `MOUSE_MACRO.md` / `BUTTON_APP_VISIBILITY.md` / `DESIGN.md` 为准，原生 UI 只换壳。

---

## 9. 建议的下一步

1. 本机实测：默认 Release 隐藏稳态 + 打开/关闭设置后内存是否回落。
2. 打磨：宏录制原生 UI、图标虚拟化列表、外观页。
3. 确认功能对等后删除 `ui-egui` / `eframe`（阶段 2 收尾）。

```bash
cargo build --release
./scripts/memory-baseline.sh --bin target/release/meotool --seconds 180
```
