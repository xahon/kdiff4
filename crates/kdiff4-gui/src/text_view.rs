use egui::epaint::text::cursor::CCursor;
use egui::{Align2, Color32, CursorIcon, FontId, Galley, Painter, Pos2, Rect, ScrollArea, Sense, Stroke, TextStyle, Ui, pos2, vec2};

use crate::editing::{self, Cursor, Lines, Position};
use crate::rows::{Mark, PaneRow, Rows, Shade};
use crate::theme::Palette;


/// Extra height of a row above the font's line height, so carets and text don't touch the next row.
const ROW_PADDING: f32 = 2.0;
const CARET_WIDTH: f32 = 1.5;
/// Width of the selected line break at the end of a selected line, as a share of the row height.
const SELECTED_LINE_BREAK_WIDTH: f32 = 0.4;
/// Distance between the stripes of a void region.
const HATCH_SPACING: f32 = 8.0;
/// Space on each side of a line number and of D's line origin.
const MARGIN_PADDING: f32 = 4.0;
const GUTTER_WIDTH: f32 = 12.0;
/// Space between a gutter block and the column's edges.
const GUTTER_INSET: f32 = 2.0;
/// Space between the margin and the text.
const TEXT_GAP: f32 = 4.0;
const CONFLICT_MARKER: &str = "<Merge Conflict>";


/// What a pane shows left of its text, after the line numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarginKind {
    Nothing,
    /// A bordered block per region.
    Gutters,
    /// Where each line of D came from: A, B, C, or M for a hand edit.
    Marks,
}


/// How a view draws: colors, its margin and the current region.
#[derive(Debug, Clone, Copy)]
pub struct ViewStyle<'a> {
    pub palette: &'a Palette,
    pub line_numbers: bool,
    pub margin: MarginKind,
    pub current_region: Option<usize>,
    pub current_region_color: Color32,
    /// Rows kept above a region the view jumps to.
    pub jump_context_rows: usize,
}


/// The caret and selection, in lines, and the void row the caret sits on, if it is in a void.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caret {
    pub cursor: Cursor,
    pub void_row: Option<usize>,
}


/// What the mouse did in the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerAction {
    /// The primary button went down here; on a void, `void_row` is the row. `extend` is Shift held: extend the selection instead of starting a new one.
    Press { position: Position, void_row: Option<usize>, extend: bool },
    /// The button pressed in this view is still held, now over this position: drag-select up to it.
    Drag { position: Position, void_row: Option<usize> },
    /// A double-click here: select the word.
    SelectWord(Position),
    /// A triple-click on this line: select it.
    SelectLine(usize),
}


#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ViewResponse {
    pub pointer: Option<PointerAction>,
    /// How many whole rows fit in the view: the step of PageUp and PageDown.
    pub page_rows: usize,
    /// The vertical scroll offset after this frame, in pixels.
    pub offset: f32,
}


/// Widths of the left margin's parts. The margin stays in place when the text scrolls sideways.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Margin {
    numbers: f32,
    column: f32,
}


impl Margin {
    fn total(&self) -> f32 {
        self.numbers + self.column + TEXT_GAP
    }
}


