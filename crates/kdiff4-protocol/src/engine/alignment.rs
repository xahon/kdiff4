use std::ops::Range;

use super::{CompareInput, Comparison, ComparisonEngine, EngineError, Job, ManualAlignment, Region, Row, check_comparison, document_letter};

/// Checks part counts, bounds and order. Alignments that overlap or cross, or that hold only anchors, are rejected.
pub fn validate_alignments(alignments: &[ManualAlignment], line_counts: &[usize]) -> Result<(), EngineError> {
    for (index, alignment) in alignments.iter().enumerate() {
        let number = index + 1;
        if alignment.parts.len() != line_counts.len() {
            return Err(EngineError::InvalidAlignment(format!(
                "alignment {number} has {} parts, expected {}",
                alignment.parts.len(),
                line_counts.len()
            )));
        }
        for (document, (part, count)) in alignment.parts.iter().zip(line_counts).enumerate() {
            if part.start > part.end || part.end > *count {
                return Err(EngineError::InvalidAlignment(format!(
                    "alignment {number}: part {part:?} in {} is outside lines 0..{count}",
                    document_letter(document)
                )));
            }
        }
        if alignment.parts.iter().all(|part| part.is_empty()) {
            return Err(EngineError::InvalidAlignment(format!("alignment {number} has only anchors")));
        }
    }
    for (index, pair) in alignments.windows(2).enumerate() {
        for (document, (previous, next)) in pair[0].parts.iter().zip(&pair[1].parts).enumerate() {
            if previous.end <= next.start {
                continue;
            }
            let overlap = previous.start.max(next.start) < previous.end.min(next.end);
            let relation = if overlap { "overlap" } else { "cross" };
            return Err(EngineError::InvalidAlignment(format!(
                "alignments {} and {} {relation} in {}",
                index + 1,
                index + 2,
                document_letter(document)
            )));
        }
    }
    Ok(())
}

/// The segmentation helper: compares each segment without alignments and joins the results.
pub(super) fn compare_segments(engine: &dyn ComparisonEngine, input: &CompareInput<'_>, job: &Job<'_>) -> Result<Comparison, EngineError> {
    let mut regions = Vec::new();
    for segment in segments(input.alignments, &input.line_counts()) {
        job.check_canceled()?;
        let sources = input.sources.iter().zip(&segment).map(|(source, range)| source.slice(range.clone())).collect();
        let segment_input = CompareInput {
            sources,
            alignments: &[],
            options: input.options,
        };
        let comparison = engine.compare(&segment_input, job)?;
        check_comparison(&comparison, &segment_input)?;
        regions.extend(comparison.regions.into_iter().map(|region| shifted(region, &segment)));
    }
    Ok(Comparison { regions })
}

/// Per segment, one line range per source document. Segments without lines are left out.
fn segments(alignments: &[ManualAlignment], line_counts: &[usize]) -> Vec<Vec<Range<usize>>> {
    let mut segments = Vec::new();
    let mut cursor = vec![0; line_counts.len()];
    for alignment in alignments {
        segments.push(cursor.iter().zip(&alignment.parts).map(|(start, part)| start.clone()..part.start.clone()).collect());
        segments.push(alignment.parts.clone());
        cursor = alignment.parts.iter().map(|part| part.end.clone()).collect();
    }
    segments.push(cursor.iter().zip(line_counts).map(|(start, count)| start.clone()..count.clone()).collect());
    segments
        .into_iter()
        .filter(|segment: &Vec<Range<usize>>| segment.iter().any(|range| !range.is_empty()))
        .collect()
}

fn shifted(region: Region, segment: &[Range<usize>]) -> Region {
    let hunks = region
        .hunks
        .iter()
        .zip(segment)
        .map(|(hunk, range)| hunk.start + range.start..hunk.end + range.start)
        .collect();
    let rows = region
        .rows
        .into_iter()
        .map(|row| Row {
            lines: row.lines.iter().zip(segment).map(|(line, range)| line.map(|line| line + range.start)).collect(),
        })
        .collect();
    Region {
        hunks,
        status: region.status,
        rows,
    }
}

