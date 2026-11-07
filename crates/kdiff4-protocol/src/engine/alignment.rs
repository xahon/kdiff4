use std::ops::Range;

use super::{CompareInput, Comparison, ComparisonEngine, EngineError, Job, ManualAlignment, Region, Row, check_comparison, document_letter};

/// Whether two parts of one document can not both be marked: ranges must not overlap and an anchor must not lie strictly inside a range. Anchors may share a position, and ranges and anchors may touch.
pub fn parts_collide(first: &Range<usize>, second: &Range<usize>) -> bool {
    match (first.is_empty(), second.is_empty()) {
        (true, true) => false,
        (true, false) => second.start < first.start && first.start < second.end,
        (false, true) => first.start < second.start && second.start < first.end,
        (false, false) => first.start < second.end && second.start < first.end,
    }
}


/// How two alignments sit relative to each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Relation {
    /// In every document the parts are apart, possibly touching.
    Siblings,
    /// In every document the first part lies inside the second one, and not the other way round in all of them.
    FirstInside,
    SecondInside,
    /// Equal parts in every document.
    Same,
    /// The parts of this document cross, or some documents disagree about the relation.
    Conflict(usize),
}


/// Whether a part lies inside another one: an anchor may sit anywhere in `outer` including its edges, and equal parts are inside each other.
fn part_inside(inner: &Range<usize>, outer: &Range<usize>) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}


fn relation(first: &ManualAlignment, second: &ManualAlignment) -> Relation {
    let pairs = first.parts.iter().zip(&second.parts);
    if pairs.clone().all(|(first, second)| !parts_collide(first, second)) {
        return Relation::Siblings;
    }
    let first_inside = pairs.clone().all(|(first, second)| part_inside(first, second));
    let second_inside = pairs.clone().all(|(first, second)| part_inside(second, first));
    match (first_inside, second_inside) {
        (true, true) => Relation::Same,
        (true, false) => Relation::FirstInside,
        (false, true) => Relation::SecondInside,
        (false, false) => {
            let crossing = pairs.clone().position(|(first, second)| parts_collide(first, second) && !part_inside(first, second) && !part_inside(second, first));
            let document = crossing.or_else(|| pairs.clone().position(|(first, second)| parts_collide(first, second)));
            Relation::Conflict(document.unwrap_or_default())
        }
    }
}


/// Per alignment, the index of the smallest alignment that encloses it, or `None` for a top-level one. An alignment is enclosed by another one when each of its parts lies inside the other's part in the same document and the two are not siblings (apart in every document). For alignments that passed `validate_alignments` the enclosing ones of an alignment form a chain.
pub fn alignment_tree(alignments: &[ManualAlignment]) -> Vec<Option<usize>> {
    let nested = |inner: usize, outer: usize| relation(&alignments[inner], &alignments[outer]) == Relation::FirstInside;
    (0..alignments.len())
        .map(|index| {
            let enclosing: Vec<usize> = (0..alignments.len()).filter(|outer| *outer != index && nested(index, *outer)).collect();
            enclosing.iter().copied().max_by_key(|candidate| enclosing.iter().filter(|other| nested(*candidate, **other)).count())
        })
        .collect()
}


/// The alignments of one sibling group in list order: the children of `parent`, or the top-level alignments for `None`.
pub fn siblings(tree: &[Option<usize>], parent: Option<usize>) -> Vec<usize> {
    (0..tree.len()).filter(|index| tree[*index] == parent).collect()
}


