use std::ops::Range;

use egui::epaint::text::cursor::CCursor;
use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, CursorIcon, FontId, Galley, Painter, Pos2, Rangef, Rect, ScrollArea, Sense, Stroke, StrokeKind, TextStyle, Ui, pos2, vec2};

use kdiff4_protocol::{Document, LineCharRuns};

use crate::char_diff::{self, CharStyle};
use crate::config::WhitespaceConfig;
use crate::editing::{self, Cursor, Lines, Position};
use crate::rows::{Block, Mark, PaneRow, Rows};
use crate::theme::{DocumentColors, Palette};


/// Extra height of a row above the font's line height, so carets and text don't touch the next row.
const ROW_PADDING: f32 = 2.0;
const CARET_WIDTH: f32 = 1.5;
/// Width of the selected line break at the end of a selected line, as a share of the row height.
const SELECTED_LINE_BREAK_WIDTH: f32 = 0.4;
/// Distance between the stripes of a void region.
const HATCH_SPACING: f32 = 8.0;
/// Space on each side of a line number and of D's line origin.
const MARGIN_PADDING: f32 = 4.0;
/// Space between a gutter block and the column's edges. The column is one row height wide, so with the block's top and bottom inset by this too, a one-row block is a square.
const GUTTER_INSET: f32 = 1.0;
const ACTIVE_GUTTER_STROKE: f32 = 2.0;
/// Space between the margin and the text.
const TEXT_GAP: f32 = 4.0;
pub const CONFLICT_MARKER: &str = "<Merge Conflict>";
const CHAR_DIFF_BACKGROUND_ALPHA: u8 = 80;
const REGION_BORDER_WIDTH: f32 = 1.0;
/// Whitespace markers are drawn in the weak text color at this opacity, so they stay behind the text.
const WHITESPACE_OPACITY: f32 = 0.6;


/// What a pane shows left of its text, after the line numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarginKind {
    Nothing,
    /// A bordered block per changed region, in the pane's document color.
    Gutters,
    /// D's gutter: a block per run of lines from one origin, colored and labeled with it: A, B, C, or M for a hand edit.
    Marks,
}


/// How a view draws: colors, its margin, the region boxes and the current region.
#[derive(Debug, Clone, Copy)]
pub struct ViewStyle<'a> {
    pub palette: &'a Palette,
    pub line_numbers: bool,
    pub margin: MarginKind,
    pub current_region: Option<usize>,
    pub current_region_color: Color32,
    /// Rows one wheel notch scrolls.
    pub scroll_step_rows: usize,
    pub region_corner_radius: u8,
    /// Opacity of the borders of the regions other than the current one, from 0 to 1.
    pub other_region_border_opacity: f32,
    pub whitespace: WhitespaceConfig,
}


/// The character-level diff of a source pane's lines; `sources` is how many source documents the rows have.
#[derive(Debug, Clone, Copy)]
pub struct CharRuns<'a> {
    pub document: Document,
    pub lines: &'a LineCharRuns,
    pub sources: usize,
}


/// A whitespace character the panes can mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Whitespace {
    Space,
    Tab,
    LineFeed,
    CarriageReturn,
    /// Non-breaking and other Unicode spaces.
    Other,
}


impl Whitespace {
    pub fn of(character: char) -> Option<Whitespace> {
        match character {
            ' ' => Some(Whitespace::Space),
            '\t' => Some(Whitespace::Tab),
            '\n' => Some(Whitespace::LineFeed),
            '\r' => Some(Whitespace::CarriageReturn),
            other if other.is_whitespace() => Some(Whitespace::Other),
            _ => None,
        }
    }


    pub fn is_shown(&self, config: &WhitespaceConfig) -> bool {
        config.render
            && match self {
                Whitespace::Space => config.spaces,
                Whitespace::Tab => config.tabs,
                Whitespace::LineFeed => config.line_feeds,
                Whitespace::CarriageReturn => config.carriage_returns,
                Whitespace::Other => config.other,
            }
    }
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
    /// A click on a gutter block of this region: make it current.
    SelectRegion(usize),
}


#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ViewResponse {
    pub pointer: Option<PointerAction>,
    /// How many whole rows fit in the view: the step of PageUp and PageDown.
    pub page_rows: usize,
    pub viewport: Viewport,
    /// The wheel scrolling over the view this frame; the caller scrolls the view, so panes that scroll together move as one.
    pub wheel: Wheel,
}


/// Pixels the wheel asked to scroll down in a frame, negative for up: `stepped` by mouse wheel notches, which may scroll smoothly, and `direct` by a trackpad, which follows the fingers at once.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Wheel {
    pub stepped: f32,
    pub direct: f32,
}


/// The rows a view shows after a frame: its vertical scroll offset and height, the height of a row and of all rows, in pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Viewport {
    pub offset: f32,
    pub height: f32,
    pub row_height: f32,
    pub content_height: f32,
}


