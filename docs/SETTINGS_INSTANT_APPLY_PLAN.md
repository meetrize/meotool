# MeoTool 设置窗「即改即生效」改造方案与开发计划

> 目标：设置中所有项变更后立即生效并持久化；关闭设置窗无需保存/取消，不出现误报「保存对设置的更改？」。  
> 架构基线：[`NATIVE_UI_ARCHITECTURE.md`](./NATIVE_UI_ARCHITECTURE.md)（默认 `ui-native`）  
> 更新日期：2026-08-28  
> 状态：**Phase 0–3 已全部实施**

---

## 1. 结论（先看这个）

| 项 | 决议 |
|----|------|
| 交互模型 | **即改即生效 + 防抖写盘**（Instant Apply + Debounced Persist） |
| 设置窗角色 | 纯偏好编辑器，关闭即关，无草稿/无保存按钮 |
| 脏检查 | **废除** TOML 指纹 `draft vs baseline` 作为关闭确认依据 |
| UI 方向 | 去掉粘性底栏保存/取消；二期侧栏导航对齐 macOS 系统设置 |
| 实施范围 | **Phase 1–2 聚焦 `ui_native`**；egui 路径 Phase 3 对齐或标为次要 |
| 预估工期 | **Phase 1 核心 3–4 天**；Phase 2 UI 2–3 天；Phase 3 增强 2 天（可选） |

---

## 2. 背景与问题

### 2.1 用户期望

- 设置里改一项，应用里立刻看到效果（与面板拖拽、按钮编辑器一致）。
- 关闭设置窗时不应被询问「是否保存」——尤其**什么都没改**时。

### 2.2 现状（native 设置窗）

```
打开 → reload_draft（draft = baseline = core.app_config 快照）
     → 用户编辑 / 或仅浏览
     → 150ms 预览循环 collect_draft → sync 归一化 draft → PreviewSettings（内存生效，不写盘）
     → 关闭 → is_dirty()（TOML 指纹 draft ≠ baseline）→ 三按钮 Alert
```

**假阳性根因**（用户未操作也会 dirty）：

| 编号 | 机制 | 关键位置 |
|------|------|----------|
| A | 预览循环 `sync_all_into_draft()` 改写 `draft` | `settings.rs` ~644–665 |
| B | `Option` 字段归一化为 `Some(默认值)` | `sync_visual_into_draft` ~1197–1222 |
| C | 颜色 wells：`None` → `Some("transparent")` / rgba 重排 | `store_theme_from_wells` ~1925–1942 |
| D | `letter_font = ""` ↔ `None` 不一致 | 同上 |
| E | 配置档按钮槽位补齐（如 2×4 仅 4 个 button） | `collect_draft_validated` ~1314–1330 |
| F | 切 Tab 时 `sync_all_into_draft()` | ~546–548 |

### 2.3 与产品其他模块的不一致

| 模块 | 持久化行为 |
|------|------------|
| 面板拖拽位置 | 300ms debounce 立即写盘 |
| 按钮编辑器 | 保存即 `save_app_config` |
| 折叠模式切换（启动器） | 立即写盘 |
| **设置窗** | 须点「保存」才写盘；取消则回滚 preview |

---

## 3. 目标架构

### 3.1 原则

1. **单一真相源**：运行时 `AppCore.app_config` + 磁盘 `config.toml`；废除 session 级 `draft/baseline` 双缓冲。
2. **变更即生效**：控件变更 → 校验 → `apply_config` → UI/热键/面板同步。
3. **变更即落盘**：同一路径上 **400ms 防抖** `save_app_config`（与面板位置写盘策略一致，可复用 `debounce_duration()` 或独立常量）。
4. **关闭无确认**：× / Esc / Cmd+W 直接 `hide()`。
5. **危险操作单独确认**：删除配置档、导入覆盖、恢复备份、缩格删非空按钮槽。

### 3.2 目标数据流

```text
控件 action / 失焦 / 录制完成
    │
    ▼
collect_from_ui() → AppConfig 片段或全量
    │
    ▼
validate（失败 → inline 提示 + 控件回滚，不写盘不 apply）
    │
    ▼
merge → core.app_config
    │
    ├─► core.apply_config(cfg)     // 立即生效（面板/折叠/上下文等）
    ├─► hotkeys.replace(...)       // 热键变更时
    └─► schedule_persist(deadline) // debounce 400ms
              │
              ▼
         save_app_config + backup
              │
              └─► （可选）轻量「已保存」反馈
```

