use std::cmp::Reverse;
use std::ops::Range;

use super::{CompareInput, Comparison, ComparisonEngine, EngineCapabilities, EngineError, Job, ManualAlignment, Region, Row, SourceLines, Token, alignment_tree, check_comparison, compare_aligned, document_letter, siblings};


/// The alignment indices of one sibling group (see `siblings`) in the order their parts have in `document`. Parts at one position keep the order of the list.
pub fn sibling_order(alignments: &[ManualAlignment], tree: &[Option<usize>], parent: Option<usize>, document: usize) -> Vec<usize> {
    let position = |index: &usize| {
        let part = alignments[*index].parts.get(document).cloned().unwrap_or(0..0);
        (part.start, part.end, *index)
    };
    let mut order = siblings(tree, parent);
    order.sort_by_key(position);
    order
}


/// The alignment indices of the list in the order their parts have in `document`: each alignment is followed by the alignments nested in it, and every sibling group is in the order of its parts. For alignments that are not nested that is the order of the parts.
pub fn document_order(alignments: &[ManualAlignment], document: usize) -> Vec<usize> {
    let tree = alignment_tree(alignments);
    let mut order = Vec::with_capacity(alignments.len());
    push_group_order(alignments, &tree, None, document, &mut order);
    order
}


fn push_group_order(alignments: &[ManualAlignment], tree: &[Option<usize>], parent: Option<usize>, document: usize, order: &mut Vec<usize>) {
    for index in sibling_order(alignments, tree, parent, document) {
        order.push(index);
        push_group_order(alignments, tree, Some(index), document, order);
    }
}


/// The sibling groups of a tree: the top level and the children of every alignment.
fn groups(tree: &[Option<usize>]) -> impl Iterator<Item = Option<usize>> {
    std::iter::once(None).chain((0..tree.len()).map(Some))
}


/// Whether some document holds the alignments of a sibling group in another order than the list.
pub(super) fn has_insets(alignments: &[ManualAlignment]) -> bool {
    let documents = alignments.first().map_or(0, |alignment| alignment.parts.len());
    let tree = alignment_tree(alignments);
    groups(&tree).any(|parent| {
        let members = siblings(&tree, parent);
        (0..documents).any(|document| sibling_order(alignments, &tree, parent, document) != members)
    })
}


/// Per alignment of the list, for a part that `document` shows away from its place: the index of the alignment whose part is shown in the place it had. This is decided within each sibling group. The parts that stay are a longest run of the group that already has the order of the list, and of several the one with the alignments listed first; the others are the moved ones. The parts in between only follow along when the places are exchanged.
pub fn moved_parts(alignments: &[ManualAlignment], document: usize) -> Vec<Option<usize>> {
    let tree = alignment_tree(alignments);
    let mut moved = vec![None; alignments.len()];
    for parent in groups(&tree) {
        let members = siblings(&tree, parent);
        let order = sibling_order(alignments, &tree, parent, document);
        let kept = longest_increasing(&order);
        for (slot, index) in order.iter().enumerate() {
            if !kept[slot] {
                moved[*index] = Some(members[slot]);
            }
        }
    }
    moved
}


/// Which entries of the permutation `order` are in a longest increasing run of it. Of several, the run whose values are the smallest stays.
fn longest_increasing(order: &[usize]) -> Vec<bool> {
    let mut length = vec![1; order.len()];
    let mut previous: Vec<Option<usize>> = vec![None; order.len()];
    for end in 0..order.len() {
        for before in (0..end).filter(|before| order[*before] < order[end]) {
            let longer = length[before] + 1 > length[end];
            let smaller = length[before] + 1 == length[end] && previous[end].is_some_and(|current| order[before] < order[current]);
            if longer || smaller {
                length[end] = length[before] + 1;
                previous[end] = Some(before);
            }
        }
    }
    let mut kept = vec![false; order.len()];
    let mut cursor = (0..order.len()).max_by_key(|end| (length[*end], Reverse(order[*end])));
    while let Some(index) = cursor {
        kept[index] = true;
        cursor = previous[index];
    }
    kept
}