/// Draws `rows` of `lines` in a scroll area, laying out only the visible rows: text with its region's shade, or hatching for a void, behind a margin of line numbers and gutters. `caret` is drawn if it is in this view; `reveal_caret` scrolls it into view. `jump_row` scrolls that row to the top, below the style's context rows. `offset` forces the vertical scroll offset, so several views can scroll together.
pub fn show(ui: &mut Ui, id_salt: &str, lines: Lines<'_>, rows: Rows<'_>, caret: Option<Caret>, reveal_caret: bool, jump_row: Option<usize>, offset: Option<f32>, style: &ViewStyle<'_>) -> ViewResponse {
    let palette = style.palette;
    let font_id = TextStyle::Monospace.resolve(ui.style());
    let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font_id)) + ROW_PADDING;
    let viewport_height = ui.available_height();
    let page_rows = ((viewport_height / row_height).floor() as usize).max(1);
    let caret = caret.map(|caret| Caret { cursor: editing::clamp_cursor(lines, caret.cursor), void_row: caret.void_row.filter(|row| rows.row(*row).is_some_and(|pane_row| pane_row.line.is_none())) });
    let char_width = ui.painter().layout_no_wrap("0".to_string(), font_id.clone(), palette.text).size().x;
    let margin = margin_widths(style, lines.count(), char_width);

    let mut area = ScrollArea::both().id_salt(id_salt).auto_shrink(false);
    let current = offset.unwrap_or_else(|| {
        egui::scroll_area::State::load(ui.ctx(), ui.make_persistent_id(id_salt))
            .map(|state| state.clamped_offset().y)
            .unwrap_or(0.0)
    });
    let jumped = jump_row.map(|row| jump_offset(row, style.jump_context_rows, row_height));
    let revealed = caret
        .filter(|_| reveal_caret)
        .and_then(|caret| reveal_offset(caret_row(rows, caret), row_height, viewport_height, current));
    if let Some(target) = jumped.or(revealed).or(offset) {
        area = area.vertical_scroll_offset(target);
    }

    let output = ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        area.show_rows(ui, row_height, rows.len(), |ui, visible| {
            let viewport = ui.clip_rect();
            let text_clip = Rect::from_min_max(pos2(viewport.left() + margin.total(), viewport.top()), viewport.max);
            let text_painter = ui.painter().with_clip_rect(text_clip);
            for row in visible {
                let pane_row = rows.row(row).unwrap_or_default();
                let galley = pane_row.line.map(|line| layout(ui, lines, line, &font_id, palette));
                let text_width = galley.as_ref().map_or(0.0, |galley| galley.size().x);
                let width = (margin.total() + text_width + CARET_WIDTH * 2.0).max(ui.available_width());
                let (rect, _) = ui.allocate_exact_size(vec2(width, row_height), Sense::hover());
                let text_rect = Rect::from_min_max(pos2(rect.left() + margin.total(), rect.top()), pos2(rect.right().max(viewport.right()), rect.bottom()));
                if let Some(shade) = pane_row.shade {
                    text_painter.rect_filled(text_rect, 0.0, shade_color(palette, shade));
                }
                if pane_row.region.is_some() && pane_row.region == style.current_region {
                    ui.painter().rect_filled(Rect::from_min_max(pos2(viewport.left(), rect.top()), pos2(viewport.right(), rect.bottom())), 0.0, style.current_region_color);
                }
                paint_margin(ui, rows, row, &pane_row, viewport.left(), rect, margin, style, &font_id);
                let caret_on_void = caret.and_then(|caret| caret.void_row) == Some(row);
                let (Some(line), Some(galley)) = (pane_row.line, galley) else {
                    if pane_row.conflict_marker {
                        text_painter.text(pos2(text_rect.left(), text_rect.center().y), Align2::LEFT_CENTER, CONFLICT_MARKER, font_id.clone(), palette.conflict_marker);
                    } else {
                        paint_void(&text_painter, text_rect, palette);
                    }
                    if caret_on_void {
                        paint_caret(&text_painter, text_rect.left() + CARET_WIDTH / 2.0, rect, palette);
                    }
                    continue;
                };
                let text_origin = text_rect.min + vec2(0.0, (row_height - galley.size().y) / 2.0);
                if let Some(caret) = caret {
                    paint_selection(&text_painter, lines, line, text_rect, &galley, caret.cursor, row_height, palette);
                }
                let caret_x = caret
                    .filter(|caret| caret.void_row.is_none() && caret.cursor.head.line == line)
                    .map(|caret| text_origin.x + galley.pos_from_cursor(CCursor::new(caret.cursor.head.column)).min.x);
                text_painter.galley(text_origin, galley, palette.text);
                if let Some(x) = caret_x {
                    paint_caret(&text_painter, x, rect, palette);
                }
            }
        })
    });
    let output = output.inner;

    // One target for the whole visible pane, so empty space, the margin and the space right of a line take the mouse too.
    let response = ui.interact(output.inner_rect, ui.make_persistent_id((id_salt, "pointer")), Sense::click_and_drag());
    let held = response.is_pointer_button_down_on();
    if response.hovered() || held {
        ui.ctx().set_cursor_icon(CursorIcon::Text);
    }
    let scroll = output.state.clamped_offset();
    let hit = |pointer: Pos2| {
        let text_x = (pointer.x - output.inner_rect.min.x - margin.total()).max(0.0) + scroll.x;
        let content_y = pointer.y - output.inner_rect.min.y + scroll.y;
        position_at(ui, lines, rows, &font_id, palette, row_height, text_x, content_y)
    };
    let pressed = response.hovered() && ui.input(|input| input.pointer.primary_pressed());
    let pointer_position = ui.input(|input| input.pointer.interact_pos());
    let pointer = match pointer_position {
        Some(pointer) if response.triple_clicked() => Some(PointerAction::SelectLine(hit(pointer).0.line)),
        Some(pointer) if response.double_clicked() => Some(PointerAction::SelectWord(hit(pointer).0)),
        Some(pointer) if pressed => {
            let (position, void_row) = hit(pointer);
            Some(PointerAction::Press { position, void_row, extend: ui.input(|input| input.modifiers.shift) })
        }
        Some(pointer) if held => {
            let (position, void_row) = hit(pointer);
            Some(PointerAction::Drag { position, void_row })
        }
        _ => None,
    };
    ViewResponse { pointer, page_rows, offset: scroll.y }
}


