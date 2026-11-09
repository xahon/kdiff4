use std::ops::Range;

use kdiff4_protocol::engine::{Comparison, HunkChange, MergeStatus, Region, RegionStatus};
use kdiff4_protocol::{Choice, Document, LineCharRuns, RegionMerge};

use crate::char_diff::{self, CharStyle, HunksEqual};
use crate::editing::Position;

/// Where a line of D came from: a chosen hunk, or a hand edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Source(Document),
    Modified,
}

/// One gutter block of a pane: the rows of a region that differs. `index` tells neighboring blocks apart; `origin` is the source document the block belongs to, or `None` in D, whose rows carry their own origins. `style` is how a source pane marks the block, by the char-diff rules: in the color of the one other document its region differs from, or red when it differs from all of them; `None` in D, and in a source pane when the hunks are all equal (a hand-edited region), where the block keeps its own document's color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    pub index: usize,
    pub origin: Option<Mark>,
    pub style: Option<CharStyle>,
}

/// One row of a pane: its line, or `None` for a void, the region it belongs to, whether that region is a difference, drawn as a bordered box, and the gutter block of that region, if any. `origin` is where a line of D in a block came from, `None` when unknown or elsewhere. `void_source` is the document whose choice left a void row of D empty. `conflict_marker` is D's placeholder of an unresolved conflict.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PaneRow {
    pub line: Option<usize>,
    pub region: Option<usize>,
    pub boxed: bool,
    pub block: Option<Block>,
    pub origin: Option<Mark>,
    pub void_source: Option<Document>,
    pub conflict_marker: bool,
}

/// The rows of one pane, built from the comparison and, for D, the merge.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowMap {
    rows: Vec<PaneRow>,
    row_of_line: Vec<usize>,
    /// Per region: the row it starts at, or would start at if it has no rows.
    region_starts: Vec<usize>,
}

impl RowMap {
    /// `heights`, when given, pads every region with void rows to that many rows, so panes line up with D. `merge` tells which regions D holds a hand edit of; they stand out like differences. With `whitespace_runs`, the char diffs computed with whitespace ignored, gutter blocks also treat hunks as equal where those runs mark nothing between them. Falls back to one plain row per line when the comparison doesn't hold every line of the document exactly once. Lines may come in any order: an inset hunk is shown away from its place in the document.
    pub fn for_source(comparison: &Comparison, merge: &[RegionMerge], document_index: usize, line_count: usize, heights: Option<&[usize]>, whitespace_runs: Option<&[LineCharRuns]>) -> RowMap {
        let equal = |region: &Region| {
            let equal = HunksEqual::from_status(&region.status);
            match whitespace_runs {
                Some(runs) => equal.or_from_char_diffs(region, runs),
                None => equal,
            }
        };
        let mut rows = Vec::new();
        let mut row_of_line = vec![usize::MAX; line_count];
        let mut shown = 0;
        let mut region_starts = Vec::with_capacity(comparison.regions.len());
        for (index, region) in comparison.regions.iter().enumerate() {
            region_starts.push(rows.len());
            let boxed = stands_out(region, merge.get(index));
            let document = Document::ALL.get(document_index).cloned();
            let style = document.and_then(|document| char_diff::region_style(document, equal(region), region.hunks.len()));
            let block = boxed.then(|| Block {
                index: index.clone(),
                origin: document.map(Mark::Source),
                style,
            });
            for row in &region.rows {
                let line = row.lines.get(document_index).cloned().flatten();
                if let Some(line) = line {
                    if row_of_line.get(line).is_none_or(|row| *row != usize::MAX) {
                        return RowMap::fallback(line_count, "a line is missing or shown twice");
                    }
                    row_of_line[line] = rows.len();
                    shown += 1;
                }
                rows.push(PaneRow {
                    line,
                    region: Some(index),
                    boxed,
                    block,
                    ..PaneRow::default()
                });
            }
            pad(&mut rows, index, region_starts[index], heights, boxed, block, None);
        }
        if shown != line_count {
            return RowMap::fallback(line_count, "lines missing");
        }
        RowMap { rows, row_of_line, region_starts }
    }