/// The source documents in display order. Per sibling group, a document that holds its alignments in another order than the list keeps the text between their parts where it is and exchanges the parts between the places they have: the place of the `n`-th part in the document shows the part of the `n`-th alignment of the group. A part carries its whole content, nested alignments included, and those are laid out the same way inside it. So every document has the siblings in the order of the list, and no text is added or lost between them.
pub(super) struct Layout {
    /// Per document, the document line of every line in display order.
    lines: Vec<Vec<usize>>,
    /// Per document, the place in display order of every document line.
    positions: Vec<Vec<usize>>,
    /// The alignments with their parts in display order.
    pub alignments: Vec<ManualAlignment>,
}

impl Layout {
    /// For alignments that passed `validate_alignments`.
    pub fn new(alignments: &[ManualAlignment], line_counts: &[usize]) -> Layout {
        let tree = alignment_tree(alignments);
        let mut lines = Vec::with_capacity(line_counts.len());
        let mut parts = vec![Vec::with_capacity(line_counts.len()); alignments.len()];
        for (document, count) in line_counts.iter().enumerate() {
            let (document_lines, document_parts) = document_layout(alignments, &tree, document, *count);
            lines.push(document_lines);
            for (alignment, part) in parts.iter_mut().zip(document_parts) {
                alignment.push(part);
            }
        }
        let positions = lines
            .iter()
            .zip(line_counts)
            .map(|(document_lines, count)| {
                let mut positions = vec![0; *count];
                for (position, line) in document_lines.iter().enumerate() {
                    positions[*line] = position;
                }
                positions
            })
            .collect();
        Layout {
            lines,
            positions,
            alignments: parts.into_iter().map(ManualAlignment::new).collect(),
        }
    }

    /// The places in display order where the next line is not the line after the previous one: a hunk must not reach over them.
    fn seams(&self, document: usize) -> Vec<usize> {
        let lines = &self.lines[document];
        (1..lines.len()).filter(|position| lines[*position] != lines[*position - 1] + 1).collect()
    }

    /// The hunk of the document lines in display order; an empty one sits after the document line that precedes it.
    fn document_hunk(&self, document: usize, hunk: &Range<usize>) -> Range<usize> {
        let lines = &self.lines[document];
        if !hunk.is_empty() {
            return lines[hunk.start]..lines[hunk.end - 1] + 1;
        }
        let position = if hunk.start == 0 { 0 } else { lines[hunk.start - 1] + 1 };
        position..position
    }

    /// Inverse of `document_hunk`; `None` for a hunk that is not one stretch of display order.
    fn display_hunk(&self, document: usize, hunk: &Range<usize>) -> Option<Range<usize>> {
        let positions = &self.positions[document];
        if hunk.is_empty() {
            let position = match hunk.start {
                0 => 0,
                after => positions.get(after - 1)? + 1,
            };
            return Some(position..position);
        }
        let start = *positions.get(hunk.start)?;
        let consecutive = (hunk.start..hunk.end).enumerate().all(|(offset, line)| positions.get(line).is_some_and(|position| *position == start + offset));
        consecutive.then(|| start..start + hunk.len())
    }