fn margin_widths(style: &ViewStyle<'_>, line_count: usize, char_width: f32) -> Margin {
    let numbers = if style.line_numbers { digits(line_count) as f32 * char_width + MARGIN_PADDING * 2.0 } else { 0.0 };
    let column = match style.margin {
        MarginKind::Nothing => 0.0,
        MarginKind::Gutters => GUTTER_WIDTH,
        MarginKind::Marks => char_width + MARGIN_PADDING * 2.0,
    };
    Margin { numbers, column }
}


/// Decimal digits of the largest line number, at least one.
fn digits(line_count: usize) -> usize {
    line_count.max(1).to_string().len()
}


/// The line number, then the gutter block or D's line origin, drawn at the left edge of the view whatever the sideways scroll.
fn paint_margin(ui: &Ui, rows: Rows<'_>, row: usize, pane_row: &PaneRow, left: f32, rect: Rect, margin: Margin, style: &ViewStyle<'_>, font_id: &FontId) {
    let palette = style.palette;
    let painter = ui.painter();
    if margin.numbers > 0.0
        && let Some(line) = pane_row.line
    {
        painter.text(pos2(left + margin.numbers - MARGIN_PADDING, rect.center().y), Align2::RIGHT_CENTER, (line + 1).to_string(), font_id.clone(), palette.weak_text);
    }
    let column_left = left + margin.numbers;
    match style.margin {
        MarginKind::Nothing => {}
        MarginKind::Gutters => {
            if pane_row.region.is_some() {
                let (first, last) = rows.region_edges(row);
                paint_gutter_block(painter, column_left, rect, first, last, palette);
            }
        }
        MarginKind::Marks => {
            if let Some(mark) = pane_row.mark {
                painter.text(pos2(column_left + margin.column / 2.0, rect.center().y), Align2::CENTER_CENTER, mark_letter(mark), font_id.clone(), palette.weak_text);
            }
        }
    }
    if margin.numbers + margin.column > 0.0 {
        let x = left + margin.numbers + margin.column + TEXT_GAP / 2.0;
        painter.vline(x, rect.y_range(), Stroke::new(1.0, palette.void_hatch));
    }
}


/// One row's slice of a region's gutter block: the sides on every row, the top on its first row and the bottom on its last, so the region reads as one bordered block.
fn paint_gutter_block(painter: &Painter, column_left: f32, rect: Rect, first: bool, last: bool, palette: &Palette) {
    let left = column_left + GUTTER_INSET;
    let right = column_left + GUTTER_WIDTH - GUTTER_INSET;
    let top = if first { rect.top() + 1.0 } else { rect.top() };
    let bottom = if last { rect.bottom() - 1.0 } else { rect.bottom() };
    let block = Rect::from_min_max(pos2(left, top), pos2(right, bottom));
    painter.rect_filled(block, 0.0, palette.gutter_fill);
    let stroke = Stroke::new(1.0, palette.gutter_border);
    painter.vline(left, block.y_range(), stroke);
    painter.vline(right, block.y_range(), stroke);
    if first {
        painter.hline(block.x_range(), top, stroke);
    }
    if last {
        painter.hline(block.x_range(), bottom, stroke);
    }
}


pub fn mark_letter(mark: Mark) -> &'static str {
    match mark {
        Mark::Source(document) => document.letter(),
        Mark::Modified => "M",
    }
}


fn paint_caret(painter: &Painter, x: f32, rect: Rect, palette: &Palette) {
    painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(CARET_WIDTH, palette.caret));
}


fn shade_color(palette: &Palette, shade: Shade) -> Color32 {
    match shade {
        Shade::Removed => palette.removed_bg,
        Shade::Added => palette.added_bg,
        Shade::Modified => palette.modified_bg,
        Shade::Conflict => palette.conflict_bg,
    }
}


/// Diagonal stripes anchored to the screen, so the stripes of consecutive void rows join up.
fn paint_void(painter: &Painter, rect: Rect, palette: &Palette) {
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let stroke = Stroke::new(1.0, palette.void_hatch);
    let first = ((rect.left() + rect.top()) / HATCH_SPACING).floor() as i64;
    let last = ((rect.right() + rect.bottom()) / HATCH_SPACING).ceil() as i64;
    for stripe in first..=last {
        let sum = stripe as f32 * HATCH_SPACING;
        painter.line_segment([pos2(sum - rect.bottom(), rect.bottom()), pos2(sum - rect.top(), rect.top())], stroke);
    }
}