### 3.3 废止的概念与事件

| 废止 | 替代 |
|------|------|
| `SettingsWindow.draft` / `baseline` | 控件绑定 `core.app_config`；打开时 `reload_from_config` 刷新控件 |
| `is_dirty()` / `config_fingerprint` 关闭逻辑 | 无关闭确认 |
| `NativeUiEvent::SaveSettings` | `NativeUiEvent::ApplySettings` + 内部 debounce persist |
| `NativeUiEvent::CancelSettings` | 直接 `hide()` |
| `NativeUiEvent::PreviewSettings`（半套 apply） | 统一走完整 `ApplySettings` |
| Footer「保存」「取消」 | 删除 |
| 标题「— 未保存」 | 固定「MeoTool 设置」 |
| 预览循环 strip 热键/上下文/匹配规则 | 全量 apply（热键即时重注册） |

### 3.4 控件提交策略

| 控件类型 | 提交时机 | 防抖 |
|----------|----------|------|
| Switch / Popup / Slider / ColorWell | `action` 回调 | apply 立即；persist 400ms |
| 数字步进（行/列/透明度等） | 变更时 | 同上 |
| 文本框（Bundle ID、正则、字体路径） | **失焦** 或 Enter | 校验后 apply + persist |
| 快捷键录制 | 录制成功 | 立即 apply + persist + 重注册热键 |
| 配置档新建/删除 | 操作完成 | 立即 persist；删除前 confirm |
| 行列缩小导致 truncate 按钮 | 若有非空槽将被删 | confirm 后 apply |

### 3.5 外部配置变更（watcher）

已有 `poll_config` 从磁盘 reload。设置窗打开时：

- 若文件 mtime 变化且非本进程刚写入 → `reload_from_config` 刷新控件
- 可选：非模态轻提示「配置已从文件重新加载」（避免与 debounce 写盘互相触发：写盘时标记 `last_self_write` 时间戳，watcher 忽略 500ms 内回弹）

---

## 4. UI 改造规格

### 4.1 Phase 1（行为，最小 UI 变动）

- 删除 Footer 右侧「取消」「保存」按钮及 Cmd+S 保存快捷键。
- 关闭拦截改为：`windowShouldClose` → 直接 `hide()`（或 `CloseSettings` 事件无分支）。
- 标题栏固定文案，去掉 dirty 后缀。
- Footer 左侧保留：**打开配置文件… | 导出… | 导入…**（配置 IO 仍需要入口）。
- 删除导入前「设置中有未保存的更改」类文案（无未保存概念）。

### 4.2 Phase 2（Chrome 优化，推荐）

**布局：顶 Tab → 左侧栏 + 右侧内容**（对齐 macOS 系统设置）

```text
┌──────────────┬──────────────────────────────────┐
│  通用        │  [分区标题]                       │
│  外观        │  表单控件…                        │
│  快捷键      │                                   │
│  配置档      │                                   │
│  权限        │                                   │
│──────────────│                                   │
│  打开配置…   │                                   │
│  导出 / 导入 │                                   │
│  查看备份…   │                                   │
└──────────────┴──────────────────────────────────┘
  ~168px 侧栏          滚动内容区
```

| 项 | 规格 |
|----|------|
| 默认尺寸 | 840 × 680（侧栏版）；或 Phase 1 维持 780 × 672（去掉 FOOTER_H 后高度可减 52） |
| 侧栏宽 | 168px，选中项高亮 + 系统蓝 accent |
| 手势 | 并入「通用」页「手势」分区，导航项由 6 减为 5 |
| Footer | **整条删除**；IO 入口移至侧栏底部 |
| 外观页 | 保留页内实时预览 + 设置窗 Normal 级别（便于看悬浮面板） |
| 反馈 | 写盘成功后右下角 1s fade「已保存」（`NSTextField` 或 status label，非模态） |

### 4.3 Phase 3（体验增强，可选）

- 会话级 **Cmd+Z** undo 栈（内存，深度 20）。
- 按钮编辑器：「保存」改「完成」，行为与设置窗一致（编辑过程已 persist）。
- egui `SettingsEditor` 对齐同一模型。

---

## 5. 模块改动清单

### 5.1 新增

