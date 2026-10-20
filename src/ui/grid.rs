use crate::config::{ButtonConfig, PanelConfig};
use crate::ui::button_menu::{ButtonContextMenu, LauncherContextMenu};
use crate::ui::collapsed::{show_collapsed, CollapsedState};
use crate::ui::icons::IconCache;
use crate::ui::modes::paint_button;
use crate::ui::theme::Theme;

pub struct GridView;

impl Default for GridView {
    fn default() -> Self {
        Self
    }
}

pub struct GridResponse {
    pub triggered: Option<usize>,
    pub needs_repaint: bool,
    pub viewport_size: Option<(f32, f32)>,
    pub native_drag_started: bool,
    pub expand_to_full: bool,
}

impl GridView {
    pub fn show(
        &self,
        ui: &mut egui::Ui,
        theme: &Theme,
        panel: &PanelConfig,
        buttons: &[ButtonConfig],
        icons: &mut IconCache,
        flash_index: Option<(usize, egui::Color32)>,
        selected_cell: Option<(u8, u8)>,
        collapsed_state: &mut CollapsedState,
        suppress_collapse: bool,
        menu: &mut ButtonContextMenu,
        launcher_menu: &mut LauncherContextMenu,
    ) -> GridResponse {
        if panel.collapsed {
            let resp = show_collapsed(
                ui,
                theme,
                panel,
                buttons,
                collapsed_state,
                icons,
                flash_index,
                selected_cell,
                suppress_collapse,
                menu,
                launcher_menu,
            );
            return GridResponse {
                triggered: resp.triggered,
                needs_repaint: resp.needs_repaint,
                viewport_size: Some(resp.viewport_size),
                native_drag_started: resp.native_drag_started,
                expand_to_full: resp.expand_to_full,
            };
        }

        let mut triggered = None;
        let button_size = egui::vec2(panel.button_width, panel.button_height);
        let cols = panel.cols as usize;

        egui::Grid::new("meotool_grid")
            .spacing(egui::vec2(panel.gap_x, panel.gap_y))
            .show(ui, |ui| {
                for (index, button) in buttons.iter().enumerate() {
                    if index > 0 && index % cols == 0 {
                        ui.end_row();
                    }

                    let row = (index / cols) as u8;
                    let col = (index % cols) as u8;
                    let selected = selected_cell == Some((row, col));

                    let flash_color = flash_index.filter(|(i, _)| *i == index).map(|(_, c)| c);

                    let result = paint_button(
                        ui,
                        theme,
                        icons,
                        button,
                        index,
                        button_size,
                        panel.opacity,
                        panel.display_mode,
                        flash_color,
                        selected,
                        menu,
                    );

                    if result.clicked {
                        triggered = Some(index);
                    }
                }
                ui.end_row();
            });

        GridResponse {
            triggered,
            needs_repaint: false,
            viewport_size: None,
            native_drag_started: false,
            expand_to_full: false,
        }
    }
}
