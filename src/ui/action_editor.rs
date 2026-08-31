use eframe::egui::{self, Context};

use crate::action::{
    action_attached_macro, action_needs_legacy_upgrade, action_to_meoflow, compile,
    macro_to_meoflow, step_preview_label,
};
use crate::config::script_presets::{
    is_comments_only, preset_by_id, render_template, PresetBuildParams, PresetParamKind,
    ScriptPreset, DEFAULT_SCRIPT_SKELETON, FLOW_PRESETS, QUICK_PRESETS,
};
use crate::config::{Action, ActionScript, MouseMacro};

use super::hotkey_recorder::{HotkeyRecorder, RecordingSlot};

const SYNTAX_HELP: &str = include_str!("../../docs/meoflow_cheatsheet.txt");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacroEditorRequest {
    /// 捕获一次点击，插入坐标行
    ScriptCaptureClick,
    /// 点选应用窗口，插入「激活应用 "bundle.id"」
    ScriptCaptureApp,
    /// 点选应用窗口，插入「打开应用 "…"」（快捷预设）
    ScriptPickOpenApp,
    /// 试运行当前脚本源码
    TryRunScript,
    /// 开始连续录制（点击序列）
    StartMacroRecord,
    /// 结束录制并追加脚本
    FinishMacroRecord,
    CancelRecord,
}

/// 流程模板多步 wizard 状态（egui）。
#[derive(Debug, Clone)]
enum PendingFlow {
    ClickHotkey {
        params: PresetBuildParams,
        replace_all: bool,
    },
    OpenAppType {
        params: PresetBuildParams,
        replace_all: bool,
    },
}

#[derive(Debug, Clone)]
enum PresetWizard {
    Text {
        title: String,
        multiline: bool,
        buf: String,
        preset: &'static ScriptPreset,
        replace_all: bool,
    },
    Url {
        buf: String,
        preset: &'static ScriptPreset,
        replace_all: bool,
    },
    Shell {
        buf: String,
        preset: &'static ScriptPreset,
        replace_all: bool,
    },
    Path {
        buf: String,
        preset: &'static ScriptPreset,
        replace_all: bool,
    },
    App {
        buf: String,
        preset: &'static ScriptPreset,
        replace_all: bool,
    },
    FlowConflict {
        preset: &'static ScriptPreset,
    },
    OpenAppTypeText {
        params: PresetBuildParams,
        buf: String,
        replace_all: bool,
    },
    ClickHotkeyHotkey {
        params: PresetBuildParams,
        replace_all: bool,
    },
}

#[derive(Debug, Clone)]
pub struct ActionEditorState {
    pub script_source: String,
    pub script_attached_macro: Option<MouseMacro>,
    pub script_status: String,
    pub script_preview: String,
    pub script_compile_ok: bool,
    pub legacy_upgrade_hint: bool,
    pending_flow: Option<PendingFlow>,
    flow_replace_all: bool,
    wizard: Option<PresetWizard>,
    show_syntax_help: bool,
    pending_hotkey_preset: Option<&'static ScriptPreset>,
    /// 「打开应用」预设点选窗口后写入脚本
    pending_open_app_pick: Option<PendingOpenAppPick>,
}

#[derive(Debug, Clone)]
struct PendingOpenAppPick {
    replace_all: bool,
}

impl Default for ActionEditorState {
    fn default() -> Self {
        Self {
            script_source: DEFAULT_SCRIPT_SKELETON.to_string(),
            script_attached_macro: None,
            script_status: String::new(),
            script_preview: String::new(),
            script_compile_ok: false,
            legacy_upgrade_hint: false,
            pending_flow: None,
            flow_replace_all: false,
            wizard: None,
            show_syntax_help: false,
            pending_hotkey_preset: None,
            pending_open_app_pick: None,
        }
    }
}