| 文件 | 职责 |
|------|------|
| `src/ui_native/settings_persist.rs`（或 `runtime` 内私有模块） | `PendingSettingsSave`：debounce、`flush`、与 watcher 去重 |
| `src/config/normalize.rs`（可选） | 加载后统一 normalize（槽位补齐、Option 默认值），避免 UI 与磁盘语义漂移 |

### 5.2 主要修改

| 文件 | 改动要点 |
|------|----------|
| `src/ui_native/settings.rs` | 去 draft/baseline/dirty/preview strip；控件变更 emit `ApplySettings`；关闭直接 hide；Phase 2 侧栏布局 |
| `src/ui_native/runtime.rs` | 处理 `ApplySettings`；`flush_pending_settings_save`；打开/关闭 settings 生命周期 |
| `src/ui_native/events.rs` | `SaveSettings`/`CancelSettings`/`PreviewSettings` → `ApplySettings(AppConfig)` + `CloseSettings` |
| `src/ui_native/layout.rs` | Phase 2：`install_pref_sidebar_chrome`、`SIDEBAR_W`、无 footer 布局 |
| `src/ui_native/button_editor.rs` | 文案「完成」；可选与 settings 共用 persist helper |
| `src/config/store.rs` | 加载后 normalize（若采用集中 normalize） |
| `src/ui/settings.rs` + `src/egui_app.rs` | Phase 3 对齐 |

### 5.3 可删除的代码（Phase 1 完成后）

- `config_fingerprint`、`is_dirty`、`handle_close_attempt` 三按钮 Alert
- `try_save` / `do_cancel` 中 baseline 回滚逻辑
- 预览循环中 hotkeys/context/match_rule 的 baseline 还原块（~651–657）
- Footer save/cancel 按钮与 `saveClicked` / `cancelClicked`

---

## 6. 分阶段开发计划

### 阶段总览

| 阶段 | 内容 | 依赖 | 预估 | 优先级 | 状态 |
|------|------|------|------|--------|------|
| **0** | 加载时 normalize + 测试基线 | 无 | 0.5 天 | P0 | **已完成** |
| **1** | 即改即生效 + debounce 写盘 + 去掉保存流 | 0 | 3–4 天 | P0 | **已完成** |
| **2** | UI Chrome（无 footer、侧栏导航） | 1 | 2–3 天 | P1 | **已完成** |
| **3** | Undo、egui 对齐、按钮编辑器统一 | 1 | 2 天 | P2 | **已完成** |

**建议迭代顺序**：0 → 1（可发布）→ 2 → 3。

---

### 阶段 0 — 配置 normalize 基线（0.5 天）

**目的**：磁盘配置与内存语义一致，避免「打开即 diff」遗留问题。

#### 任务

| ID | 任务 | 产出 |
|----|------|------|
| 0.1 | 实现 `normalize_app_config(&mut AppConfig)` | 槽位补齐/truncate；主题 `None`/`""` 统一；行列 clamp |
| 0.2 | 在 `load_app_config` / `apply_config` 入口调用 normalize | 启动、导入、reload 路径一致 |
| 0.3 | 单测：`configs/config.toml` 样例 round-trip 指纹稳定 | `config` 模块测试 |

#### 验收

- [ ] 打开设置 → 不操作 → 关闭：**无任何 Alert**（在 Phase 1 接入后回归）
- [ ] normalize 前后功能等价（按钮数、颜色显示不变）

---

### 阶段 1 — 核心行为改造（3–4 天）【必做】

#### 1.1 持久化调度（Day 1）

| ID | 任务 | 文件 |
|----|------|------|
| 1.1.1 | `PendingSettingsSave { deadline: Instant }` | `runtime.rs` 或 `settings_persist.rs` |
| 1.1.2 | `schedule_settings_save()` / `flush_pending_settings_save(force)` | 同上 |
| 1.1.3 | `tick()` 末尾调用 flush（与 `flush_pending_position_save` 并列） | `runtime.rs` |
| 1.1.4 | 写盘失败：`log::error` + 可选 status「保存失败」 | `runtime.rs` |
| 1.1.5 | `last_self_write_at` 供 watcher 忽略回弹 | `runtime.rs` + `watcher` 回调 |

#### 1.2 事件与 apply 路径（Day 1–2）

