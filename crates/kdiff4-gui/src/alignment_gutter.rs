use std::ops::Range;

use egui::{Align2, Color32, FontId, Painter, Rect, Stroke, StrokeKind, pos2};
use kdiff4_protocol::{AlignmentMark, MarkStatus};

use crate::rows::Rows;
use crate::text_view;
use crate::theme::{DocumentColors, Palette};

/// How much of a row the chip of an anchor covers.
const CHIP_HEIGHT_SHARE: f32 = 0.8;
/// The border of a mark nothing in the other documents pairs with, as a share of the color's strength.
const UNMATCHED_BORDER_SHARE: f32 = 0.45;

/// The alignment marks of one source pane, and the highest number any document uses, so a mark can be renumbered to pair with a mark of another document.
#[derive(Debug, Clone, Copy)]
pub struct PaneMarks<'a> {
    pub marks: &'a [AlignmentMark],
    pub highest: u32,
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
    format!("Alignment {}, {what}: {state}", mark.number)
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
    }
}