    /// D's rows: its lines, region by region, each line with its origin, and one row for a region that differs or is hand-edited but has no lines: a placeholder for an unresolved conflict, a void otherwise. A region that differs or is hand-edited is one gutter block, like in the source panes. `heights` pads regions like `for_source`. Lines past the regions (the empty line after D's last line break) follow as plain rows.
    pub fn for_output(merge: &[RegionMerge], regions: &[Region], line_count: usize, heights: Option<&[usize]>) -> RowMap {
        let mut rows = Vec::with_capacity(line_count + merge.len());
        let mut row_of_line = Vec::with_capacity(line_count);
        let mut region_starts = Vec::with_capacity(merge.len());
        for (index, state) in merge.iter().enumerate() {
            region_starts.push(rows.len());
            let boxed = regions.get(index).is_some_and(|region| stands_out(region, Some(state)));
            let block = boxed.then(|| Block {
                index: index.clone(),
                origin: None,
                style: None,
            });
            let void_source = accepted_void_source(state);
            if state.output_lines == 0 && (boxed || state.is_unresolved()) {
                rows.push(PaneRow {
                    region: Some(index),
                    boxed,
                    block,
                    void_source,
                    conflict_marker: state.is_unresolved(),
                    ..PaneRow::default()
                });
            }
            let marks = marks(state, regions.get(index));
            for mark in marks {
                if row_of_line.len() >= line_count {
                    return RowMap::fallback(line_count, "more region lines than D has");
                }
                let origin = if boxed { mark } else { None };
                row_of_line.push(rows.len());
                rows.push(PaneRow {
                    line: Some(row_of_line.len() - 1),
                    region: Some(index),
                    boxed,
                    block,
                    origin,
                    ..PaneRow::default()
                });
            }
            pad(&mut rows, index, region_starts[index], heights, boxed, block, void_source);
        }
        while row_of_line.len() < line_count {
            row_of_line.push(rows.len());
            rows.push(PaneRow {
                line: Some(row_of_line.len() - 1),
                ..PaneRow::default()
            });
        }
        RowMap { rows, row_of_line, region_starts }
    }

    #[cfg(test)]
    pub fn from_rows(rows: Vec<PaneRow>) -> RowMap {
        let lines: Vec<usize> = rows.iter().filter_map(|row| row.line).collect();
        let row_of_line = if lines.windows(2).all(|pair| pair[0] < pair[1]) {
            rows.iter().enumerate().filter_map(|(index, row)| row.line.map(|_| index)).collect()
        } else {
            let mut row_of_line = vec![0; lines.len()];
            for (index, line) in rows.iter().enumerate().filter_map(|(index, row)| row.line.map(|line| (index, line))) {
                row_of_line[line] = index;
            }
            row_of_line
        };
        RowMap {
            rows,
            row_of_line,
            region_starts: Vec::new(),
        }
    }

    fn fallback(line_count: usize, reason: &str) -> RowMap {
        log::warn!("the comparison doesn't fit a document of {line_count} lines ({reason}); showing it without rows");
        let rows = (0..line_count)
            .map(|line| PaneRow {
                line: Some(line),
                ..PaneRow::default()
            })
            .collect();
        RowMap {
            rows,
            row_of_line: (0..line_count).collect(),
            region_starts: Vec::new(),
        }
    }
}

/// Pads the rows of region `index`, which started at row `start`, with void rows of `block` up to its height.
fn pad(rows: &mut Vec<PaneRow>, index: usize, start: usize, heights: Option<&[usize]>, boxed: bool, block: Option<Block>, void_source: Option<Document>) {
    let height = heights.and_then(|heights| heights.get(index)).cloned().unwrap_or(0);
    while rows.len() - start < height {
        rows.push(PaneRow {
            region: Some(index),
            boxed,
            block,
            void_source,
            ..PaneRow::default()
        });
    }
}

/// The first document of the choice of a region that contributes no lines to D, when it was not edited by hand: the document whose empty hunk was accepted.
fn accepted_void_source(state: &RegionMerge) -> Option<Document> {
    match &state.choice {
        Choice::Hunks(documents) if state.output_lines == 0 && !state.edited => documents.first().cloned(),
        _ => None,
    }
}

/// The marks of a region's lines in D: per chosen hunk, one per line of that hunk, in order; all `Modified` for a hand-edited region. Unknown when the counts don't match D.
fn marks(state: &RegionMerge, region: Option<&Region>) -> Vec<Option<Mark>> {
    let unknown = || vec![None; state.output_lines];
    if state.edited {
        return vec![Some(Mark::Modified); state.output_lines];
    }
    let (Choice::Hunks(documents), Some(region)) = (&state.choice, region) else {
        return unknown();
    };
    let mut marks = Vec::with_capacity(state.output_lines);
    for document in documents {
        let length = source_index(document.clone()).and_then(|index| region.hunks.get(index)).map_or(0, |hunk| hunk.len());
        marks.extend(std::iter::repeat_n(Some(Mark::Source(document.clone())), length));
    }
    if marks.len() == state.output_lines { marks } else { unknown() }
}