#[cfg(test)]
mod tests {
    use proptest::collection::vec;
    use proptest::prelude::*;
    use proptest::sample::select;

    use super::super::test_engine::{Fixture, SimpleEngine, with_job};
    use super::super::{HunkChange, MergeStatus, RegionStatus, run_comparison};
    use super::*;

    fn alignment(parts: &[Range<usize>]) -> ManualAlignment {
        ManualAlignment { parts: parts.to_vec() }
    }

    fn error_message(alignments: &[ManualAlignment], line_counts: &[usize]) -> String {
        match validate_alignments(alignments, line_counts) {
            Err(EngineError::InvalidAlignment(message)) => message,
            other => panic!("expected an invalid alignment, got {other:?}"),
        }
    }

    #[test]
    fn accepts_ranges_and_anchors() {
        let alignments = [alignment(&[0..1, 0..0, 2..3]), alignment(&[1..1, 0..2, 3..3]), alignment(&[1..1, 2..2, 3..4])];
        assert_eq!(validate_alignments(&alignments, &[3, 3, 4]), Ok(()));
    }

    #[test]
    fn rejects_wrong_part_count() {
        assert_eq!(error_message(&[alignment(&[0..1])], &[2, 2]), "alignment 1 has 1 parts, expected 2");
    }

    #[test]
    fn rejects_part_outside_document() {
        assert_eq!(error_message(&[alignment(&[0..1, 1..3])], &[2, 2]), "alignment 1: part 1..3 in B is outside lines 0..2");
    }

    #[test]
    fn rejects_only_anchors() {
        assert_eq!(error_message(&[alignment(&[1..1, 0..0])], &[2, 2]), "alignment 1 has only anchors");
    }

    #[test]
    fn rejects_overlap() {
        let alignments = [alignment(&[0..2, 0..1]), alignment(&[1..3, 1..2])];
        assert_eq!(error_message(&alignments, &[3, 3]), "alignments 1 and 2 overlap in A");
    }

    #[test]
    fn rejects_crossing() {
        let alignments = [alignment(&[0..1, 2..3]), alignment(&[1..2, 0..1])];
        assert_eq!(error_message(&alignments, &[3, 3]), "alignments 1 and 2 cross in B");
    }

    #[test]
    fn rejects_anchor_inside_range() {
        let alignments = [alignment(&[0..3, 0..1]), alignment(&[1..1, 1..2])];
        assert_eq!(error_message(&alignments, &[3, 3]), "alignments 1 and 2 cross in A");
    }

    #[test]
    fn cuts_at_alignments_and_drops_empty_segments() {
        let alignments = [alignment(&[0..1, 0..0]), alignment(&[1..1, 0..2])];
        assert_eq!(segments(&alignments, &[3, 2]), [vec![0..1, 0..0], vec![1..1, 0..2], vec![1..3, 2..2]]);
    }

    #[test]
    fn shifts_regions_into_document_lines() {
        let region = Region {
            hunks: vec![0..1, 0..0],
            status: RegionStatus::Diff(HunkChange::Removed),
            rows: vec![Row { lines: vec![Some(0), None] }],
        };
        let shifted = shifted(region, &[4..5, 2..2]);
        assert_eq!(shifted.hunks, [4..5, 2..2]);
        assert_eq!(shifted.rows, [Row { lines: vec![Some(4), None] }]);
    }

    fn statuses(fixture: &Fixture, alignments: &[ManualAlignment]) -> Vec<(Vec<Range<usize>>, RegionStatus)> {
        with_job(|job| run_comparison(&SimpleEngine::default(), &fixture.input(alignments), job))
            .unwrap()
            .regions
            .into_iter()
            .map(|region| (region.hunks, region.status))
            .collect()
    }