impl Viewport {
    /// Whether every row of `rows` is fully visible. Rows that are empty count as visible when the place they would be is.
    pub fn shows(&self, rows: Range<usize>) -> bool {
        const TOLERANCE: f32 = 0.5;
        let top = rows.start as f32 * self.row_height;
        let bottom = rows.end.max(rows.start) as f32 * self.row_height;
        top + TOLERANCE >= self.offset && bottom <= self.offset + self.height + TOLERANCE
    }


    /// The largest vertical scroll offset.
    pub fn max_offset(&self) -> f32 {
        (self.content_height - self.height).max(0.0)
    }


    /// How many whole rows fit in the view, at least one.
    pub fn page_rows(&self) -> usize {
        if self.row_height > 0.0 { ((self.height / self.row_height).floor() as usize).max(1) } else { 1 }
    }
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


/// Draws `rows` of `lines` in a scroll area, laying out only the visible rows: text with its char diff from `char_runs`, or hatching for a void, behind a margin of line numbers and gutters, with a bordered box around each region that differs and the current region's background. `caret` is drawn if it is in this view; `reveal_caret` scrolls it into view. `offset` forces the vertical scroll offset, so several views can scroll together. Vertical wheel scrolling is not applied but reported, for the caller to scroll with.
pub fn show(ui: &mut Ui, id_salt: &str, lines: Lines<'_>, rows: Rows<'_>, char_runs: Option<CharRuns<'_>>, caret: Option<Caret>, reveal_caret: bool, offset: Option<f32>, style: &ViewStyle<'_>) -> ViewResponse {
    let palette = style.palette;
    let font_id = TextStyle::Monospace.resolve(ui.style());
    let row_height = row_height(ui);
    let viewport_height = ui.available_height();
    let page_rows = ((viewport_height / row_height).floor() as usize).max(1);
    let caret = caret.map(|caret| Caret { cursor: editing::clamp_cursor(lines, caret.cursor), void_row: caret.void_row.filter(|row| rows.row(*row).is_some_and(|pane_row| pane_row.line.is_none())) });
    let char_width = ui.painter().layout_no_wrap("0".to_string(), font_id.clone(), palette.pane_text).size().x;
    let margin = margin_widths(style, lines.count(), char_width, row_height);

    let mut area = ScrollArea::both().id_salt(id_salt).auto_shrink(false);
    let current = offset.unwrap_or_else(|| {
        egui::scroll_area::State::load(ui.ctx(), ui.make_persistent_id(id_salt))
            .map(|state| state.clamped_offset().y)
            .unwrap_or(0.0)
    });
    let revealed = caret
        .filter(|_| reveal_caret)
        .and_then(|caret| reveal_offset(caret_row(rows, caret), row_height, viewport_height, current));
    if let Some(target) = revealed.or(offset) {
        area = area.vertical_scroll_offset(target);
    }
    let wheel = take_vertical_wheel(ui, row_height, viewport_height, style.scroll_step_rows);

    let output = ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        area.show_rows(ui, row_height, rows.len(), |ui, visible| {
            let viewport = ui.clip_rect();
            let text_clip = Rect::from_min_max(pos2(viewport.left() + margin.total(), viewport.top()), viewport.max);
            let text_painter = ui.painter().with_clip_rect(text_clip);
            let mut boxes = Vec::new();
            let mut previous_region = None;
            for row in visible {
                let pane_row = rows.row(row).unwrap_or_default();
                let segments = pane_row.line
                    .zip(char_runs)
                    .map(|(line, runs)| char_diff::segments(runs.document.clone(), runs.lines.line(line), runs.sources.clone()))
                    .unwrap_or_default();
                let galley = pane_row.line.map(|line| layout_styled(ui, lines.line(line), &segments, &font_id, palette));
                let terminator = pane_row.line.map(|line| shown_terminator(lines, line, &style.whitespace)).unwrap_or_default();
                let text_width = galley.as_ref().map_or(0.0, |galley| galley.size().x) + terminator.len() as f32 * char_width;
                let width = (margin.total() + text_width + CARET_WIDTH * 2.0).max(ui.available_width());
                let (rect, _) = ui.allocate_exact_size(vec2(width, row_height), Sense::hover());
                let text_rect = Rect::from_min_max(pos2(rect.left() + margin.total(), rect.top()), pos2(rect.right().max(viewport.right()), rect.bottom()));
                if let Some(region) = pane_row.region
                    && pane_row.region != previous_region
                    && let Some(region_rect) = rows.region_rows(region).and_then(|range| region_box(range, row, rect.top(), row_height, viewport.x_range()))
                {
                    if pane_row.region == style.current_region {
                        ui.painter().rect_filled(region_rect, style.region_corner_radius, style.current_region_color);
                    }
                    if pane_row.boxed {
                        boxes.push((region_rect, pane_row.region == style.current_region));
                    }
                }
                previous_region = pane_row.region;
                paint_margin(ui, rows, row, &pane_row, viewport.left(), rect, margin, style, &font_id);
                let caret_on_void = caret.and_then(|caret| caret.void_row) == Some(row);
                let (Some(line), Some(galley)) = (pane_row.line, galley) else {
                    if pane_row.conflict_marker {
                        text_painter.text(pos2(text_rect.left(), text_rect.center().y), Align2::LEFT_CENTER, CONFLICT_MARKER, font_id.clone(), palette.red());
                    } else {
                        paint_void(&text_painter, text_rect, palette);
                    }
                    if caret_on_void {
                        paint_caret(&text_painter, text_rect.left() + CARET_WIDTH / 2.0, rect, palette);
                    }
                    continue;
                };
                let text_origin = text_rect.min + vec2(0.0, (row_height - galley.size().y) / 2.0);
                paint_char_backgrounds(&text_painter, &segments, &galley, text_origin.x, rect, palette);
                if let Some(caret) = caret {
                    paint_selection(&text_painter, lines, line, text_rect, &galley, caret.cursor, row_height, palette);
                }
                let caret_x = caret
                    .filter(|caret| caret.void_row.is_none() && caret.cursor.head.line == line)
                    .map(|caret| text_origin.x + galley.pos_from_cursor(CCursor::new(caret.cursor.head.column)).min.x);
                paint_whitespace(&text_painter, lines.line(line), &terminator, &galley, text_origin.x, rect, char_width, style);
                text_painter.galley(text_origin, galley, palette.pane_text);
                if let Some(x) = caret_x {
                    paint_caret(&text_painter, x, rect, palette);
                }
            }
            for (region_rect, current) in boxes {
                let color = if current { palette.region_border } else { palette.region_border.gamma_multiply(style.other_region_border_opacity) };
                ui.painter().rect_stroke(region_rect, style.region_corner_radius, Stroke::new(REGION_BORDER_WIDTH, color), StrokeKind::Inside);
            }
        })
    });
    let output = output.inner;

    // One target for the whole visible pane, so empty space, the margin and the space right of a line take the mouse too.
    let response = ui.interact(output.inner_rect, ui.make_persistent_id((id_salt, "pointer")), Sense::click_and_drag());
    let held = response.is_pointer_button_down_on();
    let scroll = output.state.clamped_offset();
    let gutter_at = |pointer: Pos2| gutter_region(rows, margin, style.margin, pointer.x - output.inner_rect.min.x, pointer.y - output.inner_rect.min.y + scroll.y, row_height);
    let hover_position = ui.input(|input| input.pointer.hover_pos());
    if response.hovered() && hover_position.and_then(gutter_at).is_some() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    } else if response.hovered() || held {
        ui.ctx().set_cursor_icon(CursorIcon::Text);
    }
    let hit = |pointer: Pos2| {
        let text_x = (pointer.x - output.inner_rect.min.x - margin.total()).max(0.0) + scroll.x;
        let content_y = pointer.y - output.inner_rect.min.y + scroll.y;
        position_at(ui, lines, rows, &font_id, palette, row_height, text_x, content_y)
    };
    let pressed = response.hovered() && ui.input(|input| input.pointer.primary_pressed());
    let pointer_position = ui.input(|input| input.pointer.interact_pos());
    // A press on a gutter selects its region; the rest of that click and drag must not select text.
    let gutter_press_id = ui.make_persistent_id((id_salt, "gutter_press"));
    let gutter_region_pressed = pointer_position.filter(|_| pressed).and_then(gutter_at);
    if pressed {
        ui.data_mut(|data| data.insert_temp(gutter_press_id, gutter_region_pressed.is_some()));
    }
    let in_gutter_press = ui.data(|data| data.get_temp::<bool>(gutter_press_id)).unwrap_or(false);
    let pointer = match pointer_position {
        _ if gutter_region_pressed.is_some() => gutter_region_pressed.map(PointerAction::SelectRegion),
        _ if in_gutter_press => None,
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
    let viewport = Viewport { offset: scroll.y, height: output.inner_rect.height(), row_height, content_height: output.content_size.y };
    ViewResponse { pointer, page_rows, viewport, wheel }
}


