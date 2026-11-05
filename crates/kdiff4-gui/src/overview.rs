use std::sync::Arc;

use egui::epaint::text::cursor::CCursor;
use egui::{Align2, CursorIcon, FontId, Galley, Pos2, Rect, ScrollArea, Sense, Stroke, TextStyle, Ui, pos2, vec2};
use kdiff4_protocol::{Document, LineCharRuns};

use crate::char_diff;
use crate::editing::{Cursor, Lines, Position};
use crate::mirror::Mirror;
use crate::panes::{PaneAction, PaneState};
use crate::rows::{PaneRow, Rows};
use crate::text_view::{self, Caret, ViewStyle};
use crate::theme;


/// Space on each side of a document letter and of a line number.
const LABEL_PADDING: f32 = 4.0;
/// Space between the margin and the text.
const TEXT_GAP: f32 = 4.0;
/// Space left of the text, so a caret at its start is not cut off.
const TEXT_INSET: f32 = 2.0;
/// Height the pane frame adds around the rows: its border and the body's margin, above and below.
const FRAME_HEIGHT: f32 = 2.0 * (1.0 + 4.0);


/// Where the caret sits: a row of the source panes, which they all share, or a row of D.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sources(usize),
    Output(usize),
}


/// The rows the strip shows: the row of the source panes and the row of D, each `None` when nothing lines up there.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LensRows {
    pub sources: Option<usize>,
    pub output: Option<usize>,
}


/// The rows that line up with `focus`. The other side's row is the same distance into the same region, as with void space in the output pane.
pub fn lens_rows(sources: Rows<'_>, output: Option<Rows<'_>>, focus: Focus) -> LensRows {
    match focus {
        Focus::Sources(row) => LensRows { sources: (row < sources.len()).then_some(row), output: output.and_then(|output| corresponding_row(sources, row, output)) },
        Focus::Output(row) => LensRows { sources: output.and_then(|output| corresponding_row(output, row, sources)), output: output.filter(|output| row < output.len()).map(|_| row) },
    }
}


/// The row of `to` that lines up with row `row` of `from`: the same distance into the same region. `None` when the row has no region or the region is shorter in `to`.
pub fn corresponding_row(from: Rows<'_>, row: usize, to: Rows<'_>) -> Option<usize> {
    let region = from.region_of_row(row)?;
    let offset = row.checked_sub(from.region_rows(region)?.start)?;
    let rows = to.region_rows(region)?;
    (offset < rows.len()).then(|| rows.start + offset)
}


/// The height of a strip of `documents` lines.
pub fn height(ui: &Ui, documents: usize) -> f32 {
    documents as f32 * text_view::row_height(ui) + FRAME_HEIGHT
}


/// One line of the strip: a document, its lines, rows and character diff, and the row of it the strip shows.
#[derive(Clone, Copy)]
struct Lens<'a> {
    document: Document,
    lines: Lines<'a>,
    rows: Rows<'a>,
    char_runs: Option<&'a LineCharRuns>,
    row: Option<usize>,
}


impl Lens<'_> {
    fn pane_row(&self) -> Option<PaneRow> {
        self.row.and_then(|row| self.rows.row(row))
    }


    fn line(&self) -> Option<usize> {
        self.pane_row().and_then(|pane_row| pane_row.line)
    }
}


/// The lines and rows of `document` as its pane shows them.
fn view_of(mirror: &Mirror, document: Document) -> Option<(Lines<'_>, Rows<'_>)> {
    match document {
        Document::D => mirror.output().map(|output| (output.lines(), Rows::Mapped(mirror.output_rows()))),
        document => mirror.document(document.clone()).map(|text| {
            let lines = Lines { text: &text.text, ranges: &text.lines };
            let rows = mirror.rows(document).map_or(Rows::PerLine(lines.count()), Rows::Mapped);
            (lines, rows)
        }),
    }
}