impl ActionEditorState {
    pub fn from_action(action: &Action) -> Self {
        let legacy = action_needs_legacy_upgrade(action);
        let source = match action {
            Action::Script(s) => s.source.clone(),
            Action::None => DEFAULT_SCRIPT_SKELETON.to_string(),
            other => action_to_meoflow(other),
        };
        let mut state = Self {
            script_source: source,
            script_attached_macro: action_attached_macro(action),
            legacy_upgrade_hint: legacy,
            ..Default::default()
        };
        state.refresh_script_status();
        state
    }

    /// 将捕获到的窗口相对点击插入脚本源码。
    pub fn insert_script_click(&mut self, x: f32, y: f32) {
        if let Some(PendingFlow::ClickHotkey {
            mut params,
            replace_all,
        }) = self.pending_flow.take()
        {
            params.x = Some(format!("{x:.4}"));
            params.y = Some(format!("{y:.4}"));
            self.wizard = Some(PresetWizard::ClickHotkeyHotkey { params, replace_all });
            return;
        }

        let line = format!("点击 {x:.4} {y:.4}");
        self.insert_script_fragment(&format!("{line}\n"), false);
    }

    /// 将捕获到的 Bundle ID 插入「激活应用」行。
    pub fn insert_script_activate(&mut self, bundle_id: &str) {
        let escaped = bundle_id.replace('\\', "\\\\").replace('"', "\\\"");
        let line = format!("激活应用 \"{escaped}\"");

        if let Some(PendingFlow::OpenAppType {
            mut params,
            replace_all,
        }) = self.pending_flow.take()
        {
            params.app = Some(bundle_id.to_string());
            self.wizard = Some(PresetWizard::OpenAppTypeText {
                params,
                buf: String::new(),
                replace_all,
            });
            return;
        }

        if let Some(wizard) = self.wizard.as_mut() {
            if let PresetWizard::App { buf, .. } = wizard {
                *buf = bundle_id.to_string();
                return;
            }
        }

        self.insert_script_fragment(&format!("{line}\n"), false);
    }

    pub fn take_pending_open_app_pick(&mut self) -> Option<bool> {
        self.pending_open_app_pick.take().map(|p| p.replace_all)
    }