/// The height of a row of text in the panes: the monospace font's line height plus padding.
pub fn row_height(ui: &Ui) -> f32 {
    let font_id = TextStyle::Monospace.resolve(ui.style());
    ui.fonts_mut(|fonts| fonts.row_height(&font_id)) + ROW_PADDING
}


/// The vertical wheel scrolling over the view this frame, taken from egui so its scroll area doesn't apply it too. A line of a mouse wheel scrolls `step_rows` rows; trackpads scroll by their pixels. With Ctrl (zoom) or Shift (sideways) held, egui keeps the wheel.
fn take_vertical_wheel(ui: &mut Ui, row_height: f32, page_height: f32, step_rows: usize) -> Wheel {
    if !ui.rect_contains_pointer(ui.available_rect_before_wrap()) {
        return Wheel::default();
    }
    ui.input_mut(|input| {
        let mut wheel = Wheel::default();
        for event in &input.events {
            if let egui::Event::MouseWheel { unit, delta, modifiers, .. } = event
                && !modifiers.command
                && !modifiers.shift
            {
                let pixels = wheel_pixels(unit.clone(), -delta.y, row_height, page_height, step_rows);
                match unit {
                    egui::MouseWheelUnit::Point => wheel.direct += pixels,
                    egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => wheel.stepped += pixels,
                }
            }
        }
        if !input.modifiers.command && !input.modifiers.shift {
            input.smooth_scroll_delta.y = 0.0;
        }
        wheel
    })
}