    #[test]
    fn unrelated_insertions_conflict_without_alignments() {
        let fixture = Fixture::new(&[&["Bar", "Baz"], &["Bar", "Foo", "Baz"], &["Bar", "Qux", "Baz"]]);
        let expected = [
            (vec![0..1, 0..1, 0..1], RegionStatus::Merge(MergeStatus::Unchanged)),
            (vec![1..1, 1..2, 1..2], RegionStatus::Merge(MergeStatus::Conflict)),
            (vec![1..2, 2..3, 2..3], RegionStatus::Merge(MergeStatus::Unchanged)),
        ];
        assert_eq!(statuses(&fixture, &[]), expected);
    }

    // The Foo and Qux example of Manual alignment in ARCHITECTURE.md.
    #[test]
    fn alignments_separate_unrelated_insertions() {
        let fixture = Fixture::new(&[&["Bar", "Baz"], &["Bar", "Foo", "Baz"], &["Bar", "Qux", "Baz"]]);
        let alignments = [alignment(&[1..1, 1..2, 1..1]), alignment(&[1..1, 2..2, 1..2])];
        let expected = [
            (vec![0..1, 0..1, 0..1], RegionStatus::Merge(MergeStatus::Unchanged)),
            (vec![1..1, 1..2, 1..1], RegionStatus::Merge(MergeStatus::ChangedInB)),
            (vec![1..1, 2..2, 1..2], RegionStatus::Merge(MergeStatus::ChangedInC)),
            (vec![1..2, 2..3, 2..3], RegionStatus::Merge(MergeStatus::Unchanged)),
        ];
        assert_eq!(statuses(&fixture, &alignments), expected);
    }

    #[test]
    fn two_way_alignment() {
        let fixture = Fixture::new(&[&["a", "b", "c"], &["c", "b", "a"]]);
        let alignments = [alignment(&[1..2, 1..2])];
        let expected = [
            (vec![0..1, 0..1], RegionStatus::Diff(HunkChange::Modified)),
            (vec![1..2, 1..2], RegionStatus::Diff(HunkChange::Unchanged)),
            (vec![2..3, 2..3], RegionStatus::Diff(HunkChange::Modified)),
        ];
        assert_eq!(statuses(&fixture, &alignments), expected);
    }

    const ALPHABET: [&str; 4] = ["a", "b", "c", "d"];

    /// Random documents with random non-crossing alignments: each document's cut points are sorted, so alignments keep one order everywhere.
    fn documents_with_alignments() -> impl Strategy<Value = (Vec<Vec<&'static str>>, Vec<ManualAlignment>)> {
        (2usize..=3, 0usize..4)
            .prop_flat_map(|(documents, count)| {
                vec(vec(select(ALPHABET.to_vec()), 0..12), documents).prop_flat_map(move |lines| {
                    let cuts: Vec<_> = lines.iter().map(|document| vec(0..=document.len(), count * 2)).collect();
                    (Just(lines), cuts)
                })
            })
            .prop_map(|(lines, mut cuts)| {
                cuts.iter_mut().for_each(|document| document.sort());
                let count = cuts.first().map(|document| document.len() / 2).unwrap_or(0);
                let alignments = (0..count)
                    .map(|index| ManualAlignment {
                        parts: cuts.iter().map(|document| document[index * 2]..document[index * 2 + 1]).collect(),
                    })
                    .filter(|alignment| alignment.parts.iter().any(|part| !part.is_empty()))
                    .collect();
                (lines, alignments)
            })
    }

    proptest! {
        #[test]
        fn segmented_results_pass_the_check((lines, alignments) in documents_with_alignments()) {
            let documents: Vec<&[&'static str]> = lines.iter().map(|document| document.as_slice()).collect();
            let fixture = Fixture::new(&documents);
            let result = with_job(|job| run_comparison(&SimpleEngine::default(), &fixture.input(&alignments), job));
            prop_assert!(result.is_ok(), "{result:?}");
        }
    }
}