    /// 点选窗口后插入「打开应用」行（非「激活应用」）。
    pub fn insert_script_open_app(
        &mut self,
        bundle_id: Option<String>,
        app_name: &str,
        replace_all: bool,
    ) {
        let target = bundle_id
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                let t = app_name.trim();
                (!t.is_empty()).then(|| t.to_string())
            });
        let Some(app) = target else {
            self.script_status = "状态：未能识别应用，请重试".into();
            return;
        };
        let fragment = render_template(
            "打开应用 \"{app}\"\n",
            &PresetBuildParams {
                app: Some(app),
                ..Default::default()
            },
        );
        self.insert_script_fragment(&fragment, replace_all);
    }

    pub fn append_macro_script(&mut self, macro_: &MouseMacro) {
        let fragment = macro_to_meoflow(macro_);
        self.insert_script_fragment(&format!("{fragment}\n"), false);
        self.script_attached_macro = Some(macro_.clone());
        self.refresh_script_status();
    }

    pub fn refresh_script_status(&mut self) {
        if self.legacy_upgrade_hint {
            self.script_status =
                "状态：已由旧动作类型转换为脚本，保存后将升级格式".into();
        }
        match compile(&self.script_source, self.script_attached_macro.as_ref()) {
            Ok(prog) => {
                let secs = prog.estimated_ms as f64 / 1000.0;
                let status = format!("状态：{} 步 · 约 {secs:.1} 秒 · 无错误", prog.steps.len());
                let preview: String = prog
                    .steps
                    .iter()
                    .enumerate()
                    .take(6)
                    .map(|(i, s)| format!("{}. {}", i + 1, step_preview_label(s)))
                    .collect::<Vec<_>>()
                    .join("  ");
                let preview = if prog.steps.len() > 6 {
                    format!("{preview} …")
                } else if preview.is_empty() {
                    "预览：—".into()
                } else {
                    format!("预览：{preview}")
                };
                if !self.legacy_upgrade_hint {
                    self.script_status = status;
                }
                self.script_preview = preview;
                self.script_compile_ok = true;
            }
            Err(err) => {
                let msg = err.primary_display().replace('\n', " ");
                self.script_status = format!("状态：错误 — {msg}");
                self.script_preview = "预览：修复错误后可保存与试运行".into();
                self.script_compile_ok = false;
            }
        }
    }

    pub fn validate_for_save(&self) -> Result<(), String> {
        if self.script_source.trim().is_empty() || is_comments_only(&self.script_source) {
            return Ok(());
        }
        compile(
            &self.script_source,
            self.script_attached_macro.as_ref(),
        )
        .map(|_| ())
        .map_err(|e| e.primary_display())
    }

    pub fn to_action(&self) -> Action {
        if self.script_source.trim().is_empty() || is_comments_only(&self.script_source) {
            Action::None
        } else {
            let attached = self.script_attached_macro.clone();
            let program = compile(&self.script_source, attached.as_ref()).ok();
            Action::Script(ActionScript {
                source: self.script_source.clone(),
                program,
                attached_macro: attached,
            })
        }
    }

    pub fn apply_to_button(&self, enabled: &mut bool) -> Action {
        let action = self.to_action();
        if action.is_configured() {
            action
        } else {
            *enabled = false;
            Action::None
        }
    }

    fn script_meaningful(&self) -> bool {
        !self.script_source.trim().is_empty() && !is_comments_only(&self.script_source)
    }

    fn insert_script_fragment(&mut self, fragment: &str, replace_all: bool) {
        if replace_all {
            self.script_source = fragment.to_string();
            self.legacy_upgrade_hint = false;
            self.refresh_script_status();
            return;
        }
        if !self.script_source.is_empty() && !self.script_source.ends_with('\n') {
            self.script_source.push('\n');
        }
        self.script_source.push_str(fragment);
        if !fragment.ends_with('\n') {
            self.script_source.push('\n');
        }
        self.legacy_upgrade_hint = false;
        self.refresh_script_status();
    }

    fn run_quick_preset(
        &mut self,
        preset: &'static ScriptPreset,
        request: &mut Option<MacroEditorRequest>,
        recorder: &mut HotkeyRecorder,
        slot: RecordingSlot,
    ) {
        self.run_preset(preset, false, request, recorder, slot);
    }

    fn run_flow_preset(
        &mut self,
        preset: &'static ScriptPreset,
        request: &mut Option<MacroEditorRequest>,
        recorder: &mut HotkeyRecorder,
        slot: RecordingSlot,
    ) {
        let meaningful = self.script_meaningful();
        if meaningful {
            self.wizard = Some(PresetWizard::FlowConflict { preset });
            return;
        }
        self.flow_replace_all = false;
        self.run_flow_preset_inner(preset, false, request, recorder, slot);
    }

    fn run_flow_preset_inner(
        &mut self,
        preset: &'static ScriptPreset,
        replace_all: bool,
        request: &mut Option<MacroEditorRequest>,
        recorder: &mut HotkeyRecorder,
        slot: RecordingSlot,
    ) {
        match preset.id {
            "type_enter" | "paste_enter" => {
                self.wizard = Some(PresetWizard::Text {
                    title: if preset.id == "type_enter" {
                        "输入文字".into()
                    } else {
                        "粘贴文字".into()
                    },
                    multiline: true,
                    buf: String::new(),
                    preset,
                    replace_all,
                });
            }
            "click_hotkey" => {
                self.pending_flow = Some(PendingFlow::ClickHotkey {
                    params: PresetBuildParams::default(),
                    replace_all,
                });
                *request = Some(MacroEditorRequest::ScriptCaptureClick);
            }
            "open_app_type" => {
                self.pending_flow = Some(PendingFlow::OpenAppType {
                    params: PresetBuildParams::default(),
                    replace_all,
                });
                self.wizard = Some(PresetWizard::App {
                    buf: String::new(),
                    preset,
                    replace_all,
                });
            }
            _ => {
                self.run_preset(preset, replace_all, request, recorder, slot);
            }
        }
    }

    fn run_preset(
        &mut self,
        preset: &'static ScriptPreset,
        replace_all: bool,
        request: &mut Option<MacroEditorRequest>,
        recorder: &mut HotkeyRecorder,
        slot: RecordingSlot,
    ) {
        match preset.param {
            PresetParamKind::HotkeyCapture => {
                recorder.start(slot);
                self.pending_hotkey_preset = Some(preset);
            }
            PresetParamKind::OpenAppPanel => {
                self.wizard = Some(PresetWizard::App {
                    buf: String::new(),
                    preset,
                    replace_all,
                });
            }
            PresetParamKind::Url => {
                self.wizard = Some(PresetWizard::Url {
                    buf: String::new(),
                    preset,
                    replace_all,
                });
            }
            PresetParamKind::OpenFilePanel => {
                self.wizard = Some(PresetWizard::Path {
                    buf: String::new(),
                    preset,
                    replace_all,
                });
            }
            PresetParamKind::Text { multiline } => {
                self.wizard = Some(PresetWizard::Text {
                    title: preset.label.to_string(),
                    multiline,
                    buf: String::new(),
                    preset,
                    replace_all,
                });
            }
            PresetParamKind::ShellCommand => {
                self.wizard = Some(PresetWizard::Shell {
                    buf: String::new(),
                    preset,
                    replace_all,
                });
            }
            PresetParamKind::ClickCapture => {
                *request = Some(MacroEditorRequest::ScriptCaptureClick);
            }
            PresetParamKind::MacroRecord => {
                *request = Some(MacroEditorRequest::StartMacroRecord);
            }
            PresetParamKind::FlowWizard => {}
        }
    }

    fn show_wizards(
        &mut self,
        ctx: &Context,
        id_prefix: &str,
        request: &mut Option<MacroEditorRequest>,
        recorder: &mut HotkeyRecorder,
        slot: RecordingSlot,
    ) {
        let mut close_wizard = false;
        let mut insert_from_wizard: Option<(String, bool)> = None;
        let mut flow_choice: Option<(bool, &'static ScriptPreset)> = None;
        let mut start_app_pick = false;
        let mut pick_open_app = false;

        if let Some(wizard) = self.wizard.clone() {
            let open = self.wizard.is_some();
            if open {
                let title = match &wizard {
                    PresetWizard::FlowConflict { .. } => "插入流程模板",
                    PresetWizard::ClickHotkeyHotkey { .. } => "录制快捷键",
                    PresetWizard::OpenAppTypeText { .. } => "输入文字",
                    PresetWizard::Text { title, .. } => title.as_str(),
                    PresetWizard::Url { .. } => "打开链接",
                    PresetWizard::Shell { .. } => "执行命令",
                    PresetWizard::Path { .. } => "打开文件",
                    PresetWizard::App { .. } => "打开应用",
                };
                egui::Window::new(title)
                    .id(egui::Id::new(format!("{id_prefix}_preset_wizard")))
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        match wizard {
                            PresetWizard::FlowConflict { preset } => {
                                ui.label("当前脚本已有内容，请选择：");
                                ui.horizontal(|ui| {
                                    if ui.button("追加到末尾").clicked() {
                                        flow_choice = Some((false, preset));
                                        close_wizard = true;
                                    }
                                    if ui.button("替换全部").clicked() {
                                        flow_choice = Some((true, preset));
                                        close_wizard = true;
                                    }
                                    if ui.button("取消").clicked() {
                                        close_wizard = true;
                                    }
                                });
                            }
                            PresetWizard::Text {
                                title,
                                multiline,
                                mut buf,
                                preset,
                                replace_all,
                            } => {
                                ui.label(&title);
                                if multiline {
                                    ui.add(
                                        egui::TextEdit::multiline(&mut buf)
                                            .desired_rows(4)
                                            .desired_width(280.0),
                                    );
                                } else {
                                    ui.add(egui::TextEdit::singleline(&mut buf).desired_width(280.0));
                                }
                                ui.horizontal(|ui| {
                                    if ui.button("确定").clicked() && !buf.trim().is_empty() {
                                        let fragment = render_template(
                                            preset.template,
                                            &PresetBuildParams {
                                                text: Some(buf.clone()),
                                                ..Default::default()
                                            },
                                        );
                                        insert_from_wizard = Some((fragment, replace_all));
                                        close_wizard = true;
                                    }
                                    if ui.button("取消").clicked() {
                                        close_wizard = true;
                                    }
                                });
                                if let Some(PresetWizard::Text { buf: ref mut b, .. }) =
                                    self.wizard.as_mut()
                                {
                                    *b = buf;
                                }
                            }
                            PresetWizard::Url {
                                mut buf,
                                preset,
                                replace_all,
                            } => {
                                ui.add(
                                    egui::TextEdit::singleline(&mut buf)
                                        .desired_width(280.0)
                                        .hint_text("https://…"),
                                );
                                ui.horizontal(|ui| {
                                    if ui.button("确定").clicked() && !buf.trim().is_empty() {
                                        let fragment = render_template(
                                            preset.template,
                                            &PresetBuildParams {
                                                url: Some(buf.trim().to_string()),
                                                ..Default::default()
                                            },
                                        );
                                        insert_from_wizard = Some((fragment, replace_all));
                                        close_wizard = true;
                                    }
                                    if ui.button("取消").clicked() {
                                        close_wizard = true;
                                    }
                                });
                                if let Some(PresetWizard::Url { buf: ref mut b, .. }) =
                                    self.wizard.as_mut()
                                {
                                    *b = buf;
                                }
                            }
                            PresetWizard::Shell {
                                mut buf,
                                preset,
                                replace_all,
                            } => {
                                ui.add(
                                    egui::TextEdit::multiline(&mut buf)
                                        .desired_rows(3)
                                        .desired_width(280.0)
                                        .hint_text("open ~/Desktop"),
                                );
                                ui.horizontal(|ui| {
                                    if ui.button("确定").clicked() && !buf.trim().is_empty() {
                                        let fragment = render_template(
                                            preset.template,
                                            &PresetBuildParams {
                                                text: Some(buf.clone()),
                                                ..Default::default()
                                            },
                                        );
                                        insert_from_wizard = Some((fragment, replace_all));
                                        close_wizard = true;
                                    }
                                    if ui.button("取消").clicked() {
                                        close_wizard = true;
                                    }
                                });
                                if let Some(PresetWizard::Shell { buf: ref mut b, .. }) =
                                    self.wizard.as_mut()
                                {
                                    *b = buf;
                                }
                            }
                            PresetWizard::Path {
                                mut buf,
                                preset,
                                replace_all,
                            } => {
                                ui.label("路径（支持 ~ 开头）");
                                ui.add(
                                    egui::TextEdit::singleline(&mut buf)
                                        .desired_width(280.0)
                                        .hint_text("~/Documents/a.pdf"),
                                );
                                ui.horizontal(|ui| {
                                    if ui.button("确定").clicked() && !buf.trim().is_empty() {
                                        let fragment = render_template(
                                            preset.template,
                                            &PresetBuildParams {
                                                path: Some(buf.trim().to_string()),
                                                ..Default::default()
                                            },
                                        );
                                        insert_from_wizard = Some((fragment, replace_all));
                                        close_wizard = true;
                                    }
                                    if ui.button("取消").clicked() {
                                        close_wizard = true;
                                    }
                                });
                                if let Some(PresetWizard::Path { buf: ref mut b, .. }) =
                                    self.wizard.as_mut()
                                {
                                    *b = buf;
                                }
                            }
                            PresetWizard::App {
                                mut buf,
                                preset,
                                replace_all,
                            } => {
                                ui.label("应用名、.app 路径或 Bundle ID");
                                ui.add(
                                    egui::TextEdit::singleline(&mut buf)
                                        .desired_width(280.0)
                                        .hint_text("Notes"),
                                );
                                ui.horizontal(|ui| {
                                    if ui.button("确定").clicked() && !buf.trim().is_empty() {
                                        let fragment = render_template(
                                            preset.template,
                                            &PresetBuildParams {
                                                app: Some(buf.trim().to_string()),
                                                ..Default::default()
                                            },
                                        );
                                        insert_from_wizard = Some((fragment, replace_all));
                                        close_wizard = true;
                                    }
                                    if ui.button("点选窗口").clicked() {
                                        if preset.id == "open_app" {
                                            self.pending_open_app_pick = Some(PendingOpenAppPick {
                                                replace_all,
                                            });
                                            pick_open_app = true;
                                        } else {
                                            start_app_pick = true;
                                        }
                                        close_wizard = true;
                                    }
                                    if ui.button("取消").clicked() {
                                        close_wizard = true;
                                        self.pending_flow = None;
                                    }
                                });
                                if let Some(PresetWizard::App { buf: ref mut b, .. }) =
                                    self.wizard.as_mut()
                                {
                                    *b = buf;
                                }
                            }
                            PresetWizard::OpenAppTypeText {
                                mut params,
                                mut buf,
                                replace_all,
                            } => {
                                ui.label("应用打开后将输入这段文字");
                                ui.add(
                                    egui::TextEdit::multiline(&mut buf)
                                        .desired_rows(3)
                                        .desired_width(280.0),
                                );
                                ui.horizontal(|ui| {
                                    if ui.button("确定").clicked() && !buf.trim().is_empty() {
                                        params.text = Some(buf.clone());
                                        let preset = preset_by_id("open_app_type").unwrap();
                                        let fragment = render_template(preset.template, &params);
                                        insert_from_wizard = Some((fragment, replace_all));
                                        close_wizard = true;
                                    }
                                    if ui.button("取消").clicked() {
                                        close_wizard = true;
                                    }
                                });
                                if let Some(PresetWizard::OpenAppTypeText {
                                    params: ref mut p,
                                    buf: ref mut b,
                                    ..
                                }) = self.wizard.as_mut()
                                {
                                    *p = params;
                                    *b = buf;
                                }
                            }
                            PresetWizard::ClickHotkeyHotkey {
                                mut params,
                                replace_all,
                            } => {
                                ui.label("点击已捕获，请录制快捷键（点击输入框后按键）");
                                recorder.start(slot);
                                ui.horizontal(|ui| {
                                    if ui.button("取消").clicked() {
                                        close_wizard = true;
                                    }
                                });
                                if let Some(spec) = recorder.poll(ctx, slot) {
                                    params.hotkey = Some(spec);
                                    let preset = preset_by_id("click_hotkey").unwrap();
                                    let fragment = render_template(preset.template, &params);
                                    insert_from_wizard = Some((fragment, replace_all));
                                    close_wizard = true;
                                }
                                if let Some(PresetWizard::ClickHotkeyHotkey {
                                    params: ref mut p,
                                    ..
                                }) = self.wizard.as_mut()
                                {
                                    *p = params;
                                }
                            }
                        }
                    });
            }
        }

        if close_wizard {
            self.wizard = None;
        }
        if let Some((fragment, replace_all)) = insert_from_wizard {
            self.insert_script_fragment(&fragment, replace_all);
        }
        if let Some((replace_all, preset)) = flow_choice {
            self.run_flow_preset_inner(preset, replace_all, request, recorder, slot);
        }
        if pick_open_app {
            *request = Some(MacroEditorRequest::ScriptPickOpenApp);
        } else if start_app_pick {
            *request = Some(MacroEditorRequest::ScriptCaptureApp);
        }

        if self.show_syntax_help {
            let mut open = true;
            egui::Window::new("常用 MeoFlow 命令")
                .id(egui::Id::new(format!("{id_prefix}_syntax_help")))
                .default_width(420.0)
                .open(&mut open)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(360.0)
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(SYNTAX_HELP).family(egui::FontFamily::Monospace),
                            );
                        });
                    if ui.button("关闭").clicked() {
                        self.show_syntax_help = false;
                    }
                });
            if !open {
                self.show_syntax_help = false;
            }
        }

        if let Some(preset) = self.pending_hotkey_preset {
            if let Some(spec) = recorder.poll(ctx, slot) {
                let fragment = render_template(
                    preset.template,
                    &PresetBuildParams {
                        hotkey: Some(spec),
                        ..Default::default()
                    },
                );
                self.insert_script_fragment(&fragment, false);
                self.pending_hotkey_preset = None;
            }
        }
    }
}