| ID | 任务 | 说明 |
|----|------|------|
| 1.2.1 | 新增 `NativeUiEvent::ApplySettings(AppConfig)` | `events.rs` |
| 1.2.2 | `poll_ui_events` 处理：validate 已在 settings 侧完成 → `apply_config` → `hotkeys.replace` → `schedule_settings_save` | `runtime.rs` |
| 1.2.3 | 删除 `SaveSettings` / `CancelSettings` / `PreviewSettings` 分支 | `runtime.rs` |
| 1.2.4 | 导入/备份恢复后：直接 `reload_from_config`，无 dirty 提示 | `settings.rs` |

#### 1.3 设置窗逻辑重构（Day 2–3）

| ID | 任务 | 说明 |
|----|------|------|
| 1.3.1 | `reload_draft` → `reload_from_config(&AppConfig)`：只刷新控件，不维护 baseline | `settings.rs` |
| 1.3.2 | 删除 `draft`/`baseline` 字段；`sync_*_into_draft` → `collect_*_patch` 或直接 `collect_app_config_from_ui()` | `settings.rs` |
| 1.3.3 | 删除 150ms 预览循环；改为控件 `SettingsAction::*` 时 `try_apply()` | `settings.rs` |
| 1.3.4 | `try_apply()`：`collect_app_config_from_ui` → validate → emit `ApplySettings` | `settings.rs` |
| 1.3.5 | 文本框：增加失焦 target / `controlTextDidEndEditing` 触发 apply | `settings.rs` + `layout.rs` |
| 1.3.6 | 热键录制结束：`try_apply` 全量（含 hotkeys） | `settings.rs` |
| 1.3.7 | `handle_close_attempt` → `hide()`；`windowShouldClose` 保持 emit 或直接 hide | `settings.rs` |
| 1.3.8 | 删除 Footer「保存」「取消」；移除 Cmd+S | `settings.rs` |
| 1.3.9 | 行列缩小：检测将丢失的非空 button → confirm → apply | `settings.rs` |

#### 1.4 联调与回归（Day 4）

| ID | 场景 | 预期 |
|----|------|------|
| 1.4.1 | 打开即关 | 无 Alert |
| 1.4.2 | 改透明度/颜色 | 面板即时变；400ms 内写盘；重启保留 |
| 1.4.3 | 改全局热键 | 新热键立刻可用；config.toml 已更新 |
| 1.4.4 | 改上下文模式 / 匹配规则 | 切换应用后 profile 正确 |
| 1.4.5 | 改配置档行列 | 槽位正确；缩小有内容时 confirm |
| 1.4.6 | 导出 | 导出当前 `core.app_config`（与磁盘一致） |
| 1.4.7 | 导入 | 无「未保存更改」文案；导入后控件刷新 |
| 1.4.8 | 按钮编辑器保存 | 设置窗若打开，列表自动刷新 |
| 1.4.9 | 外部改 config.toml | watcher reload；设置窗控件同步 |
| 1.4.10 | 快速连续改 slider | 仅一次写盘（debounce）；最终值正确 |

#### 阶段 1 交付标准

- [ ] 关闭设置无任何保存确认（含误报场景）
- [ ] 所有设置项分类（见 §7）变更后 **≤500ms** 内持久化
- [ ] 热键变更立即重注册
- [ ] `cargo build --release` 通过；无新增 clippy 警告（项目惯例下）

---

### 阶段 2 — UI Chrome 优化（2–3 天）【推荐】

#### 2.1 布局基础设施（Day 1）

| ID | 任务 | 文件 |
|----|------|------|
| 2.1.1 | `install_pref_sidebar_chrome`：侧栏 + scroll + 无 footer | `layout.rs` |
| 2.1.2 | `layout_pref_sidebar_chrome` resize 处理 | `layout.rs` |
| 2.1.3 | 侧栏 nav 按钮样式（选中态、hover） | `layout.rs` |
| 2.1.4 | 迁移 `SettingsWindow` 从顶 Tab 到侧栏 | `settings.rs` |
| 2.1.5 | 删除 `FOOTER_H` 相关布局；窗口高度减小或内容区增大 | `layout.rs`, `settings.rs` |

#### 2.2 信息架构（Day 2）

| ID | 任务 |
|----|------|
| 2.2.1 | 手势分区从独立 Tab 并入「通用」 |
| 2.2.2 | 侧栏底部：打开配置 / 导出 / 导入 / 备份 |
| 2.2.3 | 「通用」页去掉重复的配置管理大段（仅保留说明链接到侧栏底） |
| 2.2.4 | `NAV_TITLES` 更新为 5 项 |

#### 2.3 反馈与 polish（Day 3）

