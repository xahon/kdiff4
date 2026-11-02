use egui::epaint::text::cursor::CCursor;
use egui::{CursorIcon, FontId, Galley, Pos2, Rect, ScrollArea, Sense, Stroke, TextStyle, Ui, pos2, vec2};

use crate::editing::{self, Cursor, Lines, Position};
use crate::theme::Palette;


/// Extra height of a row above the font's line height, so carets and text don't touch the next row.
const ROW_PADDING: f32 = 2.0;
const CARET_WIDTH: f32 = 1.5;
/// Width of the selected line break at the end of a selected line, as a share of the row height.
const SELECTED_LINE_BREAK_WIDTH: f32 = 0.4;


/// What the mouse did in the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerAction {
    /// The primary button went down here. `extend` is Shift held: extend the selection instead of starting a new one.
    Press { position: Position, extend: bool },
    /// The button pressed in this view is still held, now over this position: drag-select up to it.
    Drag(Position),
    /// A double-click here: select the word.
    SelectWord(Position),
    /// A triple-click on this line: select it.
    SelectLine(usize),
}


#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ViewResponse {
    pub pointer: Option<PointerAction>,
    /// How many whole rows fit in the view: the step of PageUp and PageDown.
    pub page_rows: usize,
}


/// Draws `lines` in a scroll area, laying out only the visible rows. `cursor` (caret and selection) is drawn if it is in this view; `reveal_caret` scrolls the caret into view.
pub fn show(ui: &mut Ui, id_salt: &str, lines: Lines<'_>, cursor: Option<Cursor>, reveal_caret: bool, palette: &Palette) -> ViewResponse {
    let font_id = TextStyle::Monospace.resolve(ui.style());
    let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font_id)) + ROW_PADDING;
    let viewport_height = ui.available_height();
    let page_rows = ((viewport_height / row_height).floor() as usize).max(1);
    let cursor = cursor.map(|cursor| editing::clamp_cursor(lines, cursor));

    let mut area = ScrollArea::both().id_salt(id_salt).auto_shrink(false);
    if reveal_caret
        && let Some(cursor) = cursor
    {
        let current = egui::scroll_area::State::load(ui.ctx(), ui.make_persistent_id(id_salt))
            .map(|state| state.clamped_offset().y)
            .unwrap_or(0.0);
        if let Some(offset) = reveal_offset(cursor.head.line, row_height, viewport_height, current) {
            area = area.vertical_scroll_offset(offset);
        }
    }

    let output = ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        area.show_rows(ui, row_height, lines.count(), |ui, rows| {
            for row in rows {
                let galley = layout(ui, lines, row, &font_id, palette);
                let width = (galley.size().x + CARET_WIDTH * 2.0).max(ui.available_width());
                let (rect, _) = ui.allocate_exact_size(vec2(width, row_height), Sense::hover());
                let text_origin = rect.min + vec2(0.0, (row_height - galley.size().y) / 2.0);
                if let Some(cursor) = cursor {
                    paint_selection(ui, lines, row, rect, &galley, cursor, row_height, palette);
                }
                let caret_x = cursor
                    .filter(|cursor| cursor.head.line == row)
                    .map(|cursor| text_origin.x + galley.pos_from_cursor(CCursor::new(cursor.head.column)).min.x);
                ui.painter().galley(text_origin, galley, palette.text);
                if let Some(x) = caret_x {
                    ui.painter().line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(CARET_WIDTH, palette.caret));
                }
            }
        })
    });
    let output = output.inner;

    // One target for the whole visible pane, so empty space and the space right of a line take the mouse too.
    let response = ui.interact(output.inner_rect, ui.make_persistent_id((id_salt, "pointer")), Sense::click_and_drag());
    let held = response.is_pointer_button_down_on();
    if response.hovered() || held {
        ui.ctx().set_cursor_icon(CursorIcon::Text);
    }
    let hit = |pointer: Pos2| {
        let content = pointer - output.inner_rect.min + output.state.clamped_offset();
        position_at(ui, lines, &font_id, palette, row_height, content.x, content.y)
    };
    let pressed = response.hovered() && ui.input(|input| input.pointer.primary_pressed());
    let pointer_position = ui.input(|input| input.pointer.interact_pos());
    let pointer = match pointer_position {
        Some(pointer) if response.triple_clicked() => Some(PointerAction::SelectLine(hit(pointer).line)),
        Some(pointer) if response.double_clicked() => Some(PointerAction::SelectWord(hit(pointer))),
        Some(pointer) if pressed => Some(PointerAction::Press { position: hit(pointer), extend: ui.input(|input| input.modifiers.shift) }),
        Some(pointer) if held => Some(PointerAction::Drag(hit(pointer))),
        _ => None,
    };
    ViewResponse { pointer, page_rows }
}


