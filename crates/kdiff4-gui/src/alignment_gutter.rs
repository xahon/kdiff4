use std::ops::Range;

use egui::{Align2, Color32, FontId, Painter, Rect, Shape, Stroke, StrokeKind, pos2};
use kdiff4_protocol::{AlignmentMark, Document, InsetChoice, InsetOption, MarkStatus};

use crate::rows::Rows;
use crate::text_view;
use crate::theme::{DocumentColors, Palette};

/// How much of a row the chip of an anchor covers.
const CHIP_HEIGHT_SHARE: f32 = 0.8;
/// The strip of arrows left of the line numbers: the padding before the first lane, the width of an arrow's line, and the run after the lanes where an arrow ends in a tip or a dot.
const ARROW_EDGE: f32 = 2.0;
const ARROW_WIDTH: f32 = 1.5;
const ARROW_RUN: f32 = 6.5;
const ARROW_HEAD_LENGTH: f32 = 4.5;
const ARROW_HEAD_WIDTH: f32 = 5.0;
const ARROW_DOT_RADIUS: f32 = 2.5;
/// The border of a mark nothing in the other documents pairs with, as a share of the color's strength.
const UNMATCHED_BORDER_SHARE: f32 = 0.45;

/// The alignment marks of one source pane, and the highest number any document uses, so a mark can be renumbered to pair with a mark of another document. `insets` are the places the alignments that some document has in another place could be shown at.
#[derive(Debug, Clone, Copy)]
pub struct PaneMarks<'a> {
    pub marks: &'a [AlignmentMark],
    pub highest: u32,
    pub insets: &'a [InsetChoice],
}

/// Where a mark sits in a pane's rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// The rows from the mark's first line to its last, void rows between them included.
    Rows(Range<usize>),
    /// The edge above this row; one past the last row is the end of the text.
    Edge(usize),
}

pub fn place(mark: &AlignmentMark, rows: Rows<'_>, line_count: usize) -> Place {
    if mark.is_anchor() {
        let edge = if mark.lines.start >= line_count {
            rows.len()
        } else {
            rows.row_of_line(mark.lines.start)
        };
        return Place::Edge(edge);
    }
    let first = rows.row_of_line(mark.lines.start);
    let last = rows.row_of_line(mark.lines.end - 1);
    Place::Rows(first..last.max(first) + 1)
}

/// A hunk shown away from its place: the arrow runs from where the hunk was in the document to the rows it is shown on. Both ends are in rows, counted from the top of the text: a row's middle is its number plus a half, an edge between rows is a whole number.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Arrow {
    /// Where the hunk was: the first row of the hunk that is shown there now.
    pub from: f32,
    /// The first row of the hunk.
    pub to: f32,
}

/// The arrow of a mark whose hunk the applied comparison shows elsewhere, given the marks of its pane and where they sit. The old place of the hunk shows the hunk of the mark `replaced_by` names. Anchors have no lines to move, and a mark that is not applied yet is not where the rows show it.
pub fn arrow(mark: &AlignmentMark, marks: &[AlignmentMark], places: &[Place]) -> Option<Arrow> {
    let replaced_by = mark.replaced_by?;
    if mark.status != MarkStatus::Applied || mark.is_anchor() {
        return None;
    }
    let position_of = |number: u32| marks.iter().position(|other| other.number == number && other.status == MarkStatus::Applied);
    let Some(Place::Rows(shown)) = position_of(mark.number).and_then(|index| places.get(index)) else {
        return None;
    };
    let from = match position_of(replaced_by).and_then(|index| places.get(index))? {
        Place::Rows(rows) => rows.start as f32 + 0.5,
        Place::Edge(edge) => *edge as f32,
    };
    let to = shown.start as f32 + 0.5;
    ((from - to).abs() >= 1.0).then_some(Arrow { from, to })
}

/// The lane of every arrow, counting from 0 next to the line numbers: arrows that overlap in height take different lanes, the others share one. The arrows that begin higher up take the lower lanes.
pub fn assign_lanes(arrows: &[Arrow]) -> Vec<usize> {
    let span = |arrow: &Arrow| (arrow.from.min(arrow.to), arrow.from.max(arrow.to));
    let mut order: Vec<usize> = (0..arrows.len()).collect();
    order.sort_by(|first, second| span(&arrows[*first]).0.total_cmp(&span(&arrows[*second]).0));
    let mut lanes = vec![0; arrows.len()];
    let mut ends: Vec<f32> = Vec::new();
    for index in order {
        let (start, end) = span(&arrows[index]);
        let lane = ends.iter().position(|lane_end| *lane_end < start).unwrap_or(ends.len());
        match ends.get_mut(lane) {
            Some(lane_end) => *lane_end = end,
            None => ends.push(end),
        }
        lanes[index] = lane;
    }
    lanes
}