fn source_index(document: Document) -> Option<usize> {
    match document {
        Document::A => Some(0),
        Document::B => Some(1),
        Document::C => Some(2),
        Document::D => None,
    }
}

/// Per region, the rows every pane gives it when D lines up with the source panes: the most any pane needs, and one for the row D shows for a region without lines that differs, is hand-edited or is unresolved.
pub fn aligned_heights(regions: &[Region], merge: &[RegionMerge]) -> Vec<usize> {
    regions
        .iter()
        .enumerate()
        .map(|(index, region)| {
            let output = merge.get(index).map_or(0, |state| {
                if state.output_lines == 0 && (stands_out(region, Some(state)) || state.is_unresolved()) {
                    1
                } else {
                    state.output_lines
                }
            });
            region.rows.len().max(output)
        })
        .collect()
}

/// The rows a text view draws: one per line, or the rows of a pane.
#[derive(Debug, Clone, Copy)]
pub enum Rows<'a> {
    PerLine(usize),
    Mapped(&'a RowMap),
}

impl Rows<'_> {
    pub fn len(&self) -> usize {
        match self {
            Rows::PerLine(count) => *count,
            Rows::Mapped(map) => map.rows.len(),
        }
    }

    pub fn row(&self, row: usize) -> Option<PaneRow> {
        match self {
            Rows::PerLine(count) => (row < *count).then_some(PaneRow {
                line: Some(row),
                ..PaneRow::default()
            }),
            Rows::Mapped(map) => map.rows.get(row).cloned(),
        }
    }

    /// The row of `line`; a line past the end maps to the last row.
    pub fn row_of_line(&self, line: usize) -> usize {
        match self {
            Rows::PerLine(count) => line.min(count.saturating_sub(1)),
            Rows::Mapped(map) => map.row_of_line.get(line).or(map.row_of_line.last()).cloned().unwrap_or(0),
        }
    }

    /// The first line at or below `row`, so a click on a void lands on the text after it.
    pub fn line_at_or_after(&self, row: usize) -> Option<usize> {
        match self {
            Rows::PerLine(count) => (row < *count).then_some(row),
            Rows::Mapped(map) => map.rows.iter().skip(row).find_map(|row| row.line),
        }
    }

    /// The row region `region` starts at, clamped to the rows.
    pub fn region_start(&self, region: usize) -> Option<usize> {
        match self {
            Rows::PerLine(_) => None,
            Rows::Mapped(map) => map.region_starts.get(region).map(|row| (*row).min(map.rows.len().saturating_sub(1))),
        }
    }

    /// The rows of region `region`, empty when it has none in this pane.
    pub fn region_rows(&self, region: usize) -> Option<Range<usize>> {
        let Rows::Mapped(map) = self else {
            return None;
        };
        let start = map.region_starts.get(region).cloned()?.min(map.rows.len());
        let length = map.rows[start..].iter().take_while(|row| row.region == Some(region)).count();
        Some(start..start + length)
    }

    /// The region of `row`, if it has one.
    pub fn region_of_row(&self, row: usize) -> Option<usize> {
        self.row(row).and_then(|pane_row| pane_row.region)
    }

    /// The gutter block of `row`, if it has one.
    pub fn block_of_row(&self, row: usize) -> Option<Block> {
        self.row(row).and_then(|pane_row| pane_row.block)
    }

    /// Whether `row` is the first or the last row of its gutter block.
    pub fn block_edges(&self, row: usize) -> (bool, bool) {
        let block = self.block_of_row(row);
        let first = row == 0 || self.block_of_row(row - 1) != block;
        let last = self.block_of_row(row + 1) != block;
        (first, last)
    }

    /// The caret on `row`: on a line, at `column` of it; on a void, at the start of the line after it (the end of the text when none follows) with the void row, so the caret can be drawn there. `end` is the end of the text.
    pub fn caret_at_row(&self, row: usize, column: usize, end: Position) -> (Position, Option<usize>) {
        let row = row.min(self.len().saturating_sub(1));
        match self.row(row).and_then(|pane_row| pane_row.line) {
            Some(line) => (Position { line, column }, None),
            None => {
                let position = self.line_at_or_after(row).map_or(end, |line| Position { line, column: 0 });
                (position, self.row(row).map(|_| row))
            }
        }
    }
}

/// The row `delta` rows away from `row`, kept inside `0..count`.
pub fn step_row(row: usize, delta: isize, count: usize) -> usize {
    row.saturating_add_signed(delta).min(count.saturating_sub(1))
}