    /// The regions of a comparison of the documents in their own order, as regions of display order, for the result check.
    pub fn to_display(&self, regions: &[Region]) -> Result<Vec<Region>, EngineError> {
        regions
            .iter()
            .enumerate()
            .map(|(index, region)| {
                let mut hunks = Vec::with_capacity(region.hunks.len());
                for (document, hunk) in region.hunks.iter().enumerate() {
                    let display = self.positions.get(document).and_then(|_| self.display_hunk(document, hunk)).ok_or_else(|| {
                        EngineError::InvalidResult(format!("region {}: hunk {} is {hunk:?}, which is not one stretch in display order", index + 1, document_letter(document)))
                    })?;
                    hunks.push(display);
                }
                let mut rows = Vec::with_capacity(region.rows.len());
                for row in &region.rows {
                    let lines = row
                        .lines
                        .iter()
                        .enumerate()
                        .map(|(document, line)| match line {
                            None => Ok(None),
                            Some(line) => self.positions.get(document).and_then(|positions| positions.get(*line)).map(|position| Some(*position)).ok_or_else(|| {
                                EngineError::InvalidResult(format!("region {}: line {line} of {} does not exist", index + 1, document_letter(document)))
                            }),
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    rows.push(Row { lines });
                }
                Ok(Region { hunks, status: region.status, rows })
            })
            .collect()
    }

    /// The regions of a comparison of display order, as regions of the documents. The places where the next line is not the line after the previous one are the edges of alignment parts, so a region that passed the result check never reaches over one.
    fn to_documents(&self, regions: Vec<Region>) -> Result<Vec<Region>, EngineError> {
        let seams: Vec<Vec<usize>> = (0..self.lines.len()).map(|document| self.seams(document)).collect();
        let mut documents = Vec::with_capacity(regions.len());
        for (index, region) in regions.into_iter().enumerate() {
            if let Some(document) = (0..seams.len()).find(|document| seams[*document].iter().any(|seam| region.hunks[*document].start < *seam && *seam < region.hunks[*document].end)) {
                return Err(EngineError::InvalidResult(format!(
                    "region {}: hunk {} reaches over a place where the document was laid out in another order",
                    index + 1,
                    document_letter(document)
                )));
            }
            let hunks = region.hunks.iter().enumerate().map(|(document, hunk)| self.document_hunk(document, hunk)).collect();
            let rows = region
                .rows
                .iter()
                .map(|row| Row {
                    lines: row.lines.iter().enumerate().map(|(document, line)| line.map(|line| self.lines[document][line])).collect(),
                })
                .collect();
            documents.push(Region { hunks, status: region.status, rows });
        }
        Ok(documents)
    }
}


/// Compares documents that hold some alignments in another order than the list: lays them out in display order, compares that like any documents with alignments, and brings the regions back to the documents.
pub(super) fn compare_with_insets(engine: &dyn ComparisonEngine, input: &CompareInput<'_>, job: &Job<'_>, capabilities: &EngineCapabilities) -> Result<Comparison, EngineError> {
    let layout = Layout::new(input.alignments, &input.line_counts());
    let lines: Vec<Vec<&str>> = input.sources.iter().zip(&layout.lines).map(|(source, order)| order.iter().map(|line| source.lines[*line]).collect()).collect();
    let tokens: Vec<Vec<Token>> = input.sources.iter().zip(&layout.lines).map(|(source, order)| order.iter().map(|line| source.tokens[*line]).collect()).collect();
    let sources: Vec<SourceLines<'_>> = lines.iter().zip(&tokens).map(|(lines, tokens)| SourceLines { lines, tokens }).collect();
    let display = CompareInput {
        sources,
        alignments: &layout.alignments,
        options: input.options,
    };
    let comparison = compare_aligned(engine, &display, job, capabilities)?;
    check_comparison(&comparison, &display)?;
    job.check_canceled()?;
    let regions = layout.to_documents(comparison.regions)?;
    Ok(Comparison { regions })
}


/// One document's lines in display order, and the place of each alignment's part in them.
fn document_layout(alignments: &[ManualAlignment], tree: &[Option<usize>], document: usize, count: usize) -> (Vec<usize>, Vec<Range<usize>>) {
    let mut lines = Vec::with_capacity(count);
    let mut places = vec![0..0; alignments.len()];
    lay_out_group(alignments, tree, document, None, 0..count, &mut lines, &mut places);
    (lines, places)
}


/// Appends the lines of `area` in display order: the parts of the group `parent` exchange their places, and each part is laid out with its own nested alignments.
fn lay_out_group(alignments: &[ManualAlignment], tree: &[Option<usize>], document: usize, parent: Option<usize>, area: Range<usize>, lines: &mut Vec<usize>, places: &mut [Range<usize>]) {
    let part = |index: usize| alignments[index].parts.get(document).cloned().unwrap_or(0..0);
    let members = siblings(tree, parent);
    let mut cursor = area.start;
    for (shown, own) in sibling_order(alignments, tree, parent, document).into_iter().map(&part).enumerate() {
        lines.extend(cursor..own.start);
        let start = lines.len();
        lay_out_group(alignments, tree, document, Some(members[shown]), part(members[shown]), lines, places);
        places[members[shown]] = start..lines.len();
        cursor = own.end;
    }
    lines.extend(cursor..area.end);
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

    fn compare(fixture: &Fixture, alignments: &[ManualAlignment]) -> Comparison {
        with_job(|job| run_comparison(&SimpleEngine::default(), &fixture.input(alignments), job)).unwrap()
    }

    fn hunks(comparison: &Comparison) -> Vec<(Vec<Range<usize>>, RegionStatus)> {
        comparison.regions.iter().map(|region| (region.hunks.clone(), region.status)).collect()
    }

    const UNCHANGED: RegionStatus = RegionStatus::Merge(MergeStatus::Unchanged);

    const FUNCTIONS: [&[&str]; 3] = [
        &["pre", "f1a", "f1b", "mid", "f2a", "f2b", "post"],
        &["pre", "f2a", "f2b", "mid", "f1a", "f1b", "post"],
        &["pre", "f1a", "f1b", "mid", "f2a", "f2b", "post"],
    ];

    // The example of Inset alignment in ARCHITECTURE.md: B has the functions in the other order than A and C.
    fn functions_shown_as_in_a() -> [ManualAlignment; 2] {
        [alignment(&[1..3, 4..6, 1..3]), alignment(&[4..6, 1..3, 4..6])]
    }

    fn functions_shown_as_in_b() -> [ManualAlignment; 2] {
        [alignment(&[4..6, 1..3, 4..6]), alignment(&[1..3, 4..6, 1..3])]
    }

    #[test]
    fn documents_are_ordered_by_the_positions_of_their_parts() {
        let alignments = [alignment(&[4..6, 1..3]), alignment(&[1..3, 4..6]), alignment(&[0..0, 3..3]), alignment(&[0..0, 6..6])];
        assert_eq!(document_order(&alignments, 0), [2, 3, 1, 0]);
        assert_eq!(document_order(&alignments, 1), [0, 2, 1, 3]);
        assert!(has_insets(&alignments));
        assert!(!has_insets(&alignments[2..]));
        assert!(!has_insets(&[]));
    }

    #[test]
    fn moved_parts_are_the_ones_outside_the_longest_run_in_order() {
        let starts = [10, 20, 50, 0, 30, 40, 60];
        let alignments: Vec<ManualAlignment> = starts.iter().map(|start| alignment(&[*start..*start + 5])).collect();
        assert_eq!(document_order(&alignments, 0), [3, 0, 1, 4, 5, 2, 6]);
        assert_eq!(moved_parts(&alignments, 0), [None, None, Some(5), Some(0), None, None, None]);
    }

    #[test]
    fn a_swap_moves_the_part_that_is_listed_last() {
        let alignments = [alignment(&[3..5]), alignment(&[0..2])];
        assert_eq!(moved_parts(&alignments, 0), [None, Some(0)]);
    }

    #[test]
    fn documents_in_the_order_of_the_list_move_nothing() {
        let alignments = [alignment(&[0..2, 5..6]), alignment(&[3..4, 0..1])];
        assert_eq!(moved_parts(&alignments, 0), [None, None]);
        assert_eq!(moved_parts(&alignments, 1), [None, Some(0)]);
    }

    #[test]
    fn the_places_of_the_parts_are_exchanged_and_the_text_between_them_stays() {
        let layout = Layout::new(&functions_shown_as_in_a(), &[7, 7, 7]);
        assert_eq!(layout.lines[0], [0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(layout.lines[1], [0, 4, 5, 3, 1, 2, 6]);
        assert_eq!(layout.alignments, [alignment(&[1..3, 1..3, 1..3]), alignment(&[4..6, 4..6, 4..6])]);
        assert_eq!(layout.seams(1), [1, 3, 4, 6]);
        assert!(layout.seams(0).is_empty());
    }

    #[test]
    fn the_other_display_order_exchanges_the_places_of_the_other_documents() {
        let layout = Layout::new(&functions_shown_as_in_b(), &[7, 7, 7]);
        assert_eq!(layout.lines[0], [0, 4, 5, 3, 1, 2, 6]);
        assert_eq!(layout.lines[1], [0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(layout.lines[2], layout.lines[0]);
    }

    #[test]
    fn an_anchor_takes_the_place_of_the_part_it_is_exchanged_with() {
        let alignments = [alignment(&[0..0, 4..5]), alignment(&[0..2, 1..1])];
        let layout = Layout::new(&alignments, &[3, 6]);
        assert_eq!(layout.lines[1], [0, 4, 1, 2, 3, 5]);
        assert_eq!(layout.lines[0], [0, 1, 2]);
        assert_eq!(layout.alignments, [alignment(&[0..0, 1..2]), alignment(&[0..2, 5..5])]);
    }

    #[test]
    fn display_hunks_and_document_hunks_convert_into_each_other() {
        let layout = Layout::new(&functions_shown_as_in_a(), &[7, 7, 7]);
        assert_eq!(layout.document_hunk(1, &(4..6)), 1..3);
        assert_eq!(layout.display_hunk(1, &(1..3)), Some(4..6));
        assert_eq!(layout.display_hunk(1, &(0..2)), None);
        assert_eq!(layout.document_hunk(1, &(1..1)), 1..1);
        assert_eq!(layout.display_hunk(1, &(1..1)), Some(1..1));
        assert_eq!(layout.display_hunk(1, &(0..0)), Some(0..0));
    }

    #[test]
    fn insets_are_compared_in_display_order() {
        let fixture = Fixture::new(&FUNCTIONS);
        let comparison = compare(&fixture, &functions_shown_as_in_a());
        let expected = [
            (vec![0..1, 0..1, 0..1], UNCHANGED),
            (vec![1..3, 4..6, 1..3], UNCHANGED),
            (vec![3..4, 3..4, 3..4], UNCHANGED),
            (vec![4..6, 1..3, 4..6], UNCHANGED),
            (vec![6..7, 6..7, 6..7], UNCHANGED),
        ];
        assert_eq!(hunks(&comparison), expected);
    }

    #[test]
    fn the_other_display_order_moves_the_other_documents() {
        let fixture = Fixture::new(&FUNCTIONS);
        let comparison = compare(&fixture, &functions_shown_as_in_b());
        let expected = [
            (vec![0..1, 0..1, 0..1], UNCHANGED),
            (vec![4..6, 1..3, 4..6], UNCHANGED),
            (vec![3..4, 3..4, 3..4], UNCHANGED),
            (vec![1..3, 4..6, 1..3], UNCHANGED),
            (vec![6..7, 6..7, 6..7], UNCHANGED),
        ];
        assert_eq!(hunks(&comparison), expected);
    }

    // A and B have 1 above 2, C has 2 above 1: swapping the two hunks in A and B gives C, with no lines left over.
    fn swapped_hunks() -> [ManualAlignment; 2] {
        [alignment(&[3..5, 3..5, 0..2]), alignment(&[0..2, 0..2, 3..5])]
    }

    #[test]
    fn swapped_hunks_merge_without_changes_and_without_extra_lines() {
        let fixture = Fixture::new(&[&["1", "1", "", "2", "2"], &["1", "1", "", "2", "2"], &["2", "2", "", "1", "1"]]);
        let comparison = compare(&fixture, &swapped_hunks());
        let expected = [(vec![3..5, 3..5, 0..2], UNCHANGED), (vec![2..3, 2..3, 2..3], UNCHANGED), (vec![0..2, 0..2, 3..5], UNCHANGED)];
        assert_eq!(hunks(&comparison), expected);
    }

    #[test]
    fn a_change_in_a_swapped_hunk_stays_in_that_hunk() {
        let fixture = Fixture::new(&[&["1", "1", "", "2", "2"], &["1", "1", "", "2", "22"], &["2", "2", "", "1", "1"]]);
        let comparison = compare(&fixture, &swapped_hunks());
        let expected = [
            (vec![3..4, 3..4, 0..1], UNCHANGED),
            (vec![4..5, 4..5, 1..2], RegionStatus::Merge(MergeStatus::ChangedInB)),
            (vec![2..3, 2..3, 2..3], UNCHANGED),
            (vec![0..2, 0..2, 3..5], UNCHANGED),
        ];
        assert_eq!(hunks(&comparison), expected);
    }

    #[test]
    fn a_change_in_the_text_between_the_hunks_is_found_between_them() {
        let fixture = Fixture::new(&[&["1", "1", "", "2", "2"], &["1", "1", "x", "2", "2"], &["2", "2", "", "1", "1"]]);
        let comparison = compare(&fixture, &swapped_hunks());
        let expected = [
            (vec![3..5, 3..5, 0..2], UNCHANGED),
            (vec![2..3, 2..3, 2..3], RegionStatus::Merge(MergeStatus::ChangedInB)),
            (vec![0..2, 0..2, 3..5], UNCHANGED),
        ];
        assert_eq!(hunks(&comparison), expected);
    }

    #[test]
    fn two_way_insets_swap_too() {
        let fixture = Fixture::new(&[&["a", "", "b"], &["b", "", "a"]]);
        let alignments = [alignment(&[0..1, 2..3]), alignment(&[2..3, 0..1])];
        let comparison = compare(&fixture, &alignments);
        let unchanged = RegionStatus::Diff(HunkChange::Unchanged);
        assert_eq!(hunks(&comparison), [(vec![0..1, 2..3], unchanged), (vec![1..2, 1..2], unchanged), (vec![2..3, 0..1], unchanged)]);
    }

    #[test]
    fn a_region_over_a_seam_is_refused() {
        let layout = Layout::new(&functions_shown_as_in_a(), &[7, 7, 7]);
        let region = Region {
            hunks: vec![0..7, 0..7, 0..7],
            status: UNCHANGED,
            rows: Vec::new(),
        };
        let error = layout.to_documents(vec![region]).unwrap_err();
        assert!(matches!(error, EngineError::InvalidResult(_)), "{error:?}");
    }

    const UNCHANGED_DIFF: RegionStatus = RegionStatus::Diff(HunkChange::Unchanged);

    // B has the second function first. The first function holds a nested alignment, which moves along with it.
    fn parent_with_child() -> (Vec<Vec<&'static str>>, [ManualAlignment; 3]) {
        let documents = vec![vec!["pre", "p1a", "c1", "p1b", "mid", "p2", "post"], vec!["pre", "p2", "mid", "p1a", "c1", "p1b", "post"]];
        (documents, [alignment(&[1..4, 3..6]), alignment(&[5..6, 1..2]), alignment(&[2..3, 4..5])])
    }

    #[test]
    fn a_nested_alignment_moves_with_its_parent() {
        let (documents, alignments) = parent_with_child();
        let layout = Layout::new(&alignments, &[7, 7]);
        assert_eq!(layout.lines[1], [0, 3, 4, 5, 2, 1, 6]);
        assert_eq!(layout.alignments, [alignment(&[1..4, 1..4]), alignment(&[5..6, 5..6]), alignment(&[2..3, 2..3])]);
        assert_eq!(moved_parts(&alignments, 1), [None, Some(0), None]);
        let documents: Vec<&[&'static str]> = documents.iter().map(|document| document.as_slice()).collect();
        let comparison = compare(&Fixture::new(&documents), &alignments);
        let expected = [
            (vec![0..1, 0..1], UNCHANGED_DIFF),
            (vec![1..2, 3..4], UNCHANGED_DIFF),
            (vec![2..3, 4..5], UNCHANGED_DIFF),
            (vec![3..4, 5..6], UNCHANGED_DIFF),
            (vec![4..5, 2..3], UNCHANGED_DIFF),
            (vec![5..6, 1..2], UNCHANGED_DIFF),
            (vec![6..7, 6..7], UNCHANGED_DIFF),
        ];
        assert_eq!(hunks(&comparison), expected);
    }

    // The children of the aligned parent are in the other order in B.
    fn swapped_children() -> (Vec<Vec<&'static str>>, [ManualAlignment; 4]) {
        let documents = vec![vec!["pre", "x", "c1", "m", "c2", "y", "q"], vec!["pre", "x", "c2", "m", "c1", "y", "q"]];
        (documents, [alignment(&[1..6, 1..6]), alignment(&[2..3, 4..5]), alignment(&[4..5, 2..3]), alignment(&[6..7, 6..7])])
    }

    #[test]
    fn children_swapped_inside_an_aligned_parent_swap_only_inside_it() {
        let (documents, alignments) = swapped_children();
        let layout = Layout::new(&alignments, &[7, 7]);
        assert_eq!(layout.lines[0], [0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(layout.lines[1], [0, 1, 4, 3, 2, 5, 6]);
        assert_eq!(layout.alignments, [alignment(&[1..6, 1..6]), alignment(&[2..3, 2..3]), alignment(&[4..5, 4..5]), alignment(&[6..7, 6..7])]);
        let documents: Vec<&[&'static str]> = documents.iter().map(|document| document.as_slice()).collect();
        let comparison = compare(&Fixture::new(&documents), &alignments);
        let expected = [
            (vec![0..1, 0..1], UNCHANGED_DIFF),
            (vec![1..2, 1..2], UNCHANGED_DIFF),
            (vec![2..3, 4..5], UNCHANGED_DIFF),
            (vec![3..4, 3..4], UNCHANGED_DIFF),
            (vec![4..5, 2..3], UNCHANGED_DIFF),
            (vec![5..6, 5..6], UNCHANGED_DIFF),
            (vec![6..7, 6..7], UNCHANGED_DIFF),
        ];
        assert_eq!(hunks(&comparison), expected);
    }

    #[test]
    fn insets_are_decided_per_sibling_group() {
        let (_, alignments) = swapped_children();
        assert!(has_insets(&alignments));
        assert!(!has_insets(&alignments[..1]));
        assert_eq!(moved_parts(&alignments, 0), [None, None, None, None]);
        assert_eq!(moved_parts(&alignments, 1), [None, None, Some(1), None]);
        assert_eq!(document_order(&alignments, 1), [0, 2, 1, 3]);
        assert_eq!(sibling_order(&alignments, &alignment_tree(&alignments), None, 1), [0, 3]);
    }

    #[test]
    fn crossing_parts_are_validated_like_the_others() {
        let fixture = Fixture::new(&[&["a", "b"], &["a", "b"]]);
        let overlapping = [alignment(&[0..1, 0..2]), alignment(&[1..2, 1..2])];
        let error = with_job(|job| run_comparison(&SimpleEngine::default(), &fixture.input(&overlapping), job)).unwrap_err();
        assert_eq!(error, EngineError::InvalidAlignment("alignments 1 and 2 overlap in B".to_string()));
    }

    #[test]
    fn the_result_check_reads_regions_in_display_order() {
        let fixture = Fixture::new(&FUNCTIONS);
        let alignments = functions_shown_as_in_a();
        let comparison = compare(&fixture, &alignments);
        assert_eq!(check_comparison(&comparison, &fixture.input(&alignments)), Ok(()));
        let mut regions = comparison.regions.clone();
        regions.swap(1, 3);
        let error = check_comparison(&Comparison { regions }, &fixture.input(&alignments)).unwrap_err();
        assert!(matches!(error, EngineError::InvalidResult(_)), "{error:?}");
    }

    #[test]
    fn native_engines_get_the_alignments_in_display_order() {
        let fixture = Fixture::new(&FUNCTIONS);
        let engine = SimpleEngine {
            manual_alignment: true,
            ..SimpleEngine::default()
        };
        with_job(|job| run_comparison(&engine, &fixture.input(&functions_shown_as_in_a()), job)).unwrap();
        let seen = engine.seen_alignments.into_inner().unwrap();
        assert_eq!(seen[0], 2);
    }

    const ALPHABET: [&str; 4] = ["a", "b", "c", "d"];

    /// Random documents with two to four alignments that are listed in display order but sit in any order in every document.
    fn documents_with_insets() -> impl Strategy<Value = (Vec<Vec<&'static str>>, Vec<ManualAlignment>)> {
        (2usize..=3, 2usize..5)
            .prop_flat_map(|(documents, count)| {
                vec(vec(select(ALPHABET.to_vec()), 0..14), documents).prop_flat_map(move |lines| {
                    let cuts: Vec<_> = lines.iter().map(|document| vec(0..=document.len(), count * 2)).collect();
                    let orders: Vec<_> = (0..lines.len()).map(|_| Just((0..count).collect::<Vec<usize>>()).prop_shuffle()).collect();
                    (Just(lines), cuts, orders)
                })
            })
            .prop_map(|(lines, mut cuts, orders)| {
                cuts.iter_mut().for_each(|document| document.sort());
                let count = cuts.first().map(|document| document.len() / 2).unwrap_or(0);
                let alignments = (0..count)
                    .map(|index| {
                        let parts = (0..cuts.len()).map(|document| {
                            let part = orders[document][index];
                            cuts[document][part * 2]..cuts[document][part * 2 + 1]
                        });
                        ManualAlignment::new(parts.collect())
                    })
                    .filter(|alignment| alignment.parts.iter().any(|part| !part.is_empty()))
                    .collect();
                (lines, alignments)
            })
    }

    proptest! {
        // Passing run_comparison means the result check accepted it: every line once and in display order, alignments at region boundaries.
        #[test]
        fn insets_give_valid_results((lines, alignments) in documents_with_insets()) {
            let documents: Vec<&[&'static str]> = lines.iter().map(|document| document.as_slice()).collect();
            let fixture = Fixture::new(&documents);
            let result = with_job(|job| run_comparison(&SimpleEngine::default(), &fixture.input(&alignments), job));
            prop_assert!(result.is_ok(), "{result:?}");
            let comparison = result.unwrap();
            for (document, count) in fixture.lines.iter().map(|lines| lines.len()).enumerate() {
                let mut seen = vec![0; count];
                for line in comparison.regions.iter().flat_map(|region| region.rows.iter()).filter_map(|row| row.lines[document]) {
                    seen[line] += 1;
                }
                prop_assert!(seen.iter().all(|times| *times == 1), "document {document}: {seen:?}");
            }
        }

        // The layout keeps every line of a document once, and the alignments in the order of the list in all documents.
        #[test]
        fn layouts_keep_every_line_once((lines, alignments) in documents_with_insets()) {
            let counts: Vec<usize> = lines.iter().map(|document| document.len()).collect();
            let layout = Layout::new(&alignments, &counts);
            for (document, count) in counts.iter().enumerate() {
                let mut shown = layout.lines[document].clone();
                shown.sort();
                prop_assert_eq!(shown, (0..*count).collect::<Vec<_>>());
                let order = document_order(&layout.alignments, document);
                prop_assert!(order.iter().enumerate().all(|(slot, index)| slot == *index), "document {document}: {order:?}");
            }
        }
    }
}