pub fn show_action_editor(
    ui: &mut egui::Ui,
    ctx: &Context,
    id_prefix: &str,
    state: &mut ActionEditorState,
    recorder: &mut HotkeyRecorder,
    slot: RecordingSlot,
    record_mode: crate::platform::RecordMode,
    request: &mut Option<MacroEditorRequest>,
) {
    let recording = record_mode != crate::platform::RecordMode::Idle;

    ui.push_id(id_prefix, |ui| {
        ui.label("动作脚本");
        ui.label("一行写一步。例如：输入 \"你好\" → 等待 100 → 按键 Enter");

        if recording {
            ui.colored_label(
                egui::Color32::from_rgb(220, 80, 80),
                match record_mode {
                    crate::platform::RecordMode::CaptureOnce => {
                        "捕获中… 请到目标窗口点击一次（Esc 取消）"
                    }
                    crate::platform::RecordMode::Sequence => {
                        "录制中… 在目标窗口操作，完成后点「完成录制」（Esc 取消）"
                    }
                    crate::platform::RecordMode::Idle => "",
                },
            );
            ui.horizontal(|ui| {
                if record_mode == crate::platform::RecordMode::Sequence
                    && ui.button("完成录制").clicked()
                {
                    *request = Some(MacroEditorRequest::FinishMacroRecord);
                }
                if ui.button("取消").clicked() {
                    *request = Some(MacroEditorRequest::CancelRecord);
                }
            });
        } else {
            let response = ui.add(
                egui::TextEdit::multiline(&mut state.script_source)
                    .desired_rows(8)
                    .desired_width(f32::INFINITY)
                    .code_editor()
                    .hint_text(DEFAULT_SCRIPT_SKELETON),
            );
            if response.changed() {
                state.legacy_upgrade_hint = false;
                state.refresh_script_status();
            }
        }

        if state.script_status.is_empty() {
            state.refresh_script_status();
        }
        ui.label(&state.script_status);
        if !state.script_preview.is_empty() {
            ui.label(&state.script_preview);
        }

        if !recording {
            ui.add_space(4.0);
            ui.label("快捷添加");
            ui.horizontal_wrapped(|ui| {
                for preset in QUICK_PRESETS {
                    if ui
                        .button(preset.label)
                        .on_hover_text(preset.tooltip)
                        .clicked()
                    {
                        state.run_quick_preset(preset, request, recorder, slot);
                    }
                }
            });

            ui.add_space(4.0);
            ui.label("流程模板");
            ui.horizontal_wrapped(|ui| {
                for preset in FLOW_PRESETS {
                    if ui
                        .button(preset.label)
                        .on_hover_text(preset.tooltip)
                        .clicked()
                    {
                        state.run_flow_preset(preset, request, recorder, slot);
                    }
                }
            });

            ui.add_space(4.0);
            ui.label("工具");
            ui.horizontal(|ui| {
                if ui
                    .button("捕获点击")
                    .on_hover_text("点击目标窗口一次，插入：点击 x y")
                    .clicked()
                {
                    *request = Some(MacroEditorRequest::ScriptCaptureClick);
                }
                if ui
                    .button("捕获应用")
                    .on_hover_text("点击目标应用窗口，插入：激活应用 \"bundle.id\"")
                    .clicked()
                {
                    *request = Some(MacroEditorRequest::ScriptCaptureApp);
                }
                if ui
                    .button("试运行")
                    .on_hover_text("请先点到目标窗口")
                    .clicked()
                {
                    state.refresh_script_status();
                    if state.script_compile_ok {
                        *request = Some(MacroEditorRequest::TryRunScript);
                    }
                }
                if ui
                    .button("语法说明")
                    .on_hover_text("打开常用 MeoFlow 命令速查")
                    .clicked()
                {
                    state.show_syntax_help = true;
                }
            });
        }
    });

    state.show_wizards(ctx, id_prefix, request, recorder, slot);
}