/// Pixels to scroll down for `lines` wheel units down.
pub fn wheel_pixels(unit: egui::MouseWheelUnit, lines: f32, row_height: f32, page_height: f32, step_rows: usize) -> f32 {
    match unit {
        egui::MouseWheelUnit::Point => lines,
        egui::MouseWheelUnit::Line => lines * step_rows.max(1) as f32 * row_height,
        egui::MouseWheelUnit::Page => lines * page_height,
    }
}


fn margin_widths(style: &ViewStyle<'_>, line_count: usize, char_width: f32, row_height: f32) -> Margin {
    let numbers = if style.line_numbers { digits(line_count) as f32 * char_width + MARGIN_PADDING * 2.0 } else { 0.0 };
    let column = match style.margin {
        MarginKind::Nothing => 0.0,
        MarginKind::Gutters => row_height,
        MarginKind::Marks => row_height.max(char_width + MARGIN_PADDING * 2.0),
    };
    Margin { numbers, column }
}


/// The region of the gutter block under a point, given in pixels from the view's left edge and from the top of its content.
fn gutter_region(rows: Rows<'_>, margin: Margin, kind: MarginKind, x: f32, content_y: f32, row_height: f32) -> Option<usize> {
    if kind == MarginKind::Nothing || x < margin.numbers || x >= margin.numbers + margin.column || content_y < 0.0 {
        return None;
    }
    let row = row_at(content_y, row_height, rows.len())?;
    let pane_row = rows.row(row)?;
    pane_row.block?;
    pane_row.region
}


/// Decimal digits of the largest line number, at least one.
pub fn digits(line_count: usize) -> usize {
    line_count.max(1).to_string().len()
}


/// The line number, then the gutter block, labeled with its origin in D, drawn at the left edge of the view whatever the sideways scroll.
fn paint_margin(ui: &Ui, rows: Rows<'_>, row: usize, pane_row: &PaneRow, left: f32, rect: Rect, margin: Margin, style: &ViewStyle<'_>, font_id: &FontId) {
    let palette = style.palette;
    let painter = ui.painter();
    if margin.numbers > 0.0
        && let Some(line) = pane_row.line
    {
        painter.text(pos2(left + margin.numbers - MARGIN_PADDING, rect.center().y), Align2::RIGHT_CENTER, (line + 1).to_string(), font_id.clone(), palette.weak_text);
    }
    let column_left = left + margin.numbers;
    if style.margin != MarginKind::Nothing
        && let Some(block) = pane_row.block
    {
        let (first, last) = rows.block_edges(row);
        let active = pane_row.region.is_some() && pane_row.region == style.current_region;
        let (fill, border) = gutter_colors(palette, block, pane_row.origin);
        paint_gutter_block(painter, column_left, margin.column, rect, first, last, fill, border, active);
        if style.margin == MarginKind::Marks
            && let Some(origin) = pane_row.origin
        {
            painter.text(pos2(column_left + margin.column / 2.0, rect.center().y), Align2::CENTER_CENTER, mark_letter(origin), font_id.clone(), palette.text);
        }
    }
    if margin.numbers + margin.column > 0.0 {
        let x = left + margin.numbers + margin.column + TEXT_GAP / 2.0;
        painter.vline(x, rect.y_range(), Stroke::new(1.0, palette.void_hatch));
    }
}


/// The fill and border colors of `block`: a block marked by the char-diff rules, in a source pane, takes the colors of its style; otherwise the border is its origin's, and the fill the row's `line_origin` in D, or the block's origin.
fn gutter_colors<'a>(palette: &'a Palette, block: Block, line_origin: Option<Mark>) -> (&'a DocumentColors, &'a DocumentColors) {
    match block.style {
        Some(style) => (palette.style_colors(style), palette.style_colors(style)),
        None => (origin_colors(palette, line_origin.or(block.origin)), origin_colors(palette, block.origin)),
    }
}


/// The colors of a source document; hand-edited lines of D, and blocks or lines with no known origin, take D's.
fn origin_colors(palette: &Palette, origin: Option<Mark>) -> &DocumentColors {
    match origin {
        Some(Mark::Source(document)) => palette.colors(document),
        Some(Mark::Modified) | None => &palette.output,
    }
}