/// The width of the strip left of the line numbers that holds `lanes` arrows with `gap` pixels between their lines; nothing when there are no arrows.
pub fn arrows_width(lanes: usize, gap: f32) -> f32 {
    if lanes == 0 {
        return 0.0;
    }
    ARROW_EDGE + lanes as f32 * ARROW_WIDTH + (lanes - 1) as f32 * gap.max(0.0) + ARROW_RUN
}

/// The marks under a point of the alignment column, given in pixels from the top of the content: the ranges that cover its row and the anchors whose chip it touches.
pub fn hits(places: &[Place], row_height: f32, content_y: f32) -> Vec<usize> {
    if row_height <= 0.0 || content_y < 0.0 {
        return Vec::new();
    }
    let row = (content_y / row_height).floor() as usize;
    let half_chip = row_height * CHIP_HEIGHT_SHARE / 2.0;
    places
        .iter()
        .enumerate()
        .filter(|(_, place)| match place {
            Place::Rows(rows) => rows.contains(&row),
            Place::Edge(edge) => (content_y - *edge as f32 * row_height).abs() <= half_chip,
        })
        .map(|(index, _)| index)
        .collect()
}

/// What a mark says when the pointer rests on it.
pub fn describe(mark: &AlignmentMark) -> String {
    let what = if mark.is_anchor() {
        format!("anchor before line {}", mark.lines.start + 1)
    } else {
        format!("lines {} to {}", mark.lines.start + 1, mark.lines.end)
    };
    let state = match mark.status {
        MarkStatus::Applied => "applied",
        MarkStatus::Pending => "waits for Apply Alignments",
        MarkStatus::Unmatched => "has no effect: not every document has a mark with this number",
    };
    let moved = if mark.is_moved() { ". It is moved: shown with the other documents' parts of this alignment, away from its place here." } else { "" };
    format!("Alignment {}, {what}: {state}{moved}", mark.number)
}