/// The strip's lines: A, B and C, then D, on the rows that line up with the caret. Without a caret, the start of the current region, or the first row.
fn lenses<'a>(mirror: &'a Mirror, state: &PaneState) -> Vec<Lens<'a>> {
    let sources: Vec<(Document, Lines<'a>, Rows<'a>)> = mirror
        .documents()
        .iter()
        .filter_map(|text| view_of(mirror, text.document.clone()).map(|(lines, rows)| (text.document.clone(), lines, rows)))
        .collect();
    let output = view_of(mirror, Document::D);
    let Some(first) = sources.first().map(|(_, _, rows)| *rows) else {
        return Vec::new();
    };
    let caret_row = |rows: Rows<'_>, cursor: Cursor| text_view::caret_row(rows, Caret { cursor, void_row: state.void_row.clone() });
    let (reference, focus) = match state.caret {
        Some((Document::D, cursor)) => (first, output.map(|(_, rows)| Focus::Output(caret_row(rows, cursor)))),
        Some((document, cursor)) => match sources.iter().find(|(source, _, _)| *source == document) {
            Some((_, _, rows)) => (*rows, Some(Focus::Sources(caret_row(*rows, cursor)))),
            None => (first, None),
        },
        None => (first, None),
    };
    let focus = focus.unwrap_or_else(|| Focus::Sources(mirror.current_region().and_then(|region| first.region_start(region)).unwrap_or(0)));
    let rows = lens_rows(reference, output.map(|(_, rows)| rows), focus);
    let mut lenses: Vec<Lens<'a>> = sources
        .into_iter()
        .map(|(document, lines, source_rows)| Lens { document: document.clone(), lines, rows: source_rows, char_runs: mirror.char_diffs(document), row: rows.sources.filter(|row| *row < source_rows.len()) })
        .collect();
    if let Some((lines, output_rows)) = output {
        lenses.push(Lens { document: Document::D, lines, rows: output_rows, char_runs: None, row: rows.output });
    }
    lenses
}


/// The overview strip: per document, the line on the row that lines up with the caret, A, B and C with the same character diff as their panes, and D. Pressing, dragging, double- and triple-clicking work as in the panes and move the one caret, so D can be edited here. The lines scroll sideways together.
pub fn show(ui: &mut Ui, mirror: &Mirror, state: &PaneState, style: &ViewStyle<'_>) -> Vec<PaneAction> {
    let lenses = lenses(mirror, state);
    if lenses.is_empty() {
        return Vec::new();
    }
    let palette = style.palette;
    let radius = theme::corner_radius_of(ui);
    let sources = mirror.documents().len();
    theme::pane_frame(palette, radius)
        .show(ui, |ui| theme::pane_body_frame().show(ui, |ui| strip(ui, &lenses, sources, state, style)).inner)
        .inner
}