/// Whether the hunks of a region differ; a region whose hunks are all equal does not.
fn is_difference(region: &Region) -> bool {
    !matches!(region.status, RegionStatus::Diff(HunkChange::Unchanged) | RegionStatus::Merge(MergeStatus::Unchanged))
}

/// Whether a region is drawn as a box with a gutter block: its hunks differ, or D holds a hand edit of it, like a line split out of a stable region.
fn stands_out(region: &Region, state: Option<&RegionMerge>) -> bool {
    is_difference(region) || state.is_some_and(|state| state.edited)
}

#[cfg(test)]
mod tests {
    use kdiff4_protocol::engine::Row;

    use super::*;

    fn region(hunks: &[Range<usize>], status: RegionStatus, rows: &[&[Option<usize>]]) -> Region {
        let rows = rows.iter().map(|lines| Row { lines: lines.to_vec() }).collect();
        Region {
            hunks: hunks.to_vec(),
            status,
            rows,
        }
    }

    fn line(line: usize, region: usize) -> PaneRow {
        PaneRow {
            line: Some(line),
            region: Some(region),
            ..PaneRow::default()
        }
    }

    fn void(region: usize) -> PaneRow {
        PaneRow {
            region: Some(region),
            ..PaneRow::default()
        }
    }

    #[test]
    fn only_regions_whose_hunks_differ_are_boxed() {
        assert!(!is_difference(&region(&[0..1, 0..1], RegionStatus::Diff(HunkChange::Unchanged), &[])));
        assert!(!is_difference(&region(&[0..1, 0..1, 0..1], RegionStatus::Merge(MergeStatus::Unchanged), &[])));
        assert!(is_difference(&region(&[0..0, 0..1], RegionStatus::Diff(HunkChange::Added), &[])));
        assert!(is_difference(&region(&[0..1, 0..2, 0..1], RegionStatus::Merge(MergeStatus::ChangedInB), &[])));
        assert!(is_difference(&region(&[0..1, 0..1, 0..1], RegionStatus::Merge(MergeStatus::WhitespaceConflict), &[])));
    }

    fn two_way_comparison() -> Comparison {
        Comparison {
            regions: vec![
                region(&[0..1, 0..1], RegionStatus::Diff(HunkChange::Unchanged), &[&[Some(0), Some(0)]]),
                region(&[1..1, 1..3], RegionStatus::Diff(HunkChange::Added), &[&[None, Some(1)], &[None, Some(2)]]),
                region(&[1..2, 3..4], RegionStatus::Diff(HunkChange::Unchanged), &[&[Some(1), Some(3)]]),
            ],
        }
    }

