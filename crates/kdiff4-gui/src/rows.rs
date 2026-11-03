use kdiff4_protocol::engine::{Comparison, HunkChange, MergeStatus, Region, RegionStatus};
use kdiff4_protocol::{Choice, Document, RegionMerge};

use crate::editing::Position;


/// Background of a pane's rows in a changed region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shade {
    Removed,
    Added,
    Modified,
    Conflict,
}


/// Where a line of D came from: a chosen hunk, or a hand edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Source(Document),
    Modified,
}


/// One row of a pane: its line, or `None` for a void, the region it belongs to and the region's shade in this pane. In D, `mark` says where the line came from, and `conflict_marker` is the placeholder of an unresolved conflict.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PaneRow {
    pub line: Option<usize>,
    pub shade: Option<Shade>,
    pub region: Option<usize>,
    pub mark: Option<Mark>,
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
    /// `heights`, when given, pads every region with void rows to that many rows, so panes line up with D. Falls back to one unshaded row per line when the comparison doesn't hold every line of the document exactly once, in order.
    pub fn for_source(comparison: &Comparison, document_index: usize, line_count: usize, heights: Option<&[usize]>) -> RowMap {
        let mut rows = Vec::new();
        let mut row_of_line = Vec::with_capacity(line_count);
        let mut region_starts = Vec::with_capacity(comparison.regions.len());
        for (index, region) in comparison.regions.iter().enumerate() {
            region_starts.push(rows.len());
            let shade = shade(region, document_index);
            for row in &region.rows {
                let line = row.lines.get(document_index).cloned().flatten();
                if let Some(line) = line {
                    if line != row_of_line.len() {
                        return RowMap::fallback(line_count, "lines out of order");
                    }
                    row_of_line.push(rows.len());
                }
                rows.push(PaneRow { line, shade, region: Some(index), ..PaneRow::default() });
            }
            pad(&mut rows, index, region_starts[index], heights);
        }
        if row_of_line.len() != line_count {
            return RowMap::fallback(line_count, "lines missing");
        }
        RowMap { rows, row_of_line, region_starts }
    }


    /// D's rows: its lines, region by region, each marked with where it came from, and a conflict-shaded placeholder row for each unresolved conflict, which has no lines. `heights` pads regions like `for_source`. Lines past the regions (the empty line after D's last line break) follow as plain rows.
    pub fn for_output(merge: &[RegionMerge], regions: &[Region], line_count: usize, heights: Option<&[usize]>) -> RowMap {
        let mut rows = Vec::with_capacity(line_count + merge.len());
        let mut row_of_line = Vec::with_capacity(line_count);
        let mut region_starts = Vec::with_capacity(merge.len());
        for (index, state) in merge.iter().enumerate() {
            region_starts.push(rows.len());
            if state.output_lines == 0 && state.is_unresolved() {
                rows.push(PaneRow { shade: Some(Shade::Conflict), region: Some(index), conflict_marker: true, ..PaneRow::default() });
            }
            let marks = marks(state, regions.get(index));
            for mark in marks {
                if row_of_line.len() >= line_count {
                    return RowMap::fallback(line_count, "more region lines than D has");
                }
                row_of_line.push(rows.len());
                rows.push(PaneRow { line: Some(row_of_line.len() - 1), region: Some(index), mark, ..PaneRow::default() });
            }
            pad(&mut rows, index, region_starts[index], heights);
        }
        while row_of_line.len() < line_count {
            row_of_line.push(rows.len());
            rows.push(PaneRow { line: Some(row_of_line.len() - 1), ..PaneRow::default() });
        }
        RowMap { rows, row_of_line, region_starts }
    }


    fn fallback(line_count: usize, reason: &str) -> RowMap {
        log::warn!("the comparison doesn't fit a document of {line_count} lines ({reason}); showing it without rows");
        let rows = (0..line_count)
            .map(|line| PaneRow { line: Some(line), ..PaneRow::default() })
            .collect();
        RowMap { rows, row_of_line: (0..line_count).collect(), region_starts: Vec::new() }
    }
}