| ID | 任务 |
|----|------|
| 2.3.1 | 「已保存」toast（写盘成功后 1s） |
| 2.3.2 | 校验错误 inline（字段旁红色 help_label） |
| 2.3.3 | 外观页预览区视觉强化（可选浅边框） |

#### 阶段 2 交付标准

- [ ] 无底部保存栏；侧栏导航可用
- [ ] 窗口 resize 无布局崩溃（沿用 frame 布局惯例）
- [ ] 5 个主导航项与内容一一对应

---

### 阶段 3 — 体验增强与双 UI 对齐（2 天，可选）

| ID | 任务 |
|----|------|
| 3.1 | `SettingsUndoStack`：Apply 前 push 快照；Cmd+Z pop + apply + persist |
| 3.2 | 按钮编辑器：「保存」→「完成」；关闭不 revert |
| 3.3 | egui `SettingsEditor`：去掉「保存并关闭」；LiveApplied + debounce 写盘 |
| 3.4 | 更新 `DESIGN.md` §6 UI 设计 / §7 交互设计中设置窗描述 |

---

## 7. 设置项覆盖矩阵（Phase 1 必须全部 instant apply）

| 分类 | 字段/能力 | apply | persist | 备注 |
|------|-----------|-------|---------|------|
| 面板 | rows, cols, display_mode, collapsed, collapsed_icon | ✓ | debounce | |
| 面板 | opacity, gap, position x/y | ✓ | debounce | 与拖拽写盘统一 |
| 面板 | poll_interval_ms, context_mode | ✓ | debounce | |
| 外观 | 全局 theme 全字段 | ✓ | debounce | ColorWell 即时 |
| 外观 | 配置档 theme 覆盖 | ✓ | debounce | |
| 手势 | enabled, global_priority, threshold, dismiss, hide_after | ✓ | debounce | |
| 快捷键 | 4 个全局热键 | ✓ | **立即** | 须 `hotkeys.replace` |
| 配置档 | name, match_rule, panel 覆盖 | ✓ | 文本失焦 / debounce | 正则校验 |
| 配置档 | 按钮列表 | 编辑器内 persist | — | 设置窗只读列表 |
| 权限 | 打开系统设置 / 刷新 | — | — | 无 config |
| IO | 导出/导入/备份 | 独立流程 | 导入后立即写盘 | 无 dirty 概念 |

---

## 8. API 设计草案

### 8.1 事件（`events.rs`）

```rust
/// 设置变更已通过校验，请求运行时 apply 并调度写盘
ApplySettings(AppConfig),

/// 关闭设置窗（无回滚）
CloseSettings,
```

### 8.2 Runtime 片段（`runtime.rs`）

```rust
struct NativeState {
    // ...
    pending_settings_save: Option<Instant>,
    last_settings_self_write: Option<Instant>,
}

const SETTINGS_SAVE_DEBOUNCE: Duration = Duration::from_millis(400);

fn on_apply_settings(&mut self, cfg: AppConfig) {
    let effect = self.core.apply_config(cfg);
    if let Err(e) = self.hotkeys.replace(&self.core.app_config.hotkeys) {
        log::warn!("re-register hotkeys: {e}");
    }
    self.apply(effect);
    self.pending_settings_save = Some(Instant::now() + SETTINGS_SAVE_DEBOUNCE);
}

fn flush_pending_settings_save(&mut self, force: bool) {
    let Some(deadline) = self.pending_settings_save else { return };
    if !force && Instant::now() < deadline { return }
    self.pending_settings_save = None;
    match save_app_config(&self.core.config_path, &self.core.app_config) {
        Ok(()) => {
            self.last_settings_self_write = Some(Instant::now());
            self.settings.notify_saved(); // Phase 2 toast
        }
        Err(err) => log::error!("save settings failed: {err}"),
    }
}
```

### 8.3 设置窗（`settings.rs`）

```rust
fn try_apply(&mut self) {
    match self.collect_app_config_from_ui() {
        Ok(cfg) => emit(NativeUiEvent::ApplySettings(cfg)),
        Err(msg) => self.show_inline_error(&msg),
    }
}

fn handle_close(&mut self) {
    emit(NativeUiEvent::CloseSettings);
    self.hide();
}
```

---

## 9. 测试计划

### 9.1 自动化