/// One row's slice of a gutter block: the sides on every row, the top on its first row and the bottom on its last, so the block reads as one bordered whole. The row is filled with `fill`'s colors, the border drawn with `border`'s. Blocks of the current region are filled stronger, with a thicker border.
fn paint_gutter_block(painter: &Painter, column_left: f32, column_width: f32, rect: Rect, first: bool, last: bool, fill: &DocumentColors, border: &DocumentColors, active: bool) {
    let left = column_left + GUTTER_INSET;
    let right = column_left + column_width - GUTTER_INSET;
    let top = if first { rect.top() + GUTTER_INSET } else { rect.top() };
    let bottom = if last { rect.bottom() - GUTTER_INSET } else { rect.bottom() };
    let block = Rect::from_min_max(pos2(left, top), pos2(right, bottom));
    painter.rect_filled(block, 0.0, if active { fill.gutter_active } else { fill.header });
    let stroke = Stroke::new(if active { ACTIVE_GUTTER_STROKE } else { 1.0 }, border.gutter_border);
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


pub fn paint_caret(painter: &Painter, x: f32, rect: Rect, palette: &Palette) {
    painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(CARET_WIDTH, palette.caret));
}


/// The box of a region spanning `rows`, as wide as `x_range`, given that its row `row` starts at `row_top`. `None` when the region has no rows.
fn region_box(rows: Range<usize>, row: usize, row_top: f32, row_height: f32, x_range: Rangef) -> Option<Rect> {
    if !rows.contains(&row) {
        return None;
    }
    let top = row_top - (row - rows.start) as f32 * row_height;
    let bottom = row_top + (rows.end - row) as f32 * row_height;
    Some(Rect::from_x_y_ranges(x_range, Rangef::new(top, bottom)))
}


/// Diagonal stripes anchored to the screen, so the stripes of consecutive void rows join up.
pub fn paint_void(painter: &Painter, rect: Rect, palette: &Palette) {
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
    ui.painter().layout_no_wrap(lines.line(row).to_string(), font_id.clone(), palette.pane_text)
}


/// The characters ending `line` that the whitespace settings mark: none, a line feed, a carriage return, or both.
pub fn shown_terminator(lines: Lines<'_>, line: usize, config: &WhitespaceConfig) -> Vec<Whitespace> {
    lines
        .terminator(line)
        .chars()
        .filter_map(Whitespace::of)
        .filter(|whitespace| whitespace.is_shown(config))
        .collect()
}


/// Marks the shown whitespace inside `text`, laid out as `galley` from `left`, and then `terminator` in cells of `char_width` after the text.
pub fn paint_whitespace(painter: &Painter, text: &str, terminator: &[Whitespace], galley: &Galley, left: f32, rect: Rect, char_width: f32, style: &ViewStyle<'_>) {
    if !style.whitespace.render {
        return;
    }
    let color = style.palette.weak_text.gamma_multiply(WHITESPACE_OPACITY);
    let x_of = |column: usize| left + galley.pos_from_cursor(CCursor::new(column)).min.x;
    for (column, character) in text.chars().enumerate() {
        let whitespace = match Whitespace::of(character) {
            Some(whitespace) if whitespace.is_shown(&style.whitespace) => whitespace,
            _ => continue,
        };
        let cell = Rect::from_min_max(pos2(x_of(column), rect.top()), pos2(x_of(column + 1), rect.bottom()));
        paint_whitespace_mark(painter, whitespace, cell, color);
    }
    let end = left + galley.size().x;
    for (index, whitespace) in terminator.iter().enumerate() {
        let cell_left = end + index as f32 * char_width;
        let cell = Rect::from_min_max(pos2(cell_left, rect.top()), pos2(cell_left + char_width, rect.bottom()));
        paint_whitespace_mark(painter, whitespace.clone(), cell, color);
    }
}


/// Drawn as shapes, not glyphs, so every font shows them: a dot for a space, arrows for tabs and line ends, a ring for other spaces.
fn paint_whitespace_mark(painter: &Painter, whitespace: Whitespace, cell: Rect, color: Color32) {
    let stroke = Stroke::new(1.0, color);
    let center = cell.center();
    let size = cell.width().min(cell.height());
    match whitespace {
        Whitespace::Space => {
            painter.circle_filled(center, (size * 0.1).max(1.0), color);
        }
        Whitespace::Other => {
            painter.circle_stroke(center, (size * 0.2).max(1.5), stroke);
        }
        Whitespace::Tab => {
            let inset = (cell.width() * 0.15).min(size * 0.3);
            painter.arrow(pos2(cell.left() + inset, center.y), vec2((cell.width() - inset * 2.0).max(1.0), 0.0), stroke);
        }
        Whitespace::LineFeed => painter.arrow(pos2(center.x, center.y - size * 0.35), vec2(0.0, size * 0.7), stroke),
        Whitespace::CarriageReturn => painter.arrow(pos2(center.x + size * 0.35, center.y), vec2(-size * 0.7, 0.0), stroke),
    }
}