/// Pads the rows of region `index`, which started at row `start`, with void rows up to its height.
fn pad(rows: &mut Vec<PaneRow>, index: usize, start: usize, heights: Option<&[usize]>) {
    let height = heights.and_then(|heights| heights.get(index)).cloned().unwrap_or(0);
    while rows.len() - start < height {
        rows.push(PaneRow { region: Some(index), ..PaneRow::default() });
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
        let length = source_index(document.clone())
            .and_then(|index| region.hunks.get(index))
            .map_or(0, |hunk| hunk.len());
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


/// Per region, the rows every pane gives it when D lines up with the source panes: the most any pane needs, and one for the placeholder of an unresolved conflict.
pub fn aligned_heights(regions: &[Region], merge: &[RegionMerge]) -> Vec<usize> {
    regions
        .iter()
        .enumerate()
        .map(|(index, region)| {
            let output = merge.get(index).map_or(0, |state| if state.output_lines == 0 && state.is_unresolved() { 1 } else { state.output_lines });
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
            Rows::PerLine(count) => (row < *count).then_some(PaneRow { line: Some(row), ..PaneRow::default() }),
            Rows::Mapped(map) => map.rows.get(row).cloned(),
        }
    }


    /// The row of `line`; a line past the end maps to the last row.
    pub fn row_of_line(&self, line: usize) -> usize {
        match self {
            Rows::PerLine(count) => line.min(count.saturating_sub(1)),
            Rows::Mapped(map) => map
                .row_of_line
                .get(line)
                .or(map.row_of_line.last())
                .cloned()
                .unwrap_or(0),
        }
    }


    /// The first line at or below `row`, so a click on a void lands on the text after it.
    pub fn line_at_or_after(&self, row: usize) -> Option<usize> {
        match self {
            Rows::PerLine(count) => (row < *count).then_some(row),
            Rows::Mapped(map) => map
                .rows
                .iter()
                .skip(row)
                .find_map(|row| row.line),
        }
    }


    /// The row region `region` starts at, clamped to the rows.
    pub fn region_start(&self, region: usize) -> Option<usize> {
        match self {
            Rows::PerLine(_) => None,
            Rows::Mapped(map) => map
                .region_starts
                .get(region)
                .map(|row| (*row).min(map.rows.len().saturating_sub(1))),
        }
    }


    /// The region of `row`, if it has one.
    pub fn region_of_row(&self, row: usize) -> Option<usize> {
        self.row(row).and_then(|pane_row| pane_row.region)
    }


    /// Whether `row` is the first or the last row of its region.
    pub fn region_edges(&self, row: usize) -> (bool, bool) {
        let region = self.region_of_row(row);
        let first = row == 0 || self.region_of_row(row - 1) != region;
        let last = self.region_of_row(row + 1) != region;
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


/// How a region looks in one pane. A shows what B or C did to it; B and C show their own change. Voids get no text, so a pane is not shaded for lines it doesn't have.
fn shade(region: &Region, document_index: usize) -> Option<Shade> {
    let has_lines = region.hunks.get(document_index).is_some_and(|hunk| !hunk.is_empty());
    let a_has_lines = region.hunks.first().is_some_and(|hunk| !hunk.is_empty());
    let change_of = |other: usize| {
        let other_has_lines = region.hunks.get(other).is_some_and(|hunk| !hunk.is_empty());
        match (a_has_lines, other_has_lines) {
            (false, _) => HunkChange::Added,
            (true, false) => HunkChange::Removed,
            (true, true) => HunkChange::Modified,
        }
    };
    let change = match region.status {
        RegionStatus::Diff(HunkChange::Unchanged) | RegionStatus::Merge(MergeStatus::Unchanged) => return None,
        RegionStatus::Merge(MergeStatus::Conflict | MergeStatus::WhitespaceConflict) => return has_lines.then_some(Shade::Conflict),
        RegionStatus::Diff(change) => change,
        RegionStatus::Merge(MergeStatus::ChangedInB | MergeStatus::ChangedAlike) => change_of(1),
        RegionStatus::Merge(MergeStatus::ChangedInC) => change_of(2),
    };
    let changed_here = match region.status {
        RegionStatus::Merge(MergeStatus::ChangedInB) => document_index == 0 || document_index == 1,
        RegionStatus::Merge(MergeStatus::ChangedInC) => document_index == 0 || document_index == 2,
        _ => true,
    };
    if !changed_here || !has_lines {
        return None;
    }
    match change {
        HunkChange::Unchanged => None,
        HunkChange::Added => Some(Shade::Added),
        HunkChange::Removed => Some(Shade::Removed),
        HunkChange::Modified => Some(Shade::Modified),
    }
}


#[cfg(test)]
mod tests {
    use std::ops::Range;

    use kdiff4_protocol::engine::Row;

    use super::*;


    fn region(hunks: &[Range<usize>], status: RegionStatus, rows: &[&[Option<usize>]]) -> Region {
        let rows = rows
            .iter()
            .map(|lines| Row { lines: lines.to_vec() })
            .collect();
        Region { hunks: hunks.to_vec(), status, rows }
    }


    fn shades(region: &Region) -> Vec<Option<Shade>> {
        (0..region.hunks.len())
            .map(|document| shade(region, document))
            .collect()
    }


    fn line(line: usize, region: usize) -> PaneRow {
        PaneRow { line: Some(line), region: Some(region), ..PaneRow::default() }
    }


    fn void(region: usize) -> PaneRow {
        PaneRow { region: Some(region), ..PaneRow::default() }
    }


    #[test]
    fn two_way_shades() {
        let unchanged = region(&[0..1, 0..1], RegionStatus::Diff(HunkChange::Unchanged), &[]);
        assert_eq!(shades(&unchanged), [None, None]);
        let added = region(&[0..0, 0..1], RegionStatus::Diff(HunkChange::Added), &[]);
        assert_eq!(shades(&added), [None, Some(Shade::Added)]);
        let removed = region(&[0..1, 0..0], RegionStatus::Diff(HunkChange::Removed), &[]);
        assert_eq!(shades(&removed), [Some(Shade::Removed), None]);
        let modified = region(&[0..1, 0..2], RegionStatus::Diff(HunkChange::Modified), &[]);
        assert_eq!(shades(&modified), [Some(Shade::Modified), Some(Shade::Modified)]);
    }


    #[test]
    fn three_way_shades_follow_the_changed_side() {
        let in_b = region(&[0..1, 0..2, 0..1], RegionStatus::Merge(MergeStatus::ChangedInB), &[]);
        assert_eq!(shades(&in_b), [Some(Shade::Modified), Some(Shade::Modified), None]);
        let removed_in_c = region(&[0..1, 0..1, 0..0], RegionStatus::Merge(MergeStatus::ChangedInC), &[]);
        assert_eq!(shades(&removed_in_c), [Some(Shade::Removed), None, None]);
        let added_alike = region(&[0..0, 0..1, 0..1], RegionStatus::Merge(MergeStatus::ChangedAlike), &[]);
        assert_eq!(shades(&added_alike), [None, Some(Shade::Added), Some(Shade::Added)]);
    }


    #[test]
    fn conflicts_shade_every_pane_with_lines() {
        let conflict = region(&[0..0, 0..1, 0..2], RegionStatus::Merge(MergeStatus::Conflict), &[]);
        assert_eq!(shades(&conflict), [None, Some(Shade::Conflict), Some(Shade::Conflict)]);
        let whitespace = region(&[0..1, 0..1, 0..1], RegionStatus::Merge(MergeStatus::WhitespaceConflict), &[]);
        assert_eq!(shades(&whitespace), [Some(Shade::Conflict); 3]);
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
        let map = RowMap::for_source(&two_way_comparison(), 0, 2, None);
        let rows = Rows::Mapped(&map);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows.row(0), Some(line(0, 0)));
        assert_eq!(rows.row(1), Some(void(1)));
        assert_eq!(rows.row(3), Some(line(1, 2)));
        assert_eq!(rows.row(4), None);
        assert_eq!(rows.row_of_line(1), 3);
        let map = RowMap::for_source(&two_way_comparison(), 1, 4, None);
        assert_eq!(Rows::Mapped(&map).row(2), Some(PaneRow { shade: Some(Shade::Added), ..line(2, 1) }));
    }


    #[test]
    fn regions_know_their_rows() {
        let map = RowMap::for_source(&two_way_comparison(), 0, 2, None);
        let rows = Rows::Mapped(&map);
        assert_eq!((rows.region_start(0), rows.region_start(1), rows.region_start(2)), (Some(0), Some(1), Some(3)));
        assert_eq!(rows.region_start(3), None);
        assert_eq!(rows.region_edges(0), (true, true));
        assert_eq!(rows.region_edges(1), (true, false));
        assert_eq!(rows.region_edges(2), (false, true));
        assert_eq!(Rows::PerLine(3).region_start(0), None);
    }


    #[test]
    fn heights_pad_regions_with_voids() {
        let map = RowMap::for_source(&two_way_comparison(), 0, 2, Some(&[2, 2, 1]));
        let rows = Rows::Mapped(&map);
        assert_eq!(rows.len(), 5);
        assert_eq!(rows.row(1), Some(void(0)));
        assert_eq!(rows.row_of_line(1), 4);
        assert_eq!(rows.region_start(2), Some(4));
    }


    #[test]
    fn clicks_on_voids_go_to_the_next_line() {
        let map = RowMap::for_source(&two_way_comparison(), 0, 2, None);
        let rows = Rows::Mapped(&map);
        assert_eq!(rows.line_at_or_after(1), Some(1));
        assert_eq!(rows.line_at_or_after(3), Some(1));
        assert_eq!(rows.line_at_or_after(4), None);
    }


    #[test]
    fn caret_can_sit_on_a_void_row() {
        let map = RowMap::for_source(&two_way_comparison(), 0, 2, None);
        let rows = Rows::Mapped(&map);
        let end = Position { line: 1, column: 1 };
        assert_eq!(rows.caret_at_row(0, 3, end), (Position { line: 0, column: 3 }, None));
        assert_eq!(rows.caret_at_row(2, 3, end), (Position { line: 1, column: 0 }, Some(2)));
        assert_eq!(rows.caret_at_row(9, 0, end), (Position { line: 1, column: 0 }, None));
        let trailing = RowMap::for_source(&two_way_comparison(), 0, 2, Some(&[1, 2, 2]));
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
        let map = RowMap::for_source(&two_way_comparison(), 0, 2, None);
        assert_eq!(Rows::Mapped(&map).row_of_line(9), 3);
        assert_eq!(Rows::Mapped(&RowMap::default()).row_of_line(0), 0);
        assert_eq!(Rows::PerLine(3).row_of_line(9), 2);
        assert_eq!(Rows::PerLine(0).row_of_line(0), 0);
    }


    #[test]
    fn per_line_rows() {
        let rows = Rows::PerLine(2);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows.row(1), Some(PaneRow { line: Some(1), ..PaneRow::default() }));
        assert_eq!(rows.row(2), None);
        assert_eq!(rows.line_at_or_after(1), Some(1));
        assert_eq!(rows.line_at_or_after(2), None);
    }


    #[test]
    fn comparison_that_does_not_fit_falls_back_to_lines() {
        let map = RowMap::for_source(&two_way_comparison(), 0, 5, None);
        assert_eq!(Rows::Mapped(&map).len(), 5);
        assert_eq!(Rows::Mapped(&map).row(4), Some(PaneRow { line: Some(4), ..PaneRow::default() }));
        let mut shuffled = two_way_comparison();
        shuffled.regions.swap(0, 2);
        let map = RowMap::for_source(&shuffled, 0, 2, None);
        assert_eq!(Rows::Mapped(&map).row_of_line(1), 1);
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
    fn output_rows_show_unresolved_conflicts_as_placeholders() {
        let regions = [conflict_region(&[0..1, 0..1, 0..1], 1), conflict_region(&[1..2, 1..2, 1..2], 1), conflict_region(&[2..2, 2..2, 2..2], 0), conflict_region(&[2..4, 2..4, 2..4], 2)];
        let merge = [state(Choice::Hunks(vec![Document::A]), false, 1), state(Choice::Unresolved, false, 0), state(Choice::Hunks(Vec::new()), false, 0), state(Choice::Hunks(vec![Document::B]), false, 2)];
        let map = RowMap::for_output(&merge, &regions, 4, None);
        let rows = Rows::Mapped(&map);
        assert_eq!(rows.len(), 5);
        assert_eq!(rows.row(0), Some(PaneRow { mark: Some(Mark::Source(Document::A)), ..line(0, 0) }));
        assert_eq!(rows.row(1), Some(PaneRow { shade: Some(Shade::Conflict), conflict_marker: true, ..void(1) }));
        assert_eq!(rows.row(2), Some(PaneRow { mark: Some(Mark::Source(Document::B)), ..line(1, 3) }));
        assert_eq!(rows.row(4), Some(PaneRow { line: Some(3), ..PaneRow::default() }));
        assert_eq!(rows.row_of_line(3), 4);
        assert_eq!(rows.line_at_or_after(1), Some(1));
        assert_eq!(rows.region_start(2), Some(2));
    }


    #[test]
    fn output_marks_follow_the_choice_order_and_hand_edits() {
        let regions = [conflict_region(&[0..1, 0..2, 0..1], 2), conflict_region(&[1..2, 2..3, 1..2], 1)];
        let merge = [state(Choice::Hunks(vec![Document::C, Document::B]), false, 3), state(Choice::Hunks(vec![Document::B]), true, 1)];
        let map = RowMap::for_output(&merge, &regions, 4, None);
        let marks: Vec<Option<Mark>> = (0..4).map(|row| Rows::Mapped(&map).row(row).unwrap().mark).collect();
        assert_eq!(marks, [Some(Mark::Source(Document::C)), Some(Mark::Source(Document::B)), Some(Mark::Source(Document::B)), Some(Mark::Modified)]);
    }


    #[test]
    fn output_marks_are_unknown_when_counts_do_not_match() {
        let regions = [conflict_region(&[0..1, 0..1], 1)];
        let map = RowMap::for_output(&[state(Choice::Hunks(vec![Document::B]), false, 2)], &regions, 2, None);
        assert_eq!(Rows::Mapped(&map).row(0).unwrap().mark, None);
    }


    #[test]
    fn aligned_heights_fit_every_pane() {
        let regions = [conflict_region(&[0..1, 0..1], 1), conflict_region(&[1..3, 1..3], 2), conflict_region(&[3..3, 3..3], 0)];
        let merge = [state(Choice::Hunks(vec![Document::A, Document::B]), false, 2), state(Choice::Hunks(vec![Document::A]), false, 2), state(Choice::Unresolved, false, 0)];
        let heights = aligned_heights(&regions, &merge);
        assert_eq!(heights, [2, 2, 1]);
        let map = RowMap::for_output(&merge, &regions, 4, Some(&heights));
        assert_eq!(Rows::Mapped(&map).len(), 5);
        assert_eq!(aligned_heights(&regions, &[]), [1, 2, 0]);
    }


    #[test]
    fn output_rows_that_do_not_fit_fall_back_to_lines() {
        let map = RowMap::for_output(&[state(Choice::Unresolved, true, 3)], &[], 2, None);
        assert_eq!(Rows::Mapped(&map).len(), 2);
        assert_eq!(Rows::Mapped(&map).row(1), Some(PaneRow { line: Some(1), ..PaneRow::default() }));
    }


    #[test]
    fn empty_comparison_of_empty_documents() {
        let map = RowMap::for_source(&Comparison::default(), 0, 0, None);
        assert_eq!(Rows::Mapped(&map).len(), 0);
    }
}