    #[test]
    fn source_rows_follow_the_comparison() {
        let map = RowMap::for_source(&two_way_comparison(), &[], 0, 2, None, None);
        let rows = Rows::Mapped(&map);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows.row(0), Some(line(0, 0)));
        assert_eq!(
            rows.row(1),
            Some(PaneRow {
                boxed: true,
                block: source_block(1, Document::A, FROM_B),
                ..void(1)
            })
        );
        assert_eq!(rows.row(3), Some(line(1, 2)));
        assert_eq!(rows.row(4), None);
        assert_eq!(rows.row_of_line(1), 3);
        let map = RowMap::for_source(&two_way_comparison(), &[], 1, 4, None, None);
        assert_eq!(
            Rows::Mapped(&map).row(2),
            Some(PaneRow {
                boxed: true,
                block: source_block(1, Document::B, FROM_A),
                ..line(2, 1)
            })
        );
    }

    const FROM_A: Option<CharStyle> = Some(CharStyle::Document(Document::A));
    const FROM_B: Option<CharStyle> = Some(CharStyle::Document(Document::B));
    const RED: Option<CharStyle> = Some(CharStyle::Red);

    fn source_block(region: usize, document: Document, style: Option<CharStyle>) -> Option<Block> {
        Some(Block {
            index: region,
            origin: Some(Mark::Source(document)),
            style,
        })
    }

    #[test]
    fn unchanged_regions_have_no_gutter() {
        let map = RowMap::for_source(&two_way_comparison(), &[], 1, 4, None, None);
        let blocks: Vec<Option<Block>> = (0..4).map(|row| Rows::Mapped(&map).block_of_row(row)).collect();
        assert_eq!(blocks, [None, source_block(1, Document::B, FROM_A), source_block(1, Document::B, FROM_A), None]);
        let merge = RegionMerge {
            choice: Choice::Hunks(vec![Document::A]),
            edited: false,
            output_lines: 1,
        };
        let output = RowMap::for_output(&[merge], &two_way_comparison().regions[..1], 1, None);
        assert_eq!(Rows::Mapped(&output).block_of_row(0), None);
    }

    #[test]
    fn regions_know_their_rows() {
        let map = RowMap::for_source(&two_way_comparison(), &[], 0, 2, None, None);
        let rows = Rows::Mapped(&map);
        assert_eq!((rows.region_start(0), rows.region_start(1), rows.region_start(2)), (Some(0), Some(1), Some(3)));
        assert_eq!(rows.region_start(3), None);
        assert_eq!(rows.block_edges(0), (true, true));
        assert_eq!(rows.block_edges(1), (true, false));
        assert_eq!(rows.block_edges(2), (false, true));
        assert_eq!(Rows::PerLine(3).block_of_row(0), None);
        assert_eq!(Rows::PerLine(3).region_start(0), None);
        assert_eq!((rows.region_rows(0), rows.region_rows(1), rows.region_rows(2)), (Some(0..1), Some(1..3), Some(3..4)));
        assert_eq!(rows.region_rows(3), None);
        assert_eq!(Rows::PerLine(3).region_rows(0), None);
    }

    #[test]
    fn heights_pad_regions_with_voids() {
        let map = RowMap::for_source(&two_way_comparison(), &[], 0, 2, Some(&[2, 2, 1]), None);
        let rows = Rows::Mapped(&map);
        assert_eq!(rows.len(), 5);
        assert_eq!(rows.row(1), Some(void(0)));
        assert_eq!(
            rows.row(3),
            Some(PaneRow {
                boxed: true,
                block: source_block(1, Document::A, FROM_B),
                ..void(1)
            })
        );
        assert_eq!(rows.region_rows(1), Some(2..4));
        assert_eq!(rows.row_of_line(1), 4);
        assert_eq!(rows.region_start(2), Some(4));
    }

    #[test]
    fn clicks_on_voids_go_to_the_next_line() {
        let map = RowMap::for_source(&two_way_comparison(), &[], 0, 2, None, None);
        let rows = Rows::Mapped(&map);
        assert_eq!(rows.line_at_or_after(1), Some(1));
        assert_eq!(rows.line_at_or_after(3), Some(1));
        assert_eq!(rows.line_at_or_after(4), None);
    }

    #[test]
    fn caret_can_sit_on_a_void_row() {
        let map = RowMap::for_source(&two_way_comparison(), &[], 0, 2, None, None);
        let rows = Rows::Mapped(&map);
        let end = Position { line: 1, column: 1 };
        assert_eq!(rows.caret_at_row(0, 3, end), (Position { line: 0, column: 3 }, None));
        assert_eq!(rows.caret_at_row(2, 3, end), (Position { line: 1, column: 0 }, Some(2)));
        assert_eq!(rows.caret_at_row(9, 0, end), (Position { line: 1, column: 0 }, None));
        let trailing = RowMap::for_source(&two_way_comparison(), &[], 0, 2, Some(&[1, 2, 2]), None);
        assert_eq!(Rows::Mapped(&trailing).caret_at_row(4, 0, end), (end, Some(4)));
    }

    #[test]
    fn rows_step_inside_the_pane() {
        assert_eq!(step_row(3, -1, 10), 2);
        assert_eq!(step_row(0, -5, 10), 0);
        assert_eq!(step_row(8, 5, 10), 9);
        assert_eq!(step_row(0, 1, 0), 0);
    }

    #[test]
    fn row_of_line_past_the_end_is_the_last_row() {
        let map = RowMap::for_source(&two_way_comparison(), &[], 0, 2, None, None);
        assert_eq!(Rows::Mapped(&map).row_of_line(9), 3);
        assert_eq!(Rows::Mapped(&RowMap::default()).row_of_line(0), 0);
        assert_eq!(Rows::PerLine(3).row_of_line(9), 2);
        assert_eq!(Rows::PerLine(0).row_of_line(0), 0);
    }

    #[test]
    fn per_line_rows() {
        let rows = Rows::PerLine(2);
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows.row(1),
            Some(PaneRow {
                line: Some(1),
                ..PaneRow::default()
            })
        );
        assert_eq!(rows.row(2), None);
        assert_eq!(rows.line_at_or_after(1), Some(1));
        assert_eq!(rows.line_at_or_after(2), None);
    }

    #[test]
    fn comparison_that_does_not_fit_falls_back_to_lines() {
        let map = RowMap::for_source(&two_way_comparison(), &[], 0, 5, None, None);
        assert_eq!(Rows::Mapped(&map).len(), 5);
        assert_eq!(
            Rows::Mapped(&map).row(4),
            Some(PaneRow {
                line: Some(4),
                ..PaneRow::default()
            })
        );
        let mut twice = two_way_comparison();
        twice.regions.push(twice.regions[0].clone());
        let map = RowMap::for_source(&twice, &[], 0, 2, None, None);
        assert_eq!(Rows::Mapped(&map).len(), 2);
        assert_eq!(Rows::Mapped(&map).row_of_line(1), 1);
    }

    #[test]
    fn lines_shown_away_from_their_place_keep_their_rows() {
        let mut shuffled = two_way_comparison();
        shuffled.regions.swap(0, 2);
        let map = RowMap::for_source(&shuffled, &[], 0, 2, None, None);
        let rows = Rows::Mapped(&map);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows.row_of_line(1), 0);
        assert_eq!(rows.row_of_line(0), 3);
        assert_eq!(rows.row(0).and_then(|row| row.line), Some(1));
        assert_eq!(rows.row(3).and_then(|row| row.line), Some(0));
        assert_eq!(rows.line_at_or_after(1), Some(0));
    }

    fn state(choice: Choice, edited: bool, output_lines: usize) -> RegionMerge {
        RegionMerge { choice, edited, output_lines }
    }

    fn conflict_region(hunks: &[Range<usize>], rows: usize) -> Region {
        let rows: Vec<Vec<Option<usize>>> = (0..rows).map(|_| vec![None; hunks.len()]).collect();
        let rows: Vec<&[Option<usize>]> = rows.iter().map(Vec::as_slice).collect();
        region(hunks, RegionStatus::Merge(MergeStatus::Conflict), &rows)
    }

    #[test]
    fn output_rows_show_regions_without_lines() {
        let regions = [
            conflict_region(&[0..1, 0..1, 0..1], 1),
            conflict_region(&[1..2, 1..2, 1..2], 1),
            conflict_region(&[2..2, 2..2, 2..2], 0),
            conflict_region(&[2..4, 2..4, 2..4], 2),
        ];
        let merge = [
            state(Choice::Hunks(vec![Document::A]), false, 1),
            state(Choice::Unresolved, false, 0),
            state(Choice::Hunks(Vec::new()), false, 0),
            state(Choice::Hunks(vec![Document::B]), false, 2),
        ];
        let map = RowMap::for_output(&merge, &regions, 4, None);
        let rows = Rows::Mapped(&map);
        assert_eq!(rows.len(), 6);
        assert_eq!(
            rows.row(0),
            Some(PaneRow {
                boxed: true,
                block: output_block(0),
                origin: Some(Mark::Source(Document::A)),
                ..line(0, 0)
            })
        );
        assert_eq!(
            rows.row(1),
            Some(PaneRow {
                boxed: true,
                block: output_block(1),
                conflict_marker: true,
                ..void(1)
            })
        );
        assert_eq!(
            rows.row(2),
            Some(PaneRow {
                boxed: true,
                block: output_block(2),
                ..void(2)
            })
        );
        assert_eq!(
            rows.row(3),
            Some(PaneRow {
                boxed: true,
                block: output_block(3),
                origin: Some(Mark::Source(Document::B)),
                ..line(1, 3)
            })
        );
        assert_eq!(
            rows.row(5),
            Some(PaneRow {
                line: Some(3),
                ..PaneRow::default()
            })
        );
        assert_eq!(rows.row_of_line(3), 5);
        assert_eq!(rows.line_at_or_after(2), Some(1));
        assert_eq!(rows.region_rows(2), Some(2..3));
        assert_eq!(rows.region_rows(3), Some(3..5));
    }

    #[test]
    fn hand_edited_regions_without_lines_keep_their_void_row() {
        let regions = [conflict_region(&[0..1, 0..1], 1)];
        let map = RowMap::for_output(&[state(Choice::Unresolved, true, 0)], &regions, 0, None);
        assert_eq!(
            Rows::Mapped(&map).row(0),
            Some(PaneRow {
                boxed: true,
                block: output_block(0),
                ..void(0)
            })
        );
        assert_eq!(Rows::Mapped(&map).len(), 1);
    }

    #[test]
    fn accepted_voids_of_d_know_the_document_they_were_accepted_from() {
        let regions = [conflict_region(&[0..1, 0..0], 1)];
        let accepted = state(Choice::Hunks(vec![Document::B]), false, 0);
        let map = RowMap::for_output(&[accepted], &regions, 0, Some(&[2]));
        let sources: Vec<Option<Document>> = (0..2).map(|row| Rows::Mapped(&map).row(row).and_then(|row| row.void_source)).collect();
        assert_eq!(sources, [Some(Document::B), Some(Document::B)]);
        for merge in [state(Choice::Unresolved, false, 0), state(Choice::Hunks(vec![Document::B]), true, 0)] {
            let map = RowMap::for_output(&[merge], &regions, 0, None);
            assert_eq!(Rows::Mapped(&map).row(0).and_then(|row| row.void_source), None);
        }
    }

    #[test]
    fn unchanged_regions_without_lines_get_no_row() {
        let unchanged = region(&[0..0, 0..0], RegionStatus::Diff(HunkChange::Unchanged), &[]);
        let map = RowMap::for_output(&[state(Choice::Hunks(vec![Document::A]), false, 0)], &[unchanged], 0, None);
        assert_eq!(Rows::Mapped(&map).len(), 0);
    }

    fn output_block(region: usize) -> Option<Block> {
        Some(Block {
            index: region,
            origin: None,
            style: None,
        })
    }

    #[test]
    fn output_lines_carry_their_origin_in_one_block_per_region() {
        let regions = [conflict_region(&[0..1, 0..2, 0..1], 2), conflict_region(&[1..2, 2..3, 1..2], 1)];
        let merge = [
            state(Choice::Hunks(vec![Document::C, Document::B]), false, 3),
            state(Choice::Hunks(vec![Document::B]), true, 1),
        ];
        let map = RowMap::for_output(&merge, &regions, 4, None);
        let rows = Rows::Mapped(&map);
        let blocks: Vec<Option<Block>> = (0..4).map(|row| rows.block_of_row(row)).collect();
        assert_eq!(blocks, [output_block(0), output_block(0), output_block(0), output_block(1)]);
        let origins: Vec<Option<Mark>> = (0..4).map(|row| rows.row(row).and_then(|row| row.origin)).collect();
        assert_eq!(
            origins,
            [
                Some(Mark::Source(Document::C)),
                Some(Mark::Source(Document::B)),
                Some(Mark::Source(Document::B)),
                Some(Mark::Modified)
            ]
        );
        assert_eq!(rows.block_edges(0), (true, false));
        assert_eq!(rows.block_edges(2), (false, true));
        assert_eq!(rows.block_edges(3), (true, true));
    }

    #[test]
    fn output_blocks_of_equal_origin_stay_apart_across_regions() {
        let regions = [conflict_region(&[0..1, 0..1], 1), conflict_region(&[1..2, 1..2], 1)];
        let merge = [state(Choice::Hunks(vec![Document::B]), false, 1), state(Choice::Hunks(vec![Document::B]), false, 1)];
        let map = RowMap::for_output(&merge, &regions, 2, None);
        assert_eq!(Rows::Mapped(&map).block_edges(0), (true, true));
        assert_eq!(Rows::Mapped(&map).block_edges(1), (true, true));
    }

    #[test]
    fn output_origins_are_unknown_when_counts_do_not_match() {
        let regions = [conflict_region(&[0..1, 0..1], 1)];
        let map = RowMap::for_output(&[state(Choice::Hunks(vec![Document::B]), false, 2)], &regions, 2, None);
        assert_eq!(Rows::Mapped(&map).row(0).and_then(|row| row.origin), None);
        assert_eq!(Rows::Mapped(&map).block_of_row(0), output_block(0));
    }

    #[test]
    fn aligned_heights_fit_every_pane() {
        let regions = [conflict_region(&[0..1, 0..1], 1), conflict_region(&[1..3, 1..3], 2), conflict_region(&[3..3, 3..3], 0)];
        let merge = [
            state(Choice::Hunks(vec![Document::A, Document::B]), false, 2),
            state(Choice::Hunks(vec![Document::A]), false, 2),
            state(Choice::Unresolved, false, 0),
        ];
        let heights = aligned_heights(&regions, &merge);
        assert_eq!(heights, [2, 2, 1]);
        let map = RowMap::for_output(&merge, &regions, 4, Some(&heights));
        assert_eq!(Rows::Mapped(&map).len(), 5);
        assert_eq!(aligned_heights(&regions, &[]), [1, 2, 0]);
        let padded = RowMap::for_output(&merge, &regions, 4, Some(&[3, 2, 1]));
        assert_eq!(
            Rows::Mapped(&padded).row(2),
            Some(PaneRow {
                boxed: true,
                block: output_block(0),
                ..void(0)
            })
        );
        assert_eq!(Rows::Mapped(&padded).block_edges(2), (false, true));
    }

    #[test]
    fn output_rows_that_do_not_fit_fall_back_to_lines() {
        let map = RowMap::for_output(&[state(Choice::Unresolved, true, 3)], &[], 2, None);
        assert_eq!(Rows::Mapped(&map).len(), 2);
        assert_eq!(
            Rows::Mapped(&map).row(1),
            Some(PaneRow {
                line: Some(1),
                ..PaneRow::default()
            })
        );
    }

    fn split_stable_line() -> (Comparison, [RegionMerge; 3]) {
        let unchanged = |line: usize| {
            region(
                &[line..line + 1, line..line + 1, line..line + 1],
                RegionStatus::Merge(MergeStatus::Unchanged),
                &[&[Some(line), Some(line), Some(line)]],
            )
        };
        let merge = [
            state(Choice::Hunks(vec![Document::A]), false, 1),
            state(Choice::Hunks(vec![Document::A]), true, 1),
            state(Choice::Hunks(vec![Document::A]), false, 1),
        ];
        (
            Comparison {
                regions: vec![unchanged(0), unchanged(1), unchanged(2)],
            },
            merge,
        )
    }

    #[test]
    fn hand_edited_stable_lines_get_a_block_in_every_pane() {
        let (comparison, merge) = split_stable_line();
        let source = RowMap::for_source(&comparison, &merge, 1, 3, None, None);
        let block = source_block(1, Document::B, None);
        assert_eq!(Rows::Mapped(&source).row(0), Some(line(0, 0)));
        assert_eq!(Rows::Mapped(&source).row(1), Some(PaneRow { boxed: true, block, ..line(1, 1) }));
        assert_eq!(Rows::Mapped(&source).row(2), Some(line(2, 2)));
        let output = RowMap::for_output(&merge, &comparison.regions, 3, None);
        assert_eq!(Rows::Mapped(&output).row(0), Some(line(0, 0)));
        assert_eq!(
            Rows::Mapped(&output).row(1),
            Some(PaneRow {
                boxed: true,
                block: output_block(1),
                origin: Some(Mark::Modified),
                ..line(1, 1)
            })
        );
        assert_eq!(Rows::Mapped(&output).row(2), Some(line(2, 2)));
    }

    #[test]
    fn hand_edited_stable_lines_removed_from_d_keep_a_row() {
        let (comparison, mut merge) = split_stable_line();
        merge[1].output_lines = 0;
        assert_eq!(aligned_heights(&comparison.regions, &merge), [1, 1, 1]);
        let output = RowMap::for_output(&merge, &comparison.regions, 2, None);
        assert_eq!(
            Rows::Mapped(&output).row(1),
            Some(PaneRow {
                boxed: true,
                block: output_block(1),
                ..void(1)
            })
        );
        assert_eq!(Rows::Mapped(&output).row(2), Some(line(1, 2)));
    }

    fn styles_of_single_row_region(status: MergeStatus) -> Vec<Option<CharStyle>> {
        let comparison = Comparison {
            regions: vec![region(&[0..1, 0..1, 0..1], RegionStatus::Merge(status), &[&[Some(0), Some(0), Some(0)]])],
        };
        (0..3)
            .map(|document_index| {
                let map = RowMap::for_source(&comparison, &[], document_index, 1, None, None);
                Rows::Mapped(&map).block_of_row(0).and_then(|block| block.style)
            })
            .collect()
    }

    #[test]
    fn source_blocks_are_marked_in_the_color_of_the_documents_their_region_differs_from() {
        assert_eq!(styles_of_single_row_region(MergeStatus::ChangedInB), [FROM_B, RED, FROM_B]);
        assert_eq!(
            styles_of_single_row_region(MergeStatus::ChangedInC),
            [Some(CharStyle::Document(Document::C)), Some(CharStyle::Document(Document::C)), RED]
        );
        assert_eq!(styles_of_single_row_region(MergeStatus::ChangedAlike), [RED, FROM_A, FROM_A]);
        assert_eq!(styles_of_single_row_region(MergeStatus::Conflict), [RED, RED, RED]);
        assert_eq!(styles_of_single_row_region(MergeStatus::WhitespaceConflict), [RED, RED, RED]);
    }

    #[test]
    fn empty_comparison_of_empty_documents() {
        let map = RowMap::for_source(&Comparison::default(), &[], 0, 0, None, None);
        assert_eq!(Rows::Mapped(&map).len(), 0);
    }
}
