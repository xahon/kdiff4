use std::collections::HashSet;

use super::{CompareInput, Comparison, EngineError, Region, RegionStatus, document_letter};


/// Checks that regions cover every line of every source document exactly once and in order, that rows pair exactly the lines of their region, and that every manual alignment starts and ends at a region boundary.
pub fn check_comparison(comparison: &Comparison, input: &CompareInput<'_>) -> Result<(), EngineError> {
    let line_counts = input.line_counts();
    let mut cursor = vec![0; line_counts.len()];
    let mut boundaries = HashSet::new();
    boundaries.insert(cursor.clone());
    for (index, region) in comparison.regions.iter().enumerate() {
        check_region(index + 1, region, &cursor)?;
        cursor = region.hunks.iter().map(|hunk| hunk.end.clone()).collect();
        boundaries.insert(cursor.clone());
    }
    if cursor != line_counts {
        return Err(invalid(format!("regions end at lines {cursor:?}, the documents have {line_counts:?} lines")));
    }
    for (index, alignment) in input.alignments.iter().enumerate() {
        let starts: Vec<usize> = alignment.parts.iter().map(|part| part.start.clone()).collect();
        let ends: Vec<usize> = alignment.parts.iter().map(|part| part.end.clone()).collect();
        if !boundaries.contains(&starts) || !boundaries.contains(&ends) {
            return Err(invalid(format!("manual alignment {} does not start and end at region boundaries", index + 1)));
        }
    }
    Ok(())
}


fn check_region(number: usize, region: &Region, starts: &[usize]) -> Result<(), EngineError> {
    if region.hunks.len() != starts.len() {
        return Err(invalid(format!("region {number} has {} hunks, expected {}", region.hunks.len(), starts.len())));
    }
    for (document, (hunk, start)) in region.hunks.iter().zip(starts).enumerate() {
        if hunk.start != *start || hunk.end < hunk.start {
            return Err(invalid(format!("region {number}: hunk {} is {hunk:?}, expected it to start at line {start}", document_letter(document))));
        }
    }
    if region.hunks.iter().all(|hunk| hunk.is_empty()) {
        return Err(invalid(format!("region {number} has no lines")));
    }
    let status_fits = match region.status {
        RegionStatus::Diff(_) => starts.len() == 2,
        RegionStatus::Merge(_) => starts.len() == 3,
    };
    if !status_fits {
        return Err(invalid(format!("region {number}: status {:?} in a {}-way comparison", region.status, starts.len())));
    }
    let mut next: Vec<usize> = region.hunks.iter().map(|hunk| hunk.start.clone()).collect();
    for (row_index, row) in region.rows.iter().enumerate() {
        let row_number = row_index + 1;
        if row.lines.len() != next.len() {
            return Err(invalid(format!("region {number}, row {row_number} has {} lines, expected {}", row.lines.len(), next.len())));
        }
        if row.lines.iter().all(|line| line.is_none()) {
            return Err(invalid(format!("region {number}, row {row_number} has no lines")));
        }
        for (document, (line, expected)) in row.lines.iter().zip(next.iter_mut()).enumerate() {
            let Some(line) = line else { continue };
            if line != expected {
                return Err(invalid(format!("region {number}, row {row_number}: line {line} of {}, expected line {expected}", document_letter(document))));
            }
            *expected += 1;
        }
    }
    let ends: Vec<usize> = region.hunks.iter().map(|hunk| hunk.end.clone()).collect();
    if next != ends {
        return Err(invalid(format!("region {number}: rows hold lines up to {next:?}, the hunks end at {ends:?}")));
    }
    Ok(())
}


fn invalid(message: String) -> EngineError {
    EngineError::InvalidResult(message)
}


#[cfg(test)]
mod tests {
    use std::ops::Range;

    use super::super::test_engine::Fixture;
    use super::super::{HunkChange, ManualAlignment, MergeStatus, Row};
    use super::*;