fn layout(ui: &Ui, lines: Lines<'_>, row: usize, font_id: &FontId, palette: &Palette) -> std::sync::Arc<Galley> {
    ui.painter().layout_no_wrap(lines.line(row).to_string(), font_id.clone(), palette.text)
}


/// The row the caret is drawn on.
pub fn caret_row(rows: Rows<'_>, caret: Caret) -> usize {
    caret.void_row.unwrap_or_else(|| rows.row_of_line(caret.cursor.head.line))
}


/// The text position at `x`, `y` pixels from the top left of the text, and the row if it is a void. Below the last row it is the end of the text; right of a line's text, the end of that line; on a void, the start of the next line.
fn position_at(ui: &Ui, lines: Lines<'_>, rows: Rows<'_>, font_id: &FontId, palette: &Palette, row_height: f32, x: f32, y: f32) -> (Position, Option<usize>) {
    let last_line = lines.count().saturating_sub(1);
    let end = Position { line: last_line, column: lines.line_chars(last_line) };
    let Some(row) = row_at(y, row_height, rows.len()) else {
        return (end, None);
    };
    match rows.row(row).and_then(|pane_row| pane_row.line) {
        Some(line) => {
            let galley = layout(ui, lines, line, font_id, palette);
            let column = galley.cursor_from_pos(vec2(x, galley.size().y / 2.0)).index.0;
            (Position { line, column }, None)
        }
        None => rows.caret_at_row(row, 0, end),
    }
}


/// Highlights the selected part of `line`, drawn in `rect`. A selection that continues past the line also covers a little space for the line break.
fn paint_selection(painter: &Painter, lines: Lines<'_>, line: usize, rect: Rect, galley: &Galley, cursor: Cursor, row_height: f32, palette: &Palette) {
    if !cursor.has_selection() {
        return;
    }
    let (start, end) = cursor.range();
    if line < start.line || line > end.line {
        return;
    }
    let from = if line == start.line { start.column } else { 0 };
    let to = if line == end.line { end.column } else { lines.line_chars(line) };
    let left = rect.left() + galley.pos_from_cursor(CCursor::new(from)).min.x;
    let mut right = rect.left() + galley.pos_from_cursor(CCursor::new(to)).min.x;
    if line < end.line {
        right += row_height * SELECTED_LINE_BREAK_WIDTH;
    }
    painter.rect_filled(Rect::from_min_max(pos2(left, rect.top()), pos2(right, rect.bottom())), 0.0, palette.selection);
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


/// The scroll offset that shows `row` with `context` rows above it, or at the top when there are fewer.
pub fn jump_offset(row: usize, context: usize, row_height: f32) -> f32 {
    row.saturating_sub(context) as f32 * row_height
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::LIGHT;


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


    #[test]
    fn jumps_keep_context_rows_above() {
        assert_eq!(jump_offset(10, 3, 10.0), 70.0);
        assert_eq!(jump_offset(2, 3, 10.0), 0.0);
        assert_eq!(jump_offset(5, 0, 10.0), 50.0);
    }


    fn style(line_numbers: bool, margin: MarginKind) -> ViewStyle<'static> {
        ViewStyle { palette: &LIGHT, line_numbers, margin, current_region: None, current_region_color: Color32::TRANSPARENT, jump_context_rows: 0 }
    }


    #[test]
    fn margin_fits_the_largest_line_number() {
        assert_eq!(digits(0), 1);
        assert_eq!(digits(9), 1);
        assert_eq!(digits(10), 2);
        assert_eq!(digits(12345), 5);
        let numbers = margin_widths(&style(true, MarginKind::Nothing), 120, 7.0);
        assert_eq!(numbers, Margin { numbers: 3.0 * 7.0 + MARGIN_PADDING * 2.0, column: 0.0 });
        assert_eq!(margin_widths(&style(false, MarginKind::Gutters), 120, 7.0), Margin { numbers: 0.0, column: GUTTER_WIDTH });
        assert_eq!(margin_widths(&style(false, MarginKind::Marks), 1, 7.0).column, 7.0 + MARGIN_PADDING * 2.0);
        assert_eq!(margin_widths(&style(false, MarginKind::Nothing), 1, 7.0).total(), TEXT_GAP);
    }


    #[test]
    fn marks_read_as_letters() {
        assert_eq!(mark_letter(Mark::Source(kdiff4_protocol::Document::B)), "B");
        assert_eq!(mark_letter(Mark::Modified), "M");
    }


    #[test]
    fn caret_row_prefers_the_void_row() {
        let rows = Rows::PerLine(5);
        let cursor = Cursor::at(Position { line: 3, column: 0 });
        assert_eq!(caret_row(rows, Caret { cursor, void_row: None }), 3);
        assert_eq!(caret_row(rows, Caret { cursor, void_row: Some(1) }), 1);
    }
}