| 类型 | 内容 |
|------|------|
| 单元测试 | `normalize_app_config` round-trip；`collect_app_config_from_ui` 校验失败路径 |
| 集成 | 已有 `config/import_export` 测试保持绿 |

### 9.2 手工测试清单（Phase 1 发布前）

复制为 PR Checklist：

```markdown
- [ ] 打开设置 → 关闭：无对话框
- [ ] 切换 6/5 个 Tab 后关闭：无对话框
- [ ] 修改外观色：面板即时更新
- [ ] 修改热键：新组合生效，旧组合失效
- [ ] 修改 bundle id 匹配：切换应用后 profile 切换
- [ ] 配置档删行/列（无按钮）：无 confirm，槽位变化
- [ ] 配置档删行/列（有按钮）：confirm 后出现
- [ ] 导出 TOML：内容与当前行为一致
- [ ] 导入：覆盖后 UI 刷新，无「未保存」文案
- [ ] 恢复备份：同上
- [ ] 按钮编辑器保存：设置列表更新
- [ ] 设置开着时外部编辑 config：reload 或忽略自写
- [ ] Quit 前 flush：未 debounce 的变更已写盘
```

### 9.3 性能

- debounce 期间连续拖动 slider：写盘次数 ≤ 1 + 停止后 1 次
- apply_config 不应每 keystroke 触发（文本失焦策略）

---

## 10. 风险与缓解

| 风险 | 影响 | 缓解 |
|------|------|------|
| 热键录制中途误触 apply | 注册无效热键 | 录制态禁止 apply；仅 `FinishRecord` 提交 |
| 正则校验失败 | 半写入状态 | validate 失败不 emit Apply；保留旧 config |
| watcher 与 debounce 写盘打架 | 双 reload UI 闪动 | `last_self_write` 忽略窗口 |
| 缩格误删按钮 | 数据丢失 | confirm + backups 可恢复 |
| Phase 2 侧栏重构引入 layout bug | 崩溃 | 沿用 frame 布局；resize 单测 + 手工拖窗 |
| egui 与 native 行为不一致 | 混淆 | Phase 3 对齐或文档标明 egui 为 legacy |

---

## 11. 里程碑与排期（建议）

假设 1 名熟悉代码库的开发者：

| 里程碑 | 日期（相对） | 产出 |
|--------|--------------|------|
| M0 | D+0.5 | normalize + 测试 |
| M1 | D+4 | Phase 1 可合并主干；关闭无误报；全项 instant apply |
| M2 | D+7 | Phase 2 侧栏 UI 可演示 |
| M3 | D+9 | Phase 3 可选增强 |

**首个可发布版本**：完成 **M1** 即可发布（行为正确优先于 UI 翻新）。

---

## 12. PR 拆分建议

便于 review，建议 4 个 PR：

| PR | 标题（中文） | 范围 |
|----|--------------|------|
| PR-1 | 新增配置 normalize 并在加载路径调用 | 阶段 0 |
| PR-2 | 设置变更即时 apply 与防抖写盘 | runtime + events + persist |
| PR-3 | 移除设置窗保存/取消与脏检查逻辑 | settings.rs 主体 |
| PR-4 | 设置窗侧栏布局与 footer 移除 | layout + settings UI |

PR-2 与 PR-3 可合并为一个大 PR 若人力有限。

---

## 13. 文档同步

| 文档 | 更新内容 |
|------|----------|
| `docs/DESIGN.md` | §6 设置 UI、§7 设置交互改为 instant apply |
| `docs/NATIVE_UI_ARCHITECTURE.md` | bridge 事件列表去掉 Preview/Save/Cancel |
| 本文件 | 各阶段完成后更新「状态」列 |

---

## 14. 附录：现状关键代码索引

| Concern | 位置 |
|---------|------|
| 关闭拦截 + dirty Alert | `src/ui_native/settings.rs:59–64, 715–737` |
| `is_dirty` / 标题 | `settings.rs:796–806` |
| 预览循环 | `settings.rs:644–665` |
| Save/Cancel/Preview 处理 | `runtime.rs:541–558` |
| Footer 按钮 | `settings.rs:407–450` |
| 面板位置 debounce 参考 | `runtime.rs:102–103, flush_pending_position_save` |
| egui 对照（LiveApplied） | `src/ui/settings.rs:207–218` |

---

**下一步行动**：按 §6 阶段 0 → 1 创建分支 `feat/settings-instant-apply`，先落地 PR-1（normalize）与 PR-2/3（行为改造）。