    fn region(hunks: &[Range<usize>], rows: &[&[Option<usize>]]) -> Region {
        let status = match hunks.len() {
            2 => RegionStatus::Diff(HunkChange::Modified),
            _ => RegionStatus::Merge(MergeStatus::Conflict),
        };
        let rows = rows
            .iter()
            .map(|lines| Row { lines: lines.to_vec() })
            .collect();
        Region { hunks: hunks.to_vec(), status, rows }
    }


    fn check(regions: Vec<Region>, alignments: &[ManualAlignment]) -> Result<(), String> {
        let fixture = Fixture::new(&[&["a", "b"], &["a", "c", "d"]]);
        match check_comparison(&Comparison { regions }, &fixture.input(alignments)) {
            Ok(()) => Ok(()),
            Err(EngineError::InvalidResult(message)) => Err(message),
            Err(other) => panic!("unexpected error {other:?}"),
        }
    }


    fn valid_regions() -> Vec<Region> {
        vec![region(&[0..1, 0..1], &[&[Some(0), Some(0)]]), region(&[1..2, 1..3], &[&[Some(1), Some(1)], &[None, Some(2)]])]
    }


    #[test]
    fn accepts_valid_comparison() {
        assert_eq!(check(valid_regions(), &[]), Ok(()));
    }


    #[test]
    fn rejects_lines_left_over() {
        let regions = vec![region(&[0..1, 0..1], &[&[Some(0), Some(0)]])];
        assert_eq!(check(regions, &[]), Err("regions end at lines [1, 1], the documents have [2, 3] lines".to_string()));
    }


    #[test]
    fn rejects_gap_between_hunks() {
        let regions = vec![region(&[0..1, 0..1], &[&[Some(0), Some(0)]]), region(&[1..2, 2..3], &[&[Some(1), Some(2)]])];
        assert_eq!(check(regions, &[]), Err("region 2: hunk B is 2..3, expected it to start at line 1".to_string()));
    }


    #[test]
    fn rejects_wrong_hunk_count() {
        let regions = vec![region(&[0..2, 0..3, 0..0], &[])];
        assert_eq!(check(regions, &[]), Err("region 1 has 3 hunks, expected 2".to_string()));
    }


    #[test]
    fn rejects_empty_region() {
        let mut regions = valid_regions();
        regions.insert(1, region(&[1..1, 1..1], &[]));
        assert_eq!(check(regions, &[]), Err("region 2 has no lines".to_string()));
    }


    #[test]
    fn rejects_status_of_other_mode() {
        let mut regions = valid_regions();
        regions[0].status = RegionStatus::Merge(MergeStatus::Unchanged);
        assert_eq!(check(regions, &[]), Err("region 1: status Merge(Unchanged) in a 2-way comparison".to_string()));
    }


    #[test]
    fn rejects_lines_out_of_order() {
        let mut regions = valid_regions();
        regions[1] = region(&[1..2, 1..3], &[&[Some(1), Some(2)], &[None, Some(1)]]);
        assert_eq!(check(regions, &[]), Err("region 2, row 1: line 2 of B, expected line 1".to_string()));
    }


    #[test]
    fn rejects_lines_missing_from_rows() {
        let mut regions = valid_regions();
        regions[1] = region(&[1..2, 1..3], &[&[Some(1), Some(1)]]);
        assert_eq!(check(regions, &[]), Err("region 2: rows hold lines up to [2, 2], the hunks end at [2, 3]".to_string()));
    }


    #[test]
    fn rejects_void_row() {
        let mut regions = valid_regions();
        regions[1] = region(&[1..2, 1..3], &[&[Some(1), Some(1)], &[None, None], &[None, Some(2)]]);
        assert_eq!(check(regions, &[]), Err("region 2, row 2 has no lines".to_string()));
    }


    #[test]
    fn rejects_alignment_inside_region() {
        let alignments = [ManualAlignment { parts: vec![1..2, 2..3] }];
        assert_eq!(check(valid_regions(), &alignments), Err("manual alignment 1 does not start and end at region boundaries".to_string()));
        let alignments = [ManualAlignment { parts: vec![1..2, 1..3] }];
        assert_eq!(check(valid_regions(), &alignments), Ok(()));
    }
}