/// Where an option shows its alignment among the others, and the documents whose hunk it moves there.
pub fn option_label(option: &InsetOption) -> String {
    let place = match (option.above, option.below) {
        (None, Some(below)) => format!("Above alignment {below}"),
        (Some(above), None) => format!("Below alignment {above}"),
        (Some(above), Some(below)) => format!("Between alignments {above} and {below}"),
        (None, None) => "Here".to_string(),
    };
    let letters: Vec<&str> = option.moved.iter().map(Document::letter).collect();
    let moves = match letters.as_slice() {
        [] => return place,
        [only] => only.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    format!("{place} (moves {moves})")
}

/// The numbers a mark of a pane can be renumbered to: every number up to one past the highest, except its own. The bool is whether another mark of the pane has the number, so the two swap.
pub fn renumber_choices(marks: &[AlignmentMark], highest: u32, own: u32) -> Vec<(u32, bool)> {
    (1..=highest + 1)
        .filter(|number| *number != own)
        .map(|number| (number, marks.iter().any(|mark| mark.number == number)))
        .collect()
}

/// The fill and the border of a mark by its state: applied marks are filled strongly, marks waiting for Apply lightly, and marks without a partner are only outlined, faintly.
fn colors(status: MarkStatus, color: Color32, palette: &Palette) -> (DocumentColors, bool) {
    let derived = DocumentColors::derive(color, palette.pane_bg);
    match status {
        MarkStatus::Applied => (derived, true),
        MarkStatus::Pending => (derived, false),
        MarkStatus::Unmatched => (
            DocumentColors {
                header: Color32::TRANSPARENT,
                gutter_border: derived.gutter_border.gamma_multiply(UNMATCHED_BORDER_SHARE),
                gutter_active: Color32::TRANSPARENT,
            },
            false,
        ),
    }
}

/// Paints what the alignment column shows on `row`: the slice of every range mark that covers it, with the number on its first row, and the chip of every anchor on the edge above it, or below it when it is the last row and the anchor is at the end of the text.
pub fn paint_row(
    painter: &Painter,
    marks: &[AlignmentMark],
    places: &[Place],
    row: usize,
    row_count: usize,
    rect: Rect,
    column_left: f32,
    column_width: f32,
    color: Color32,
    palette: &Palette,
    font_id: &FontId,
) {
    for (mark, place) in marks.iter().zip(places) {
        let Place::Rows(rows) = place else { continue };
        if !rows.contains(&row) {
            continue;
        }
        let (set, active) = colors(mark.status, color, palette);
        text_view::paint_gutter_block(painter, column_left, column_width, rect, row == rows.start, row + 1 == rows.end, &set, &set, active);
        if row == rows.start {
            painter.text(
                pos2(column_left + column_width / 2.0, rect.center().y),
                Align2::CENTER_CENTER,
                mark.number.to_string(),
                font_id.clone(),
                palette.text,
            );
        }
    }
    paint_chips(
        painter,
        marks,
        places,
        rect.top(),
        rect,
        column_left,
        column_width,
        color,
        palette,
        font_id,
        Place::Edge(row),
    );
    if row + 1 == row_count {
        paint_chips(
            painter,
            marks,
            places,
            rect.bottom(),
            rect,
            column_left,
            column_width,
            color,
            palette,
            font_id,
            Place::Edge(row_count),
        );
    }
}

/// The part of every arrow on `row`, in the strip from `left` to `left + width`, like a branch in a graph: a line down its lane, a dot at the right end of the strip, next to the line numbers, where the hunk was taken from, and a tip there pointing at the first row of the hunk. `lanes` has the lane of every arrow, and `gap` is the space between lanes.
pub fn paint_arrows(painter: &Painter, arrows: &[Arrow], lanes: &[usize], row: usize, row_count: usize, rect: Rect, left: f32, width: f32, gap: f32, color: Color32) {
    let lane_count = lanes.iter().max().map_or(0, |lane| lane + 1);
    let pitch = ARROW_WIDTH + gap.max(0.0);
    let end_x = left + width - 1.0;
    let screen_y = |position: f32| rect.top() + (position - row as f32) * rect.height();
    let on_row = |y: f32| y >= rect.top() && (y < rect.bottom() || (row + 1 == row_count && y <= rect.bottom()));
    let stroke = Stroke::new(ARROW_WIDTH, color);
    for (arrow, lane) in arrows.iter().zip(lanes) {
        let x = left + ARROW_EDGE + ARROW_WIDTH / 2.0 + (lane_count - 1 - lane) as f32 * pitch;
        let (from_y, to_y) = (screen_y(arrow.from), screen_y(arrow.to));
        let (top, bottom) = (from_y.min(to_y).max(rect.top()), from_y.max(to_y).min(rect.bottom()));
        if top < bottom {
            painter.line_segment([pos2(x, top), pos2(x, bottom)], stroke);
        }
        if on_row(from_y) {
            let dot = pos2(end_x - ARROW_DOT_RADIUS, from_y);
            painter.line_segment([pos2(x, from_y), dot], stroke);
            painter.circle_filled(dot, ARROW_DOT_RADIUS, color);
        }
        if on_row(to_y) {
            let base = end_x - ARROW_HEAD_LENGTH;
            painter.line_segment([pos2(x, to_y), pos2(base, to_y)], stroke);
            let head = vec![pos2(end_x, to_y), pos2(base, to_y - ARROW_HEAD_WIDTH / 2.0), pos2(base, to_y + ARROW_HEAD_WIDTH / 2.0)];
            painter.add(Shape::convex_polygon(head, color, Stroke::NONE));
        }
    }
}

/// One chip on the edge `edge` at height `y`, labeled with the numbers of the anchors there, lowest first.
fn paint_chips(
    painter: &Painter,
    marks: &[AlignmentMark],
    places: &[Place],
    y: f32,
    rect: Rect,
    column_left: f32,
    column_width: f32,
    color: Color32,
    palette: &Palette,
    font_id: &FontId,
    edge: Place,
) {
    let mut anchors: Vec<&AlignmentMark> = marks.iter().zip(places).filter(|(_, place)| **place == edge).map(|(mark, _)| mark).collect();
    anchors.sort_by_key(|mark| mark.number);
    let Some(first) = anchors.first() else { return };
    let status = if anchors.iter().any(|mark| mark.status == MarkStatus::Applied) {
        MarkStatus::Applied
    } else {
        first.status
    };
    let (set, active) = colors(status, color, palette);
    let half = rect.height() * CHIP_HEIGHT_SHARE / 2.0;
    let chip = Rect::from_min_max(pos2(column_left, y - half), pos2(column_left + column_width, y + half));
    painter.rect_filled(chip, 1.0, if active { set.gutter_active } else { set.header });
    painter.rect_stroke(chip, 1.0, Stroke::new(1.0, set.gutter_border), StrokeKind::Inside);
    painter.hline(chip.x_range(), y, Stroke::new(1.0, set.gutter_border));
    let label = anchors.iter().map(|mark| mark.number.to_string()).collect::<Vec<_>>().join(",");
    painter.text(chip.center(), Align2::CENTER_CENTER, label, font_id.clone(), palette.text);
}

#[cfg(test)]
mod tests {
    use kdiff4_protocol::Document;

    use super::*;
    use crate::rows::{PaneRow, RowMap};

    fn mark(number: u32, lines: Range<usize>, status: MarkStatus) -> AlignmentMark {
        AlignmentMark {
            document: Document::B,
            number,
            lines,
            status,
            replaced_by: None,
        }
    }

    /// Lines 0 and 1, then a void, then lines 2 and 3.
    fn rows_with_a_void() -> RowMap {
        RowMap::from_rows(vec![
            PaneRow {
                line: Some(0),
                ..PaneRow::default()
            },
            PaneRow {
                line: Some(1),
                ..PaneRow::default()
            },
            PaneRow::default(),
            PaneRow {
                line: Some(2),
                ..PaneRow::default()
            },
            PaneRow {
                line: Some(3),
                ..PaneRow::default()
            },
        ])
    }

    #[test]
    fn a_range_covers_the_rows_from_its_first_line_to_its_last_void_rows_included() {
        let map = rows_with_a_void();
        let rows = Rows::Mapped(&map);
        assert_eq!(place(&mark(1, 0..1, MarkStatus::Pending), rows, 4), Place::Rows(0..1));
        assert_eq!(place(&mark(1, 1..3, MarkStatus::Pending), rows, 4), Place::Rows(1..4));
        assert_eq!(place(&mark(1, 2..4, MarkStatus::Pending), rows, 4), Place::Rows(3..5));
    }

    #[test]
    fn an_anchor_sits_on_the_edge_above_its_line_or_below_the_text() {
        let map = rows_with_a_void();
        let rows = Rows::Mapped(&map);
        assert_eq!(place(&mark(1, 0..0, MarkStatus::Pending), rows, 4), Place::Edge(0));
        assert_eq!(place(&mark(1, 2..2, MarkStatus::Pending), rows, 4), Place::Edge(3));
        assert_eq!(place(&mark(1, 4..4, MarkStatus::Pending), rows, 4), Place::Edge(5));
        assert_eq!(place(&mark(1, 2..2, MarkStatus::Pending), Rows::PerLine(4), 4), Place::Edge(2));
        assert_eq!(place(&mark(1, 4..4, MarkStatus::Pending), Rows::PerLine(4), 4), Place::Edge(4));
    }

    #[test]
    fn the_pointer_hits_ranges_by_row_and_anchors_by_their_chip() {
        let places = [Place::Rows(1..3), Place::Edge(5), Place::Edge(5), Place::Edge(7)];
        assert_eq!(hits(&places, 10.0, 15.0), [0]);
        assert_eq!(hits(&places, 10.0, 29.9), [0]);
        assert!(hits(&places, 10.0, 30.1).is_empty());
        assert_eq!(hits(&places, 10.0, 50.0), [1, 2]);
        assert_eq!(hits(&places, 10.0, 53.9), [1, 2]);
        assert!(hits(&places, 10.0, 54.1).is_empty());
        assert_eq!(hits(&places, 10.0, 46.5), [1, 2]);
        assert!(hits(&places, 10.0, -1.0).is_empty());
        assert!(hits(&places, 0.0, 5.0).is_empty());
    }

    #[test]
    fn renumbering_offers_every_number_up_to_one_past_the_highest() {
        let marks = [mark(1, 0..1, MarkStatus::Pending), mark(3, 2..3, MarkStatus::Pending)];
        assert_eq!(renumber_choices(&marks, 3, 1), [(2, false), (3, true), (4, false)]);
        assert_eq!(renumber_choices(&marks, 5, 3), [(1, true), (2, false), (4, false), (5, false), (6, false)]);
        assert_eq!(renumber_choices(&[], 0, 1), []);
    }

    #[test]
    fn marks_describe_their_state() {
        assert_eq!(describe(&mark(2, 4..7, MarkStatus::Applied)), "Alignment 2, lines 5 to 7: applied");
        assert_eq!(
            describe(&mark(1, 3..3, MarkStatus::Pending)),
            "Alignment 1, anchor before line 4: waits for Apply Alignments"
        );
        assert!(describe(&mark(1, 0..1, MarkStatus::Unmatched)).contains("has no effect"));
        let moved = AlignmentMark {
            replaced_by: Some(1),
            ..mark(2, 4..7, MarkStatus::Applied)
        };
        assert!(describe(&moved).starts_with("Alignment 2, lines 5 to 7: applied. It is moved"));
    }

    fn moved(number: u32, lines: Range<usize>, replaced_by: u32, status: MarkStatus) -> AlignmentMark {
        AlignmentMark {
            replaced_by: Some(replaced_by),
            ..mark(number, lines, status)
        }
    }

    /// Lines 0 to 4 shown as 3, 4, 2, 0, 1: the hunks of lines 0 and 1 and of lines 3 and 4 swapped places, and the line between them stayed.
    fn rows_with_swapped_hunks() -> RowMap {
        RowMap::from_rows([3, 4, 2, 0, 1].into_iter().map(|line| PaneRow { line: Some(line), ..PaneRow::default() }).collect())
    }

    fn places_of(marks: &[AlignmentMark], rows: Rows<'_>) -> Vec<Place> {
        marks.iter().map(|mark| place(mark, rows, 5)).collect()
    }

    #[test]
    fn the_arrow_runs_from_where_the_hunk_was_to_its_first_row() {
        let map = rows_with_swapped_hunks();
        let rows = Rows::Mapped(&map);
        let marks = [moved(1, 0..2, 2, MarkStatus::Applied), mark(2, 3..5, MarkStatus::Applied)];
        let places = places_of(&marks, rows);
        assert_eq!(arrow(&marks[0], &marks, &places), Some(Arrow { from: 0.5, to: 3.5 }));
        assert_eq!(arrow(&marks[1], &marks, &places), None);
    }

    #[test]
    fn only_applied_ranges_that_moved_get_an_arrow() {
        let map = rows_with_swapped_hunks();
        let rows = Rows::Mapped(&map);
        let pending = [moved(1, 0..2, 2, MarkStatus::Pending), mark(2, 3..5, MarkStatus::Applied)];
        assert_eq!(arrow(&pending[0], &pending, &places_of(&pending, rows)), None);
        let anchor = [moved(1, 0..0, 2, MarkStatus::Applied), mark(2, 3..5, MarkStatus::Applied)];
        assert_eq!(arrow(&anchor[0], &anchor, &places_of(&anchor, rows)), None);
        let unmoved = [mark(1, 0..2, MarkStatus::Applied), mark(2, 3..5, MarkStatus::Applied)];
        assert_eq!(arrow(&unmoved[0], &unmoved, &places_of(&unmoved, rows)), None);
        let unpaired = [moved(1, 0..2, 2, MarkStatus::Applied)];
        assert_eq!(arrow(&unpaired[0], &unpaired, &places_of(&unpaired, rows)), None);
    }

    #[test]
    fn a_hunk_whose_old_place_shows_an_anchor_points_from_its_edge() {
        let marks = [moved(1, 0..1, 2, MarkStatus::Applied), mark(2, 0..0, MarkStatus::Applied)];
        let places = [Place::Rows(2..3), Place::Edge(0)];
        assert_eq!(arrow(&marks[0], &marks, &places), Some(Arrow { from: 0.0, to: 2.5 }));
        let near = [Place::Rows(0..1), Place::Edge(0)];
        assert_eq!(arrow(&marks[0], &marks, &near), None);
    }

    #[test]
    fn arrows_that_overlap_in_height_take_different_lanes() {
        let arrows = [Arrow { from: 0.5, to: 3.5 }, Arrow { from: 6.5, to: 2.5 }, Arrow { from: 4.5, to: 8.5 }];
        assert_eq!(assign_lanes(&arrows), [0, 1, 0]);
        assert_eq!(assign_lanes(&[]), Vec::<usize>::new());
        assert_eq!(assign_lanes(&[Arrow { from: 1.5, to: 3.5 }, Arrow { from: 3.5, to: 1.5 }]), [0, 1]);
    }

    #[test]
    fn the_strip_of_arrows_grows_with_the_lanes_and_the_gap() {
        assert_eq!(arrows_width(0, 1.0), 0.0);
        assert_eq!(arrows_width(1, 1.0), ARROW_EDGE + ARROW_WIDTH + ARROW_RUN);
        assert_eq!(arrows_width(3, 1.0), ARROW_EDGE + 3.0 * ARROW_WIDTH + 2.0 + ARROW_RUN);
        assert_eq!(arrows_width(3, 0.0), ARROW_EDGE + 3.0 * ARROW_WIDTH + ARROW_RUN);
        assert_eq!(arrows_width(3, -4.0), arrows_width(3, 0.0));
    }

    /// The shapes `paint_arrows` draws on rows 0 to 5, each 10 pixels high, for a strip from 4 to 24.
    fn painted(arrows: &[Arrow], gap: f32) -> Vec<Shape> {
        let lanes = assign_lanes(arrows);
        let context = egui::Context::default();
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.ctx().layer_painter(egui::LayerId::background());
            for row in 0..6 {
                let rect = Rect::from_min_size(pos2(0.0, row as f32 * 10.0), egui::vec2(100.0, 10.0));
                paint_arrows(&painter, arrows, &lanes, row, 6, rect, 4.0, 20.0, gap, Color32::BLUE);
            }
        });
        output.textures_delta.clear();
        output.shapes.into_iter().map(|clipped| clipped.shape).collect()
    }

    #[test]
    fn an_arrow_is_drawn_like_a_branch_with_both_ends_next_to_the_line_numbers() {
        let shapes = painted(&[Arrow { from: 0.5, to: 3.5 }], 1.0);
        let dots: Vec<&egui::epaint::CircleShape> = shapes.iter().filter_map(|shape| if let Shape::Circle(circle) = shape { Some(circle) } else { None }).collect();
        assert_eq!(dots.len(), 1);
        assert_eq!((dots[0].center.x + ARROW_DOT_RADIUS, dots[0].center.y), (23.0, 5.0));
        let heads: Vec<&egui::epaint::PathShape> = shapes.iter().filter_map(|shape| if let Shape::Path(path) = shape { Some(path) } else { None }).collect();
        assert_eq!(heads.len(), 1);
        assert_eq!(heads[0].points[0], pos2(23.0, 35.0));
        let lines: Vec<[egui::Pos2; 2]> = shapes.iter().filter_map(|shape| if let Shape::LineSegment { points, .. } = shape { Some(*points) } else { None }).collect();
        let lane_x = 4.0 + ARROW_EDGE + ARROW_WIDTH / 2.0;
        let vertical: f32 = lines.iter().filter(|points| points[0].x == lane_x && points[1].x == lane_x).map(|points| (points[1].y - points[0].y).abs()).sum();
        assert_eq!(vertical, 30.0);
        assert!(lines.iter().any(|points| points[0] == pos2(lane_x, 5.0) && points[1].y == 5.0 && points[1].x > lane_x));
        assert!(lines.iter().any(|points| points[0] == pos2(lane_x, 35.0) && points[1] == pos2(23.0 - ARROW_HEAD_LENGTH, 35.0)));
    }

    #[test]
    fn further_lanes_are_a_line_and_a_gap_further_left() {
        let arrows = [Arrow { from: 0.5, to: 3.5 }, Arrow { from: 2.5, to: 5.5 }];
        let shapes = painted(&arrows, 3.0);
        let xs: Vec<f32> = shapes
            .iter()
            .filter_map(|shape| if let Shape::LineSegment { points, .. } = shape { Some(*points) } else { None })
            .filter(|points| points[0].x == points[1].x)
            .map(|points| points[0].x)
            .collect();
        let near_numbers = 4.0 + ARROW_EDGE + ARROW_WIDTH / 2.0 + (ARROW_WIDTH + 3.0);
        assert!(xs.contains(&near_numbers));
        assert!(xs.contains(&(near_numbers - (ARROW_WIDTH + 3.0))));
    }

    #[test]
    fn options_say_where_they_show_the_alignment_and_who_moves() {
        let option = |above, below, moved: &[Document]| InsetOption {
            slot: 0,
            above,
            below,
            moved: moved.to_vec(),
        };
        assert_eq!(option_label(&option(None, Some(1), &[Document::A, Document::C])), "Above alignment 1 (moves A and C)");
        assert_eq!(option_label(&option(Some(1), None, &[Document::B])), "Below alignment 1 (moves B)");
        assert_eq!(option_label(&option(Some(1), Some(3), &[Document::A, Document::B, Document::C])), "Between alignments 1 and 3 (moves A, B and C)");
        assert_eq!(option_label(&option(Some(2), None, &[])), "Below alignment 2");
    }
}