/// The margin of letters and line numbers, then the lines in a sideways scroll area, and what the mouse did over them.
fn strip(ui: &mut Ui, lenses: &[Lens<'_>], sources: usize, state: &PaneState, style: &ViewStyle<'_>) -> Vec<PaneAction> {
    let palette = style.palette;
    let font_id = TextStyle::Monospace.resolve(ui.style());
    let row_height = text_view::row_height(ui);
    let char_width = ui.painter().layout_no_wrap("0".to_string(), font_id.clone(), palette.pane_text).size().x;
    let letter_width = char_width + LABEL_PADDING * 2.0;
    let longest = lenses
        .iter()
        .map(|lens| lens.lines.count())
        .max()
        .unwrap_or(0);
    let number_width = if style.line_numbers { text_view::digits(longest) as f32 * char_width + LABEL_PADDING * 2.0 } else { 0.0 };
    let height = lenses.len() as f32 * row_height;
    ui.set_width(ui.available_width());
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    ui.horizontal(|ui| {
        let (margin, _) = ui.allocate_exact_size(vec2(letter_width + number_width + TEXT_GAP, height), Sense::hover());
        for (index, lens) in lenses.iter().enumerate() {
            let top = margin.top() + index as f32 * row_height;
            let letter = Rect::from_min_size(pos2(margin.left(), top), vec2(letter_width, row_height)).shrink(1.0);
            ui.painter().rect_filled(letter, 2.0, palette.colors(lens.document.clone()).header);
            ui.painter().text(letter.center(), Align2::CENTER_CENTER, lens.document.letter(), font_id.clone(), palette.text);
            if let Some(line) = lens.line()
                && number_width > 0.0
            {
                ui.painter().text(pos2(margin.left() + letter_width + number_width - LABEL_PADDING, top + row_height / 2.0), Align2::RIGHT_CENTER, (line + 1).to_string(), font_id.clone(), palette.weak_text);
            }
        }
        ui.painter().vline(margin.right() - TEXT_GAP / 2.0, margin.y_range(), Stroke::new(1.0, palette.void_hatch));
        let caret_id = ui.make_persistent_id("overview_strip_caret");
        let shown_caret = state.caret.filter(|_| !state.caret_hidden);
        let caret_moved = ui.data(|data| data.get_temp::<Option<(Document, Cursor)>>(caret_id)) != Some(shown_caret);
        ui.data_mut(|data| data.insert_temp(caret_id, shown_caret));
        let output = ScrollArea::horizontal()
            .id_salt("overview_strip")
            .auto_shrink([false, true])
            .show(ui, |ui| paint_lines(ui, lenses, sources, state, style, &font_id, row_height, char_width, caret_moved));
        pointer_actions(ui, lenses, &output.inner, output.inner_rect, output.state.clamped_offset().x, row_height)
    })
    .inner
}


/// Draws every line of the strip, one row each, with the char diff of a source line as its pane draws it; `sources` is how many source documents the rows have. Returns their laid-out text, `None` for a void or a placeholder. Scrolls the caret into view when it moved.
fn paint_lines(ui: &mut Ui, lenses: &[Lens<'_>], sources: usize, state: &PaneState, style: &ViewStyle<'_>, font_id: &FontId, row_height: f32, char_width: f32, caret_moved: bool) -> Vec<Option<Arc<Galley>>> {
    let palette = style.palette;
    let laid_out: Vec<(Option<Arc<Galley>>, Vec<(std::ops::Range<usize>, char_diff::CharStyle)>, Vec<text_view::Whitespace>)> = lenses
        .iter()
        .map(|lens| match lens.line() {
            Some(line) => {
                let segments = lens.char_runs.map(|runs| char_diff::segments(lens.document.clone(), runs.line(line), sources.clone())).unwrap_or_default();
                let galley = text_view::layout_styled(ui, lens.lines.line(line), &segments, font_id, palette, &style.char_diff);
                let terminator = text_view::shown_terminator(lens.lines, line, &style.whitespace);
                (Some(galley), segments, terminator)
            }
            None => (None, Vec::new(), Vec::new()),
        })
        .collect();
    let widest = laid_out
        .iter()
        .map(|(galley, _, terminator)| galley.as_ref().map_or(0.0, |galley| galley.size().x) + terminator.len() as f32 * char_width)
        .fold(0.0, f32::max);
    let width = (TEXT_INSET * 2.0 + widest + char_width).max(ui.available_width());
    let (content, _) = ui.allocate_exact_size(vec2(width, lenses.len() as f32 * row_height), Sense::hover());
    let painter = ui.painter().with_clip_rect(ui.clip_rect());
    let mut reveal = None;
    for (index, (lens, (galley, segments, terminator))) in lenses.iter().zip(&laid_out).enumerate() {
        let rect = Rect::from_min_size(pos2(content.left(), content.top() + index as f32 * row_height), vec2(width.max(ui.clip_rect().right() - content.left()), row_height));
        let text_rect = Rect::from_min_max(pos2(rect.left() + TEXT_INSET, rect.top()), rect.max);
        let caret = state.caret_in(lens.document.clone()).filter(|_| !state.caret_hidden);
        let void_row = state.void_row.filter(|row| lens.rows.row(*row).is_some_and(|pane_row| pane_row.line.is_none()));
        let (Some(line), Some(galley)) = (lens.line(), galley) else {
            if lens.pane_row().is_some_and(|pane_row| pane_row.conflict_marker) {
                painter.text(pos2(text_rect.left(), text_rect.center().y), Align2::LEFT_CENTER, text_view::CONFLICT_MARKER, font_id.clone(), palette.conflict_marker);
            } else {
                text_view::paint_void(&painter, text_rect, palette);
            }
            if caret.is_some() && void_row.is_some() && void_row == lens.row {
                text_view::paint_caret(&painter, text_rect.left(), rect, palette);
                reveal = Some(text_rect.left());
            }
            continue;
        };
        let text_origin = text_rect.min + vec2(0.0, (row_height - galley.size().y) / 2.0);
        text_view::paint_char_backgrounds(&painter, segments, galley, text_origin.x, rect, &style.char_diff);
        if let Some(cursor) = caret {
            text_view::paint_selection(&painter, lens.lines, line, text_rect, galley, cursor, row_height, palette);
        }
        text_view::paint_whitespace(&painter, lens.lines.line(line), terminator, galley, text_origin.x, rect, char_width, style);
        painter.galley(text_origin, galley.clone(), palette.pane_text);
        if let Some(cursor) = caret
            && void_row.is_none()
            && cursor.head.line == line
        {
            let x = text_origin.x + galley.pos_from_cursor(CCursor::new(cursor.head.column)).min.x;
            text_view::paint_caret(&painter, x, rect, palette);
            reveal = Some(x);
        }
    }
    if caret_moved && let Some(x) = reveal {
        ui.scroll_to_rect(Rect::from_min_max(pos2(x - char_width, content.top()), pos2(x + char_width, content.bottom())), None);
    }
    laid_out
        .into_iter()
        .map(|(galley, _, _)| galley)
        .collect()
}


/// Turns the mouse over the lines into pane actions on the document of the line it is over. A drag stays on the line it started on.
fn pointer_actions(ui: &mut Ui, lenses: &[Lens<'_>], galleys: &[Option<Arc<Galley>>], rect: Rect, scroll_x: f32, row_height: f32) -> Vec<PaneAction> {
    let response = ui.interact(rect, ui.make_persistent_id("overview_strip_pointer"), Sense::click_and_drag());
    let held = response.is_pointer_button_down_on();
    if response.hovered() || held {
        ui.ctx().set_cursor_icon(CursorIcon::Text);
    }
    let index_at = |pointer: Pos2| (((pointer.y - rect.top()) / row_height).floor().max(0.0) as usize).min(lenses.len().saturating_sub(1));
    let column_at = |index: usize, pointer: Pos2| {
        galleys
            .get(index)
            .and_then(Option::as_ref)
            .map_or(0, |galley| galley.cursor_from_pos(vec2(pointer.x - rect.left() + scroll_x - TEXT_INSET, galley.size().y / 2.0)).index.0)
    };
    let pressed = response.hovered() && ui.input(|input| input.pointer.primary_pressed());
    let press_id = ui.make_persistent_id("overview_strip_press");
    let Some(pointer) = ui.input(|input| input.pointer.interact_pos()) else {
        return Vec::new();
    };
    if pressed {
        ui.data_mut(|data| data.insert_temp(press_id, Some(index_at(pointer))));
    }
    let pressed_on = ui.data(|data| data.get_temp::<Option<usize>>(press_id)).flatten();
    let action = if response.triple_clicked() {
        let lens = &lenses[index_at(pointer)];
        lens.line().map(|line| PaneAction::SelectLine(lens.document.clone(), line))
    } else if response.double_clicked() {
        let index = index_at(pointer);
        let lens = &lenses[index];
        lens.line().map(|line| PaneAction::SelectWord(lens.document.clone(), Position { line, column: column_at(index, pointer) }))
    } else if pressed {
        let index = index_at(pointer);
        press(&lenses[index], column_at(index, pointer), ui.input(|input| input.modifiers.shift))
    } else if held && let Some(index) = pressed_on {
        let lens = &lenses[index.min(lenses.len().saturating_sub(1))];
        lens.line().map(|line| PaneAction::Drag { document: lens.document.clone(), position: Position { line, column: column_at(index, pointer) }, void_row: None })
    } else {
        None
    };
    action.into_iter().collect()
}


/// A press on a lens at `column`: on a line, the caret goes there; on a void or a placeholder, onto that row of the pane, as a click there would. A lens with no row takes no press.
fn press(lens: &Lens<'_>, column: usize, extend: bool) -> Option<PaneAction> {
    let row = lens.row?;
    let last_line = lens.lines.count().saturating_sub(1);
    let end = Position { line: last_line, column: lens.lines.line_chars(last_line) };
    let (position, void_row) = match lens.line() {
        Some(line) => (Position { line, column }, None),
        None => lens.rows.caret_at_row(row, 0, end),
    };
    Some(PaneAction::Press { document: lens.document.clone(), position, void_row, extend })
}


#[cfg(test)]
mod tests {
    use std::ops::Range;
    use std::path::PathBuf;

    use kdiff4_protocol::engine::{CharRun, Comparison, ComparisonOptions, EngineCapabilities, HunkChange, Region, RegionStatus, Row};
    use kdiff4_protocol::{Choice, DocumentText, EolStyle, OutputDocument, RegionMerge, Revision, Snapshot};

    use egui::{Color32, Event, Modifiers, PointerButton};
    use egui_kittest::Harness;

    use super::*;
    use crate::char_diff::CharDiffLook;
    use crate::config::WhitespaceConfig;
    use crate::rows::{self, RowMap};
    use crate::text_view::MarginKind;
    use crate::theme::LIGHT;


    fn region(status: HunkChange, a: Range<usize>, b: Range<usize>, rows: &[(Option<usize>, Option<usize>)]) -> Region {
        Region { hunks: vec![a, b], status: RegionStatus::Diff(status), rows: rows.iter().map(|(a, b)| Row { lines: vec![a.clone(), b.clone()] }).collect() }
    }


    /// A and B share two lines, then A has three lines where B has one, then one more shared line.
    fn comparison() -> Comparison {
        Comparison {
            regions: vec![
                region(HunkChange::Unchanged, 0..2, 0..2, &[(Some(0), Some(0)), (Some(1), Some(1))]),
                region(HunkChange::Modified, 2..5, 2..3, &[(Some(2), Some(2)), (Some(3), None), (Some(4), None)]),
                region(HunkChange::Unchanged, 5..6, 3..4, &[(Some(5), Some(3))]),
            ],
        }
    }


    fn chosen(document: Document, lines: usize) -> RegionMerge {
        RegionMerge { choice: Choice::Hunks(vec![document]), edited: false, output_lines: lines }
    }


    /// D takes the first region from A, the one line B has in the second region, and the last region: four lines.
    fn merge() -> Vec<RegionMerge> {
        vec![chosen(Document::A, 2), chosen(Document::B, 1), chosen(Document::A, 1)]
    }


    #[test]
    fn source_rows_line_up_with_d_by_their_place_in_the_region() {
        let comparison = comparison();
        let source = RowMap::for_source(&comparison, &merge(), 0, 6, None);
        let output = RowMap::for_output(&merge(), &comparison.regions, 4, None);
        let (source, output) = (Rows::Mapped(&source), Rows::Mapped(&output));
        assert_eq!(corresponding_row(source, 1, output), Some(1));
        assert_eq!(corresponding_row(source, 2, output), Some(2));
        assert_eq!(corresponding_row(source, 3, output), None);
        assert_eq!(corresponding_row(source, 5, output), Some(3));
        assert_eq!(corresponding_row(output, 2, source), Some(2));
        assert_eq!(corresponding_row(output, 3, source), Some(5));
        assert_eq!(corresponding_row(source, 9, output), None);
    }


    #[test]
    fn with_void_space_d_has_a_row_for_every_source_row() {
        let comparison = comparison();
        let heights = rows::aligned_heights(&comparison.regions, &merge());
        let source = RowMap::for_source(&comparison, &merge(), 0, 6, Some(&heights));
        let output = RowMap::for_output(&merge(), &comparison.regions, 4, Some(&heights));
        let (source, output) = (Rows::Mapped(&source), Rows::Mapped(&output));
        for row in 0..6 {
            assert_eq!(corresponding_row(source, row, output), Some(row));
        }
        assert_eq!(output.row(3).and_then(|row| row.line), None);
    }


    #[test]
    fn lens_rows_follow_the_pane_with_the_caret() {
        let comparison = comparison();
        let source = RowMap::for_source(&comparison, &merge(), 0, 6, None);
        let output = RowMap::for_output(&merge(), &comparison.regions, 4, None);
        let (source, output) = (Rows::Mapped(&source), Rows::Mapped(&output));
        assert_eq!(lens_rows(source, Some(output), Focus::Sources(2)), LensRows { sources: Some(2), output: Some(2) });
        assert_eq!(lens_rows(source, Some(output), Focus::Sources(4)), LensRows { sources: Some(4), output: None });
        assert_eq!(lens_rows(source, Some(output), Focus::Output(3)), LensRows { sources: Some(5), output: Some(3) });
        assert_eq!(lens_rows(source, Some(output), Focus::Output(7)), LensRows { sources: None, output: None });
        assert_eq!(lens_rows(source, None, Focus::Sources(4)), LensRows { sources: Some(4), output: None });
        assert_eq!(lens_rows(source, None, Focus::Sources(6)), LensRows { sources: None, output: None });
    }


    fn text(document: Document, lines: &[&str]) -> DocumentText {
        let text = lines.join("\n");
        let mut ranges = Vec::new();
        let mut start = 0;
        for line in lines {
            ranges.push(start..start + line.len());
            start += line.len() + 1;
        }
        DocumentText { document, path: PathBuf::from(format!("{}.txt", document.letter())), label: None, eol: EolStyle::Lf, text, lines: ranges }
    }


    fn char_runs(lines: usize) -> LineCharRuns {
        LineCharRuns { lines: vec![vec![CharRun { columns: 0..1, same_in: vec![true, false] }]; lines] }
    }


    fn mirror(current_region: Option<usize>) -> Mirror {
        mirror_with_a3("a3", current_region)
    }


    /// The merge of `comparison` and `merge`, with `a3` as A's fourth line.
    fn mirror_with_a3(a3: &str, current_region: Option<usize>) -> Mirror {
        let output = OutputDocument { path: None, text: "s0\ns1\nb2\ns3".to_string(), modified: false };
        let documents = vec![text(Document::A, &["s0", "s1", "a2", a3, "a4", "s3"]), text(Document::B, &["s0", "s1", "b2", "s3"])];
        let snapshot = Snapshot { revision: Revision(1), documents, output: Some(output), comparison: comparison(), engine: EngineCapabilities::default(), merge: merge(), options: ComparisonOptions::default(), current_region, char_diffs: vec![char_runs(6), char_runs(4)] };
        Mirror::from_snapshot(snapshot, false)
    }


    fn shown(mirror: &Mirror, state: &PaneState) -> Vec<(Document, Option<usize>, Option<usize>)> {
        lenses(mirror, state)
            .iter()
            .map(|lens| (lens.document.clone(), lens.row, lens.line()))
            .collect()
    }


    #[test]
    fn strip_shows_every_document_on_the_caret_row() {
        let mirror = mirror(None);
        let mut state = PaneState::default();
        state.set_caret(Document::A, Cursor::at(Position { line: 3, column: 1 }), None);
        assert_eq!(shown(&mirror, &state), [(Document::A, Some(3), Some(3)), (Document::B, Some(3), None), (Document::D, None, None)]);
        state.set_caret(Document::D, Cursor::at(Position { line: 2, column: 0 }), None);
        assert_eq!(shown(&mirror, &state), [(Document::A, Some(2), Some(2)), (Document::B, Some(2), Some(2)), (Document::D, Some(2), Some(2))]);
        state.set_caret(Document::B, Cursor::at(Position { line: 3, column: 0 }), Some(4));
        assert_eq!(shown(&mirror, &state), [(Document::A, Some(4), Some(4)), (Document::B, Some(4), None), (Document::D, None, None)]);
    }


    #[test]
    fn strip_follows_a_jump_and_starts_at_the_current_region() {
        let mut state = PaneState::default();
        assert_eq!(shown(&mirror(None), &state)[0], (Document::A, Some(0), Some(0)));
        assert_eq!(shown(&mirror(Some(2)), &state)[0], (Document::A, Some(5), Some(5)));
        state.set_hidden_caret(Document::A, Cursor::at(Position { line: 2, column: 0 }), None);
        assert_eq!(shown(&mirror(None), &state)[2], (Document::D, Some(2), Some(2)));
    }


    #[test]
    fn source_lines_carry_their_char_diff_and_d_none() {
        let mirror = mirror(None);
        let lenses = lenses(&mirror, &PaneState::default());
        assert_eq!(lenses[0].char_runs, mirror.char_diffs(Document::A));
        assert_eq!(lenses[1].char_runs, mirror.char_diffs(Document::B));
        assert_eq!(lenses[2].char_runs, None);
    }


    #[test]
    fn presses_move_the_caret_into_the_lens_document() {
        let mirror = mirror(None);
        let mut state = PaneState::default();
        state.set_caret(Document::A, Cursor::at(Position { line: 3, column: 0 }), None);
        let lenses = lenses(&mirror, &state);
        assert_eq!(press(&lenses[0], 2, true), Some(PaneAction::Press { document: Document::A, position: Position { line: 3, column: 2 }, void_row: None, extend: true }));
        assert_eq!(press(&lenses[1], 2, false), Some(PaneAction::Press { document: Document::B, position: Position { line: 3, column: 0 }, void_row: Some(3), extend: false }));
        assert_eq!(press(&lenses[2], 2, false), None);
    }


    const LOOK: CharDiffLook = CharDiffLook { red: Color32::RED, blue: Color32::BLUE, green: Color32::GREEN };


    /// The strip in a test harness, and what its frames did: the actions it returned, the rect it took, the height `height` reserves for it, and its row height and line count.
    struct StripTest {
        mirror: Mirror,
        panes: PaneState,
        actions: Vec<PaneAction>,
        rect: Rect,
        height: f32,
        row_height: f32,
        lines: usize,
    }


    fn harness(mirror: Mirror, panes: PaneState) -> Harness<'static, StripTest> {
        let test = StripTest { mirror, panes, actions: Vec::new(), rect: Rect::NOTHING, height: 0.0, row_height: 0.0, lines: 0 };
        let mut harness = Harness::builder()
            .with_size(vec2(400.0, 200.0))
            .with_step_dt(0.01)
            .build_ui_state(
                |ui, test: &mut StripTest| {
                    let style = ViewStyle { palette: &LIGHT, line_numbers: true, margin: MarginKind::Nothing, current_region: None, current_region_color: Color32::TRANSPARENT, scroll_step_rows: 3, region_corner_radius: 0, other_region_border_opacity: 1.0, char_diff: LOOK, whitespace: WhitespaceConfig::default() };
                    let shown = ui.scope(|ui| show(ui, &test.mirror, &test.panes, &style));
                    test.actions.extend(shown.inner);
                    test.rect = shown.response.rect;
                    test.lines = lenses(&test.mirror, &test.panes).len();
                    test.height = height(ui, test.lines.clone());
                    test.row_height = text_view::row_height(ui);
                },
                test,
            );
        harness.run_steps(4);
        harness
    }


    /// A point near the right edge of the strip on its line `index`: past the end of a short line.
    fn right_of(test: &StripTest, index: usize) -> Pos2 {
        let top = test.rect.center().y - test.lines as f32 * test.row_height / 2.0;
        pos2(test.rect.right() - 20.0, top + (index as f32 + 0.5) * test.row_height)
    }


    /// Clicks `times` times at `pos` with `modifiers` held, then waits out the double-click delay, and returns the actions the strip returned meanwhile.
    fn click(harness: &mut Harness<'_, StripTest>, pos: Pos2, times: usize, modifiers: Modifiers) -> Vec<PaneAction> {
        harness.state_mut().actions.clear();
        harness.hover_at(pos);
        harness.event(Event::ModifiersChanged(modifiers));
        for _ in 0..times {
            harness.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: modifiers.clone() });
            harness.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: modifiers.clone() });
        }
        harness.event(Event::ModifiersChanged(Modifiers::NONE));
        harness.step();
        harness.run_steps(50);
        std::mem::take(&mut harness.state_mut().actions)
    }


    /// The column of a single press on A's fourth line.
    fn pressed_column(actions: &[PaneAction]) -> Option<usize> {
        match actions {
            [PaneAction::Press { document: Document::A, position: Position { line: 3, column }, .. }] => Some(column.clone()),
            _ => None,
        }
    }


    fn caret_on_a3(column: usize) -> PaneState {
        let mut panes = PaneState::default();
        panes.set_caret(Document::A, Cursor::at(Position { line: 3, column }), None);
        panes
    }


    #[test]
    fn drawn_strip_takes_the_height_reserved_for_it() {
        let harness = harness(mirror(None), PaneState::default());
        let test = harness.state();
        assert_eq!(test.lines, 3);
        assert_eq!(test.rect.height(), test.height);
    }


    #[test]
    fn clicking_a_line_presses_its_document() {
        let mut harness = harness(mirror(None), caret_on_a3(0));
        let (a, b, d) = (right_of(harness.state(), 0), right_of(harness.state(), 1), right_of(harness.state(), 2));
        assert_eq!(click(&mut harness, a, 1, Modifiers::NONE), [PaneAction::Press { document: Document::A, position: Position { line: 3, column: 2 }, void_row: None, extend: false }]);
        assert_eq!(click(&mut harness, b, 1, Modifiers::NONE), [PaneAction::Press { document: Document::B, position: Position { line: 3, column: 0 }, void_row: Some(3), extend: false }]);
        assert!(click(&mut harness, d, 1, Modifiers::NONE).is_empty(), "D has no row that lines up with A's fourth line");
        assert_eq!(click(&mut harness, a, 1, Modifiers::SHIFT), [PaneAction::Press { document: Document::A, position: Position { line: 3, column: 2 }, void_row: None, extend: true }]);
    }


    #[test]
    fn double_click_selects_a_word_and_triple_click_a_line() {
        let mut harness = harness(mirror(None), caret_on_a3(0));
        let a = right_of(harness.state(), 0);
        assert_eq!(click(&mut harness, a, 2, Modifiers::NONE).last(), Some(&PaneAction::SelectWord(Document::A, Position { line: 3, column: 2 })));
        assert_eq!(click(&mut harness, a, 3, Modifiers::NONE).last(), Some(&PaneAction::SelectLine(Document::A, 3)));
    }


    #[test]
    fn drag_stays_on_the_line_it_started_on() {
        let mut panes = PaneState::default();
        panes.set_caret(Document::D, Cursor::at(Position { line: 2, column: 0 }), None);
        let mut harness = harness(mirror(None), panes);
        let (a, b) = (right_of(harness.state(), 0), right_of(harness.state(), 1));
        harness.hover_at(a);
        harness.drag_at(a);
        harness.hover_at(b);
        harness.step();
        let actions = std::mem::take(&mut harness.state_mut().actions);
        assert_eq!(actions.first(), Some(&PaneAction::Press { document: Document::A, position: Position { line: 2, column: 2 }, void_row: None, extend: false }));
        assert_eq!(actions.last(), Some(&PaneAction::Drag { document: Document::A, position: Position { line: 2, column: 2 }, void_row: None }));
        harness.drop_at(b);
        harness.step();
        assert!(harness.state().actions.is_empty());
    }


    #[test]
    fn strip_scrolls_sideways_to_the_caret_when_it_moves() {
        let long = format!("a3{}", "x".repeat(300));
        let mut harness = harness(mirror_with_a3(&long, None), caret_on_a3(0));
        let a = right_of(harness.state(), 0);
        let column = pressed_column(&click(&mut harness, a, 1, Modifiers::NONE)).unwrap();
        assert!(column < 100, "the start of the line is in view, column {column} is under the pointer");
        harness.state_mut().panes = caret_on_a3(302);
        harness.run_steps(50);
        let column = pressed_column(&click(&mut harness, a, 1, Modifiers::NONE)).unwrap();
        assert!(column > 250, "the caret at the end of the line is in view, column {column} is under the pointer");
    }
}
