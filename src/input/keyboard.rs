use egui::{Context, Key};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardAction {
    None,
    Trigger(usize),
    ClearSelection,
}

/// 处理面板聚焦时的键盘输入：方向键、Enter、Escape。
/// `selectable` 为各槽位是否可选（长度应为 rows×cols）。
pub fn handle_panel_keyboard(
    ctx: &Context,
    rows: u8,
    cols: u8,
    selected: &mut Option<(u8, u8)>,
    selectable: &[bool],
) -> KeyboardAction {
    if ctx.wants_pointer_input() && ctx.is_pointer_over_area() {
        // 有其他控件捕获输入时不处理
    }

    let mut action = KeyboardAction::None;

    ctx.input(|input| {
        if !input.focused {
            return;
        }

        for event in &input.events {
            if let egui::Event::Key {
                key,
                pressed: true,
                repeat: false,
                ..
            } = event
            {
                match key {
                    Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight => {
                        move_selection(rows, cols, selected, *key, selectable);
                    }
                    Key::Enter => {
                        if let Some((row, col)) = *selected {
                            let index = row as usize * cols as usize + col as usize;
                            action = KeyboardAction::Trigger(index);
                        }
                    }
                    Key::Escape => {
                        *selected = None;
                        action = KeyboardAction::ClearSelection;
                    }
                    _ => {}
                }
            }
        }
    });

    action
}

fn move_selection(
    rows: u8,
    cols: u8,
    selected: &mut Option<(u8, u8)>,
    key: Key,
    selectable: &[bool],
) {
    let (start_row, start_col) = selected.unwrap_or((0, 0));
    let (drow, dcol): (i16, i16) = match key {
        Key::ArrowUp => (-1, 0),
        Key::ArrowDown => (1, 0),
        Key::ArrowLeft => (0, -1),
        Key::ArrowRight => (0, 1),
        _ => return,
    };

    let mut row = start_row as i16;
    let mut col = start_col as i16;
    let rows_i = rows as i16;
    let cols_i = cols as i16;

    loop {
        row += drow;
        col += dcol;
        if row < 0 || col < 0 || row >= rows_i || col >= cols_i {
            *selected = Some((start_row, start_col));
            return;
        }
        let index = row as usize * cols as usize + col as usize;
        if selectable.get(index).copied().unwrap_or(false) {
            *selected = Some((row as u8, col as u8));
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_selection_bounds() {
        let selectable = vec![true; 9];
        let mut sel = Some((0, 0));
        move_selection(3, 3, &mut sel, Key::ArrowDown, &selectable);
        assert_eq!(sel, Some((1, 0)));
        move_selection(3, 3, &mut sel, Key::ArrowRight, &selectable);
        assert_eq!(sel, Some((1, 1)));
        move_selection(3, 3, &mut sel, Key::ArrowRight, &selectable);
        move_selection(3, 3, &mut sel, Key::ArrowRight, &selectable);
        assert_eq!(sel, Some((1, 2)));
    }

    #[test]
    fn move_selection_skips_empty_slots() {
        // 槽位: 0 可选, 1 空, 2 可选
        let selectable = vec![true, false, true];
        let mut sel = Some((0, 0));
        move_selection(1, 3, &mut sel, Key::ArrowRight, &selectable);
        assert_eq!(sel, Some((0, 2)));
    }

    #[test]
    fn move_selection_keeps_when_no_target() {
        let selectable = vec![true, false, false];
        let mut sel = Some((0, 0));
        move_selection(1, 3, &mut sel, Key::ArrowRight, &selectable);
        assert_eq!(sel, Some((0, 0)));
    }
}
