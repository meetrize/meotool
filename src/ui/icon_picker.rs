use std::path::PathBuf;

use eframe::egui::{self, Context, ViewportId};

use super::icons::{list_available_icons, IconCache};
use super::overlay::icon_picker_viewport;

const ICON_PICKER_VIEWPORT: &str = "meotool_icon_picker";
const CELL_SIZE: f32 = 44.0;
const ROW_HEIGHT: f32 = CELL_SIZE + 4.0;

fn grid_cols_for_width(available_width: f32, item_spacing_x: f32) -> usize {
    let cell_stride = CELL_SIZE + item_spacing_x;
    if cell_stride <= 0.0 {
        return 1;
    }
    let cols = ((available_width + item_spacing_x) / cell_stride).floor() as usize;
    cols.max(1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconPickTarget {
    ProfileButton { profile: usize, button: usize },
    CollapsedIcon,
}

#[derive(Debug)]
pub struct IconPicker {
    pub open: bool,
    pub search: String,
    target: Option<IconPickTarget>,
    all_icons: Vec<String>,
    filtered: Vec<usize>,
    icons_dirty: bool,
    centered: bool,
}

impl Default for IconPicker {
    fn default() -> Self {
        Self {
            open: false,
            search: String::new(),
            target: None,
            all_icons: Vec::new(),
            filtered: Vec::new(),
            icons_dirty: true,
            centered: false,
        }
    }
}

impl IconPicker {
    pub fn open_for(&mut self, target: IconPickTarget) {
        self.target = Some(target);
        self.search.clear();
        self.icons_dirty = true;
        self.centered = false;
        self.open = true;
    }

    pub fn close(&mut self, icons: &mut IconCache) {
        if self.open {
            icons.clear_picker_cache();
        }
        self.open = false;
        self.target = None;
        self.centered = false;
    }

    /// 显示图标选择独立窗口；若用户选中图标则返回 `(target, icon_name)`
    pub fn show(
        &mut self,
        ctx: &Context,
        icon_dirs: &[PathBuf],
        icons: &mut IconCache,
    ) -> Option<(IconPickTarget, String)> {
        if !self.open {
            return None;
        }

        if self.icons_dirty {
            self.all_icons = list_available_icons(icon_dirs);
            self.rebuild_filter();
            self.icons_dirty = false;
        }

        let mut selected: Option<(IconPickTarget, String)> = None;
        let mut should_close = false;

        ctx.show_viewport_immediate(
            ViewportId::from_hash_of(ICON_PICKER_VIEWPORT),
            icon_picker_viewport(),
            |ctx, _class| {
                if !self.centered {
                    if let Some(cmd) = egui::ViewportCommand::center_on_screen(ctx) {
                        ctx.send_viewport_cmd(cmd);
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    self.centered = true;
                }

                egui::CentralPanel::default().show(ctx, |ui| {
                    self.draw_body(ui, ctx, icons, &mut selected, &mut should_close);
                    ui.add_space(8.0);
                    if ui.button("关闭").clicked() {
                        should_close = true;
                    }
                });

                if ctx.input(|i| i.viewport().close_requested()) {
                    should_close = true;
                }
            },
        );

        if should_close {
            self.close(icons);
        }

        selected
    }

    fn draw_body(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &Context,
        icons: &mut IconCache,
        selected: &mut Option<(IconPickTarget, String)>,
        should_close: &mut bool,
    ) {
        ui.horizontal(|ui| {
            ui.label("搜索");
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("图标名称，如 copy、search…")
                    .desired_width(f32::INFINITY),
            );
            if response.changed() {
                self.rebuild_filter();
            }
            if ui.button("清除").clicked() {
                self.search.clear();
                self.rebuild_filter();
            }
        });

        let total = self.all_icons.len();
        let shown = self.filtered.len();
        ui.label(format!(
            "共 {total} 个图标{}",
            if self.search.is_empty() {
                String::new()
            } else {
                format!("，匹配 {shown} 个")
            }
        ));

        if self.filtered.is_empty() {
            ui.add_space(8.0);
            ui.label("无匹配图标");
            return;
        }

        let scroll = egui::ScrollArea::vertical()
            .id_salt("icon_picker_scroll")
            .auto_shrink([false, false]);

        let spacing_x = ui.spacing().item_spacing.x;
        let cols = grid_cols_for_width(ui.available_width(), spacing_x);
        let row_count = self.filtered.len().div_ceil(cols);

        scroll.show_rows(ui, ROW_HEIGHT, row_count, |ui, row_range| {
            for row in row_range {
                ui.horizontal(|ui| {
                    for col in 0..cols {
                        let idx = row * cols + col;
                        if idx >= self.filtered.len() {
                            break;
                        }
                        let icon_idx = self.filtered[idx];
                        let icon_name = self.all_icons[icon_idx].clone();
                        self.icon_cell(ui, ctx, icons, &icon_name, selected, should_close);
                    }
                });
            }
        });
    }

    fn icon_cell(
        &self,
        ui: &mut egui::Ui,
        ctx: &Context,
        icons: &mut IconCache,
        icon_name: &str,
        selected: &mut Option<(IconPickTarget, String)>,
        should_close: &mut bool,
    ) {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(CELL_SIZE, CELL_SIZE), egui::Sense::click());

        if ui.is_rect_visible(rect) {
            let visuals = ui.style().interact(&response);
            ui.painter()
                .rect(rect, 4.0, visuals.weak_bg_fill, visuals.bg_stroke);

            if let Some(texture) = icons.get_for_picker(ctx, icon_name) {
                let pad = 6.0;
                let icon_rect = rect.shrink(pad);
                ui.painter().image(
                    texture.id(),
                    icon_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }

            if response.hovered() {
                response.clone().on_hover_text(icon_name);
            }
        }

        if response.clicked() {
            if let Some(target) = self.target {
                *selected = Some((target, icon_name.to_string()));
                *should_close = true;
            }
        }
    }

    fn rebuild_filter(&mut self) {
        let query = self.search.trim().to_ascii_lowercase();
        if query.is_empty() {
            self.filtered = (0..self.all_icons.len()).collect();
            return;
        }

        self.filtered = self
            .all_icons
            .iter()
            .enumerate()
            .filter(|(_, name)| name.to_ascii_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect();
    }
}