/// Lays out `text` with the colors of its char-diff `segments`, given in columns.
pub fn layout_styled(ui: &Ui, text: &str, segments: &[(Range<usize>, CharStyle)], font_id: &FontId, palette: &Palette) -> std::sync::Arc<Galley> {
    let mut job = LayoutJob::default();
    for (bytes, color) in colored_byte_ranges(text, segments, palette.pane_text, palette.red()) {
        job.append(&text[bytes], 0.0, TextFormat::simple(font_id.clone(), color));
    }
    ui.painter().layout_job(job)
}


/// `text` cut into byte ranges at the edges of `segments`, each with its color: `red` for a red segment, otherwise `plain`, as between segments. Columns past the text's end are ignored.
fn colored_byte_ranges(text: &str, segments: &[(Range<usize>, CharStyle)], plain: Color32, red: Color32) -> Vec<(Range<usize>, Color32)> {
    let offsets: Vec<usize> = text
        .char_indices()
        .map(|(offset, _)| offset)
        .chain(std::iter::once(text.len()))
        .collect();
    let byte = |column: usize| offsets.get(column).cloned().unwrap_or(text.len());
    let mut ranges = Vec::new();
    let mut at = 0;
    for (columns, char_style) in segments {
        let start = byte(columns.start.clone()).max(at);
        let end = byte(columns.end.clone()).max(start);
        if start > at {
            ranges.push((at..start, plain.clone()));
        }
        if end > start {
            ranges.push((start..end, if *char_style == CharStyle::Red { red.clone() } else { plain.clone() }));
        }
        at = end;
    }
    if at < text.len() || ranges.is_empty() {
        ranges.push((at..text.len(), plain));
    }
    ranges
}


/// The background of a char-diff segment: red at partial alpha, or the stronger fill of the document the characters differ from.
fn char_background(style: &CharStyle, palette: &Palette) -> Color32 {
    match style {
        CharStyle::Red => palette.red().gamma_multiply_u8(CHAR_DIFF_BACKGROUND_ALPHA),
        CharStyle::Document(_) => palette.style_colors(style.clone()).gutter_active.clone(),
    }
}