/// Checks part counts and bounds. Alignments that hold only anchors are rejected. Two alignments must be siblings (apart in every document) or one must be nested in the other (inside it in every document); parts that cross, equal alignments and alignments that documents see differently are rejected. A document may hold the parts of siblings in any order: alignments that cross in order are insets.
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
    for first in 0..alignments.len() {
        for second in first + 1..alignments.len() {
            match relation(&alignments[first], &alignments[second]) {
                Relation::Same => return Err(EngineError::InvalidAlignment(format!("alignments {} and {} are the same", first + 1, second + 1))),
                Relation::Conflict(document) => {
                    return Err(EngineError::InvalidAlignment(format!("alignments {} and {} overlap in {}", first + 1, second + 1, document_letter(document))));
                }
                Relation::Siblings | Relation::FirstInside | Relation::SecondInside => {}
            }
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

/// Per segment, one line range per source document. Segments without lines are left out. An alignment with nested alignments is cut up further inside its parts. Siblings must be in the same order in every document.
fn segments(alignments: &[ManualAlignment], line_counts: &[usize]) -> Vec<Vec<Range<usize>>> {
    let mut segments = Vec::new();
    let whole: Vec<Range<usize>> = line_counts.iter().map(|count| 0..count.clone()).collect();
    cut_group(alignments, &alignment_tree(alignments), None, &whole, &mut segments);
    segments
        .into_iter()
        .filter(|segment: &Vec<Range<usize>>| segment.iter().any(|range| !range.is_empty()))
        .collect()
}


fn cut_group(alignments: &[ManualAlignment], tree: &[Option<usize>], parent: Option<usize>, area: &[Range<usize>], segments: &mut Vec<Vec<Range<usize>>>) {
    let mut cursor: Vec<usize> = area.iter().map(|range| range.start.clone()).collect();
    for index in siblings(tree, parent) {
        let parts = &alignments[index].parts;
        segments.push(cursor.iter().zip(parts).map(|(start, part)| start.clone()..part.start.clone()).collect());
        cut_group(alignments, tree, Some(index), parts, segments);
        cursor = parts.iter().map(|part| part.end.clone()).collect();
    }
    segments.push(cursor.iter().zip(area).map(|(start, range)| start.clone()..range.end.clone()).collect());
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
        ManualAlignment::new(parts.to_vec())
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
    fn accepts_crossing_alignments() {
        let alignments = [alignment(&[0..1, 2..3]), alignment(&[1..2, 0..1])];
        assert_eq!(validate_alignments(&alignments, &[3, 3]), Ok(()));
    }

    #[test]
    fn rejects_anchor_inside_range() {
        let alignments = [alignment(&[0..3, 0..1]), alignment(&[1..1, 1..2])];
        assert_eq!(error_message(&alignments, &[3, 3]), "alignments 1 and 2 overlap in A");
    }

    #[test]
    fn rejects_overlap_of_alignments_that_are_not_next_to_each_other() {
        let alignments = [alignment(&[0..2, 0..1]), alignment(&[2..3, 1..2]), alignment(&[1..2, 2..3])];
        assert_eq!(error_message(&alignments, &[3, 3]), "alignments 1 and 3 overlap in A");
    }

    #[test]
    fn accepts_nested_alignments() {
        let alignments = [alignment(&[0..3, 0..3]), alignment(&[1..2, 1..2]), alignment(&[1..1, 1..2])];
        assert_eq!(validate_alignments(&alignments, &[3, 3]), Ok(()));
    }

    #[test]
    fn rejects_equal_alignments() {
        let alignments = [alignment(&[0..2, 1..1]), alignment(&[0..2, 1..1])];
        assert_eq!(error_message(&alignments, &[3, 3]), "alignments 1 and 2 are the same");
    }

    #[test]
    fn rejects_nesting_that_documents_see_differently() {
        let alignments = [alignment(&[0..3, 1..2]), alignment(&[1..2, 0..3])];
        assert_eq!(error_message(&alignments, &[3, 3]), "alignments 1 and 2 overlap in A");
    }

    #[test]
    fn names_the_document_where_the_parts_cross() {
        let alignments = [alignment(&[0..3, 0..2]), alignment(&[1..2, 1..3])];
        assert_eq!(error_message(&alignments, &[3, 3]), "alignments 1 and 2 overlap in B");
    }

    #[test]
    fn siblings_win_over_nesting_when_they_are_apart_everywhere() {
        let alignments = [alignment(&[0..1, 0..1]), alignment(&[1..1, 1..2])];
        assert_eq!(alignment_tree(&alignments), [None, None]);
    }

    #[test]
    fn the_tree_gives_every_alignment_its_smallest_enclosing_alignment() {
        let alignments = [alignment(&[2..3, 2..3]), alignment(&[0..5, 0..5]), alignment(&[5..6, 5..6]), alignment(&[1..4, 1..4])];
        assert_eq!(alignment_tree(&alignments), [Some(3), None, None, Some(1)]);
        assert_eq!(siblings(&alignment_tree(&alignments), None), [1, 2]);
        assert_eq!(siblings(&alignment_tree(&alignments), Some(1)), [3]);
    }

    #[test]
    fn cuts_nested_alignments_inside_their_parent() {
        let alignments = [alignment(&[1..6, 1..6]), alignment(&[2..3, 2..3]), alignment(&[4..5, 4..5])];
        let expected = [vec![0..1, 0..1], vec![1..2, 1..2], vec![2..3, 2..3], vec![3..4, 3..4], vec![4..5, 4..5], vec![5..6, 5..6], vec![6..7, 6..7]];
        assert_eq!(segments(&alignments, &[7, 7]), expected);
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

    const NESTED: [&[&str]; 2] = [&["pre", "o1", "i1", "i2", "o2", "post"], &["pre", "o1", "i1", "i2", "o2", "post"]];

    fn nested_alignments() -> [ManualAlignment; 2] {
        [alignment(&[1..5, 1..5]), alignment(&[2..4, 2..4])]
    }

    #[test]
    fn a_change_inside_an_inner_alignment_stays_inside_it() {
        let fixture = Fixture::new(&[NESTED[0], &["pre", "o1", "j1", "i2", "o2", "post"]]);
        let modified = RegionStatus::Diff(HunkChange::Modified);
        let unchanged = RegionStatus::Diff(HunkChange::Unchanged);
        let expected = [
            (vec![0..1, 0..1], unchanged),
            (vec![1..2, 1..2], unchanged),
            (vec![2..3, 2..3], modified),
            (vec![3..4, 3..4], unchanged),
            (vec![4..5, 4..5], unchanged),
            (vec![5..6, 5..6], unchanged),
        ];
        assert_eq!(statuses(&fixture, &nested_alignments()), expected);
    }

    #[test]
    fn engines_that_honor_alignments_get_nested_ones_cut_by_the_helper() {
        let fixture = Fixture::new(&NESTED);
        let engine = SimpleEngine {
            manual_alignment: true,
            ..SimpleEngine::default()
        };
        with_job(|job| run_comparison(&engine, &fixture.input(&nested_alignments()), job)).unwrap();
        let seen = engine.seen_alignments.into_inner().unwrap();
        assert_eq!(seen, [0; 5]);
    }

    // Per document the same sorted cut points, joined into pairs the same way: the pairs are siblings or nested, never crossing.
    const SHAPES: [&[(usize, usize)]; 5] = [&[(0, 3), (1, 2)], &[(0, 1), (2, 3)], &[(0, 5), (1, 2), (3, 4)], &[(0, 3), (1, 2), (4, 5)], &[(0, 5), (1, 4), (2, 3)]];

    fn documents_with_nested_alignments() -> impl Strategy<Value = (Vec<Vec<&'static str>>, Vec<ManualAlignment>)> {
        (2usize..=3, select(SHAPES.to_vec()))
            .prop_flat_map(|(documents, shape)| {
                vec(vec(select(ALPHABET.to_vec()), 0..12), documents).prop_flat_map(move |lines| {
                    let cuts: Vec<_> = lines.iter().map(|document| vec(0..=document.len(), shape.len() * 2)).collect();
                    (Just(lines), cuts, Just(shape))
                })
            })
            .prop_map(|(lines, mut cuts, shape)| {
                cuts.iter_mut().for_each(|document| document.sort());
                let mut alignments: Vec<ManualAlignment> = Vec::new();
                for (open, close) in shape {
                    let alignment = ManualAlignment::new(cuts.iter().map(|document| document[*open]..document[*close]).collect());
                    if alignment.parts.iter().any(|part| !part.is_empty()) && !alignments.contains(&alignment) {
                        alignments.push(alignment);
                    }
                }
                (lines, alignments)
            })
    }

    proptest! {
        #[test]
        fn nested_results_pass_the_check((lines, alignments) in documents_with_nested_alignments()) {
            let documents: Vec<&[&'static str]> = lines.iter().map(|document| document.as_slice()).collect();
            let fixture = Fixture::new(&documents);
            let result = with_job(|job| run_comparison(&SimpleEngine::default(), &fixture.input(&alignments), job));
            prop_assert!(result.is_ok(), "{result:?}");
        }
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
                    .map(|index| ManualAlignment::new(cuts.iter().map(|document| document[index * 2]..document[index * 2 + 1]).collect()))
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
