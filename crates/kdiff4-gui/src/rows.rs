use kdiff4_protocol::RegionMerge;
use kdiff4_protocol::engine::{Comparison, HunkChange, MergeStatus, Region, RegionStatus};


/// Background of a pane's rows in a changed region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shade {
    Removed,
    Added,
    Modified,
    Conflict,
}


/// One row of a pane: its line, or `None` for a void, and the region's shade in this pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneRow {
    pub line: Option<usize>,
    pub shade: Option<Shade>,
}


/// The rows of one source pane, built from the comparison.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowMap {
    rows: Vec<PaneRow>,
    row_of_line: Vec<usize>,
}


impl RowMap {
    /// Falls back to one unshaded row per line when the comparison doesn't hold every line of the document exactly once, in order.
    pub fn for_source(comparison: &Comparison, document_index: usize, line_count: usize) -> RowMap {
        let mut rows = Vec::new();
        let mut row_of_line = Vec::with_capacity(line_count);
        for region in &comparison.regions {
            let shade = shade(region, document_index);
            for row in &region.rows {
                let line = row.lines.get(document_index).cloned().flatten();
                if let Some(line) = line {
                    if line != row_of_line.len() {
                        return RowMap::fallback(line_count, "lines out of order");
                    }
                    row_of_line.push(rows.len());
                }
                rows.push(PaneRow { line, shade });
            }
        }
        if row_of_line.len() != line_count {
            return RowMap::fallback(line_count, "lines missing");
        }
        RowMap { rows, row_of_line }
    }


    /// D's rows: its lines, region by region, with a conflict-shaded void row for each unresolved conflict, which has no lines. Lines past the regions (the empty line after D's last line break) follow as plain rows.
    pub fn for_output(merge: &[RegionMerge], line_count: usize) -> RowMap {
        let mut rows = Vec::with_capacity(line_count + merge.len());
        let mut row_of_line = Vec::with_capacity(line_count);
        for state in merge {
            if state.output_lines == 0 && state.is_unresolved() {
                rows.push(PaneRow { line: None, shade: Some(Shade::Conflict) });
                continue;
            }
            for _ in 0..state.output_lines {
                if row_of_line.len() >= line_count {
                    return RowMap::fallback(line_count, "more region lines than D has");
                }
                row_of_line.push(rows.len());
                rows.push(PaneRow { line: Some(row_of_line.len() - 1), shade: None });
            }
        }
        while row_of_line.len() < line_count {
            row_of_line.push(rows.len());
            rows.push(PaneRow { line: Some(row_of_line.len() - 1), shade: None });
        }
        RowMap { rows, row_of_line }
    }


    fn fallback(line_count: usize, reason: &str) -> RowMap {
        log::warn!("the comparison doesn't fit a document of {line_count} lines ({reason}); showing it without rows");
        let rows = (0..line_count)
            .map(|line| PaneRow { line: Some(line), shade: None })
            .collect();
        RowMap { rows, row_of_line: (0..line_count).collect() }
    }
}


/// The rows a text view draws: one per line, or the rows of a source pane.
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
            Rows::PerLine(count) => (row < *count).then_some(PaneRow { line: Some(row), shade: None }),
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
        let map = RowMap::for_source(&two_way_comparison(), 0, 2);
        let rows = Rows::Mapped(&map);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows.row(0), Some(PaneRow { line: Some(0), shade: None }));
        assert_eq!(rows.row(1), Some(PaneRow { line: None, shade: None }));
        assert_eq!(rows.row(3), Some(PaneRow { line: Some(1), shade: None }));
        assert_eq!(rows.row(4), None);
        assert_eq!(rows.row_of_line(1), 3);
        let map = RowMap::for_source(&two_way_comparison(), 1, 4);
        assert_eq!(Rows::Mapped(&map).row(2), Some(PaneRow { line: Some(2), shade: Some(Shade::Added) }));
    }


    #[test]
    fn clicks_on_voids_go_to_the_next_line() {
        let map = RowMap::for_source(&two_way_comparison(), 0, 2);
        let rows = Rows::Mapped(&map);
        assert_eq!(rows.line_at_or_after(1), Some(1));
        assert_eq!(rows.line_at_or_after(3), Some(1));
        assert_eq!(rows.line_at_or_after(4), None);
    }


    #[test]
    fn row_of_line_past_the_end_is_the_last_row() {
        let map = RowMap::for_source(&two_way_comparison(), 0, 2);
        assert_eq!(Rows::Mapped(&map).row_of_line(9), 3);
        assert_eq!(Rows::Mapped(&RowMap::default()).row_of_line(0), 0);
        assert_eq!(Rows::PerLine(3).row_of_line(9), 2);
        assert_eq!(Rows::PerLine(0).row_of_line(0), 0);
    }


    #[test]
    fn per_line_rows() {
        let rows = Rows::PerLine(2);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows.row(1), Some(PaneRow { line: Some(1), shade: None }));
        assert_eq!(rows.row(2), None);
        assert_eq!(rows.line_at_or_after(1), Some(1));
        assert_eq!(rows.line_at_or_after(2), None);
    }


    #[test]
    fn comparison_that_does_not_fit_falls_back_to_lines() {
        let map = RowMap::for_source(&two_way_comparison(), 0, 5);
        assert_eq!(Rows::Mapped(&map).len(), 5);
        assert_eq!(Rows::Mapped(&map).row(4), Some(PaneRow { line: Some(4), shade: None }));
        let mut shuffled = two_way_comparison();
        shuffled.regions.swap(0, 2);
        let map = RowMap::for_source(&shuffled, 0, 2);
        assert_eq!(Rows::Mapped(&map).row_of_line(1), 1);
    }


    fn state(output_lines: usize, unresolved: bool) -> RegionMerge {
        let choice = if unresolved { kdiff4_protocol::Choice::Unresolved } else { kdiff4_protocol::Choice::Hunks(Vec::new()) };
        RegionMerge { choice, edited: false, output_lines }
    }


    #[test]
    fn output_rows_show_unresolved_conflicts_as_placeholders() {
        let map = RowMap::for_output(&[state(1, false), state(0, true), state(0, false), state(2, false)], 4);
        let rows = Rows::Mapped(&map);
        assert_eq!(rows.len(), 5);
        assert_eq!(rows.row(0), Some(PaneRow { line: Some(0), shade: None }));
        assert_eq!(rows.row(1), Some(PaneRow { line: None, shade: Some(Shade::Conflict) }));
        assert_eq!(rows.row(2), Some(PaneRow { line: Some(1), shade: None }));
        assert_eq!(rows.row(4), Some(PaneRow { line: Some(3), shade: None }));
        assert_eq!(rows.row_of_line(3), 4);
        assert_eq!(rows.line_at_or_after(1), Some(1));
    }


    #[test]
    fn output_rows_that_do_not_fit_fall_back_to_lines() {
        let map = RowMap::for_output(&[state(3, false)], 2);
        assert_eq!(Rows::Mapped(&map).len(), 2);
        assert_eq!(Rows::Mapped(&map).row(1), Some(PaneRow { line: Some(1), shade: None }));
    }


    #[test]
    fn empty_comparison_of_empty_documents() {
        let map = RowMap::for_source(&Comparison::default(), 0, 0);
        assert_eq!(Rows::Mapped(&map).len(), 0);
    }
}