/// Fills the background of each char-diff segment, over the full height of the row `rect`; the text starts at `text_left`.
pub fn paint_char_backgrounds(painter: &Painter, segments: &[(Range<usize>, CharStyle)], galley: &Galley, text_left: f32, rect: Rect, palette: &Palette) {
    for (columns, char_style) in segments {
        if columns.is_empty() {
            continue;
        }
        let left = text_left + galley.pos_from_cursor(CCursor::new(columns.start.clone())).min.x;
        let right = text_left + galley.pos_from_cursor(CCursor::new(columns.end.clone())).min.x;
        if right <= left {
            continue;
        }
        painter.rect_filled(Rect::from_min_max(pos2(left, rect.top()), pos2(right, rect.bottom())), 0.0, char_background(char_style, palette));
    }
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
pub fn paint_selection(painter: &Painter, lines: Lines<'_>, line: usize, rect: Rect, galley: &Galley, cursor: Cursor, row_height: f32, palette: &Palette) {
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


#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::{Block, RowMap};
    use crate::theme::{DARK, LIGHT};


    const PLAIN: Color32 = Color32::BLACK;
    const FROM_B: CharStyle = CharStyle::Document(kdiff4_protocol::Document::B);


    #[test]
    fn unmarked_text_is_one_plain_range() {
        assert_eq!(colored_byte_ranges("abc", &[], PLAIN, Color32::RED), [(0..3, PLAIN)]);
        assert_eq!(colored_byte_ranges("", &[], PLAIN, Color32::RED), [(0..0, PLAIN)]);
    }


    #[test]
    fn red_segments_color_their_bytes_and_document_segments_keep_the_plain_color() {
        let segments = [(1..2, CharStyle::Red), (2..4, FROM_B)];
        assert_eq!(colored_byte_ranges("abcde", &segments, PLAIN, Color32::RED), [(0..1, PLAIN), (1..2, Color32::RED), (2..4, PLAIN), (4..5, PLAIN)]);
    }


    #[test]
    fn segment_columns_are_characters() {
        assert_eq!(colored_byte_ranges("äöü", &[(1..2, CharStyle::Red)], PLAIN, Color32::RED), [(0..2, PLAIN), (2..4, Color32::RED), (4..6, PLAIN)]);
    }


    #[test]
    fn segments_past_the_end_are_cut_off() {
        assert_eq!(colored_byte_ranges("ab", &[(1..9, CharStyle::Red), (9..12, FROM_B)], PLAIN, Color32::RED), [(0..1, PLAIN), (1..2, Color32::RED)]);
    }


    #[test]
    fn red_backgrounds_are_translucent_and_document_backgrounds_are_the_stronger_document_fill() {
        for palette in [&LIGHT, &DARK] {
            assert_eq!(char_background(&CharStyle::Red, palette), palette.red().gamma_multiply_u8(CHAR_DIFF_BACKGROUND_ALPHA));
        }
        for (palette, document) in [(&LIGHT, kdiff4_protocol::Document::A), (&LIGHT, kdiff4_protocol::Document::B), (&LIGHT, kdiff4_protocol::Document::C), (&DARK, kdiff4_protocol::Document::B)] {
            assert_eq!(char_background(&CharStyle::Document(document), palette), palette.colors(document).gutter_active);
        }
    }


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
    fn wheel_lines_scroll_by_the_step() {
        assert_eq!(wheel_pixels(egui::MouseWheelUnit::Line, 1.0, 10.0, 200.0, 3), 30.0);
        assert_eq!(wheel_pixels(egui::MouseWheelUnit::Line, -2.0, 10.0, 200.0, 1), -20.0);
        assert_eq!(wheel_pixels(egui::MouseWheelUnit::Line, 1.0, 10.0, 200.0, 0), 10.0);
        assert_eq!(wheel_pixels(egui::MouseWheelUnit::Point, 7.5, 10.0, 200.0, 3), 7.5);
        assert_eq!(wheel_pixels(egui::MouseWheelUnit::Page, -1.0, 10.0, 200.0, 3), -200.0);
    }


    #[test]
    fn viewport_knows_its_pages_and_scroll_range() {
        let viewport = Viewport { offset: 0.0, height: 55.0, row_height: 10.0, content_height: 300.0 };
        assert_eq!(viewport.page_rows(), 5);
        assert_eq!(viewport.max_offset(), 245.0);
        assert_eq!(Viewport { content_height: 20.0, ..viewport }.max_offset(), 0.0);
        assert_eq!(Viewport { row_height: 0.0, ..viewport }.page_rows(), 1);
    }


    #[test]
    fn viewport_shows_rows_that_fit_entirely() {
        let viewport = Viewport { offset: 100.0, height: 50.0, row_height: 10.0, content_height: 1000.0 };
        assert!(viewport.shows(10..15));
        assert!(viewport.shows(12..13));
        assert!(!viewport.shows(9..12));
        assert!(!viewport.shows(14..16));
        assert!(!viewport.shows(0..30));
        assert!(viewport.shows(15..15));
        assert!(!viewport.shows(16..16));
        assert!(Viewport { offset: 100.2, ..viewport }.shows(10..15));
    }


    #[test]
    fn region_boxes_span_all_rows_of_the_region() {
        let x_range = Rangef::new(0.0, 200.0);
        let rect = region_box(4..7, 5, 50.0, 10.0, x_range).unwrap();
        assert_eq!(rect, Rect::from_min_max(pos2(0.0, 40.0), pos2(200.0, 70.0)));
        assert_eq!(region_box(4..7, 4, 40.0, 10.0, x_range), Some(rect));
        assert_eq!(region_box(3..3, 3, 0.0, 10.0, x_range), None);
        assert_eq!(region_box(3..5, 7, 0.0, 10.0, x_range), None);
    }


    fn style(line_numbers: bool, margin: MarginKind) -> ViewStyle<'static> {
        ViewStyle { palette: &LIGHT, line_numbers, margin, current_region: None, current_region_color: Color32::TRANSPARENT, scroll_step_rows: 3, region_corner_radius: 0, other_region_border_opacity: 1.0, whitespace: WhitespaceConfig::default() }
    }


    #[test]
    fn margin_fits_the_largest_line_number() {
        assert_eq!(digits(0), 1);
        assert_eq!(digits(9), 1);
        assert_eq!(digits(10), 2);
        assert_eq!(digits(12345), 5);
        let numbers = margin_widths(&style(true, MarginKind::Nothing), 120, 7.0, 16.0);
        assert_eq!(numbers, Margin { numbers: 3.0 * 7.0 + MARGIN_PADDING * 2.0, column: 0.0 });
        assert_eq!(margin_widths(&style(false, MarginKind::Nothing), 1, 7.0, 16.0).total(), TEXT_GAP);
    }


    #[test]
    fn gutter_columns_are_one_row_wide() {
        assert_eq!(margin_widths(&style(false, MarginKind::Gutters), 120, 7.0, 16.0), Margin { numbers: 0.0, column: 16.0 });
        assert_eq!(margin_widths(&style(false, MarginKind::Marks), 1, 7.0, 16.0).column, 16.0);
        assert_eq!(margin_widths(&style(false, MarginKind::Marks), 1, 20.0, 16.0).column, 20.0 + MARGIN_PADDING * 2.0);
    }


    #[test]
    fn clicks_on_gutter_blocks_select_their_region() {
        let block = Some(Block { index: 0, origin: None, style: None });
        let map = RowMap::from_rows(vec![PaneRow { line: Some(0), region: Some(0), ..PaneRow::default() }, PaneRow { line: Some(1), region: Some(1), block, ..PaneRow::default() }, PaneRow { region: Some(1), block, ..PaneRow::default() }]);
        let rows = Rows::Mapped(&map);
        let margin = Margin { numbers: 20.0, column: 16.0 };
        assert_eq!(gutter_region(rows, margin, MarginKind::Gutters, 25.0, 20.0, 16.0), Some(1));
        assert_eq!(gutter_region(rows, margin, MarginKind::Marks, 20.0, 40.0, 16.0), Some(1));
        assert_eq!(gutter_region(rows, margin, MarginKind::Gutters, 25.0, 5.0, 16.0), None, "a row without a block");
        assert_eq!(gutter_region(rows, margin, MarginKind::Gutters, 19.0, 20.0, 16.0), None, "the line numbers");
        assert_eq!(gutter_region(rows, margin, MarginKind::Gutters, 36.0, 20.0, 16.0), None, "the text");
        assert_eq!(gutter_region(rows, margin, MarginKind::Gutters, 25.0, 60.0, 16.0), None, "below the last row");
        assert_eq!(gutter_region(rows, margin, MarginKind::Nothing, 25.0, 20.0, 16.0), None);
    }


    #[test]
    fn origins_take_their_document_colors() {
        let colors = |origin| origin_colors(&LIGHT, origin);
        assert_eq!(colors(Some(Mark::Source(kdiff4_protocol::Document::C))), &LIGHT.sources[2]);
        assert_eq!(colors(Some(Mark::Modified)), &LIGHT.output);
        assert_eq!(colors(None), &LIGHT.output);
    }


    #[test]
    fn marked_gutter_blocks_take_the_colors_of_their_style() {
        use kdiff4_protocol::Document;
        let block = |origin: Document, style| Block { index: 0, origin: Some(Mark::Source(origin)), style };
        let (fill, border) = gutter_colors(&LIGHT, block(Document::A, Some(FROM_B)), None);
        assert_eq!((fill, border), (&LIGHT.sources[1], &LIGHT.sources[1]));
        let (fill, border) = gutter_colors(&LIGHT, block(Document::B, Some(CharStyle::Red)), None);
        assert_eq!((fill, border), (&LIGHT.conflict, &LIGHT.conflict));
        let (fill, border) = gutter_colors(&DARK, block(Document::C, None), None);
        assert_eq!((fill, border), (&DARK.sources[2], &DARK.sources[2]));
    }


    #[test]
    fn output_gutter_blocks_keep_the_origin_colors_of_their_lines() {
        use kdiff4_protocol::Document;
        let block = Block { index: 0, origin: None, style: None };
        let (fill, border) = gutter_colors(&LIGHT, block, Some(Mark::Source(Document::B)));
        assert_eq!((fill, border), (&LIGHT.sources[1], &LIGHT.output));
        let (fill, border) = gutter_colors(&LIGHT, block, None);
        assert_eq!((fill, border), (&LIGHT.output, &LIGHT.output));
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


    #[test]
    fn whitespace_characters_are_classified() {
        assert_eq!(Whitespace::of(' '), Some(Whitespace::Space));
        assert_eq!(Whitespace::of('\t'), Some(Whitespace::Tab));
        assert_eq!(Whitespace::of('\n'), Some(Whitespace::LineFeed));
        assert_eq!(Whitespace::of('\r'), Some(Whitespace::CarriageReturn));
        assert_eq!(Whitespace::of('\u{a0}'), Some(Whitespace::Other));
        assert_eq!(Whitespace::of('\u{3000}'), Some(Whitespace::Other));
        assert_eq!(Whitespace::of('x'), None);
    }


    #[test]
    fn each_kind_follows_its_toggle_and_the_master_switch() {
        let all = WhitespaceConfig::default();
        let kinds = [Whitespace::Space, Whitespace::Tab, Whitespace::LineFeed, Whitespace::CarriageReturn, Whitespace::Other];
        assert!(kinds.iter().all(|kind| kind.is_shown(&all)));
        let off = WhitespaceConfig { render: false, ..all.clone() };
        assert!(kinds.iter().all(|kind| !kind.is_shown(&off)));
        let no_tabs = WhitespaceConfig { tabs: false, ..all.clone() };
        assert!(!Whitespace::Tab.is_shown(&no_tabs));
        assert!(Whitespace::Space.is_shown(&no_tabs));
    }


    #[test]
    fn terminators_are_marked_per_line() {
        let text = "a\r\nb\nc\rd";
        let ranges = [0..1, 3..4, 5..6, 7..8];
        let lines = Lines { text, ranges: &ranges };
        let all = WhitespaceConfig::default();
        assert_eq!(shown_terminator(lines, 0, &all), [Whitespace::CarriageReturn, Whitespace::LineFeed]);
        assert_eq!(shown_terminator(lines, 1, &all), [Whitespace::LineFeed]);
        assert_eq!(shown_terminator(lines, 2, &all), [Whitespace::CarriageReturn]);
        assert_eq!(shown_terminator(lines, 3, &all), []);
        let no_line_feeds = WhitespaceConfig { line_feeds: false, ..all.clone() };
        assert_eq!(shown_terminator(lines, 0, &no_line_feeds), [Whitespace::CarriageReturn]);
        let off = WhitespaceConfig { render: false, ..all };
        assert_eq!(shown_terminator(lines, 0, &off), []);
    }
}