fn layout(ui: &Ui, lines: Lines<'_>, row: usize, font_id: &FontId, palette: &Palette) -> std::sync::Arc<Galley> {
    ui.painter().layout_no_wrap(lines.line(row).to_string(), font_id.clone(), palette.text)
}


/// The text position at `x`, `y` pixels from the top left of the text. Below the last line it is the end of the text; right of a line's text, the end of that line.
fn position_at(ui: &Ui, lines: Lines<'_>, font_id: &FontId, palette: &Palette, row_height: f32, x: f32, y: f32) -> Position {
    match row_at(y, row_height, lines.count()) {
        Some(row) => {
            let galley = layout(ui, lines, row, font_id, palette);
            let column = galley.cursor_from_pos(vec2(x, galley.size().y / 2.0)).index.0;
            Position { line: row, column }
        }
        None => {
            let line = lines.count().saturating_sub(1);
            Position { line, column: lines.line_chars(line) }
        }
    }
}


/// Highlights the selected part of `row`. A selection that continues past the row also covers a little space for the line break.
fn paint_selection(ui: &Ui, lines: Lines<'_>, row: usize, rect: Rect, galley: &Galley, cursor: Cursor, row_height: f32, palette: &Palette) {
    if !cursor.has_selection() {
        return;
    }
    let (start, end) = cursor.range();
    if row < start.line || row > end.line {
        return;
    }
    let from = if row == start.line { start.column } else { 0 };
    let to = if row == end.line { end.column } else { lines.line_chars(row) };
    let left = rect.left() + galley.pos_from_cursor(CCursor::new(from)).min.x;
    let mut right = rect.left() + galley.pos_from_cursor(CCursor::new(to)).min.x;
    if row < end.line {
        right += row_height * SELECTED_LINE_BREAK_WIDTH;
    }
    ui.painter().rect_filled(Rect::from_min_max(pos2(left, rect.top()), pos2(right, rect.bottom())), 0.0, palette.selection);
}


/// The row at `content_y` pixels from the top of the text, or `None` below the last row.
pub fn row_at(content_y: f32, row_height: f32, line_count: usize) -> Option<usize> {
    if row_height <= 0.0 || content_y < 0.0 {
        return (line_count > 0).then_some(0);
    }
    let row = (content_y / row_height).floor() as usize;
    (row < line_count).then_some(row)
}


/// The scroll offset that brings `line` fully into a viewport scrolled to `current`, or `None` if it is already visible.
pub fn reveal_offset(line: usize, row_height: f32, viewport_height: f32, current: f32) -> Option<f32> {
    let top = line as f32 * row_height;
    let bottom = top + row_height;
    if top < current {
        Some(top)
    } else if bottom > current + viewport_height {
        Some((bottom - viewport_height).max(0.0))
    } else {
        None
    }
}


#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn click_height_maps_to_row() {
        assert_eq!(row_at(0.0, 10.0, 3), Some(0));
        assert_eq!(row_at(9.9, 10.0, 3), Some(0));
        assert_eq!(row_at(10.0, 10.0, 3), Some(1));
        assert_eq!(row_at(29.0, 10.0, 3), Some(2));
    }


    #[test]
    fn click_below_last_row_has_no_row() {
        assert_eq!(row_at(30.0, 10.0, 3), None);
        assert_eq!(row_at(500.0, 10.0, 3), None);
        assert_eq!(row_at(0.0, 10.0, 0), None);
    }


    #[test]
    fn click_above_text_is_first_row() {
        assert_eq!(row_at(-3.0, 10.0, 3), Some(0));
        assert_eq!(row_at(-3.0, 10.0, 0), None);
    }


    #[test]
    fn visible_line_needs_no_scrolling() {
        assert_eq!(reveal_offset(3, 10.0, 100.0, 0.0), None);
        assert_eq!(reveal_offset(9, 10.0, 100.0, 0.0), None);
    }


    #[test]
    fn line_above_view_scrolls_up_to_it() {
        assert_eq!(reveal_offset(2, 10.0, 100.0, 50.0), Some(20.0));
    }


    #[test]
    fn line_below_view_scrolls_down_until_it_fits() {
        assert_eq!(reveal_offset(10, 10.0, 100.0, 0.0), Some(10.0));
        assert_eq!(reveal_offset(0, 10.0, 5.0, 0.0), Some(5.0));
    }
}
