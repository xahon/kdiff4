use std::ops::Range;

use super::{CompareInput, Comparison, ComparisonEngine, EngineCapabilities, EngineError, Job, ManualAlignment, Region, Row, SourceLines, Token, check_comparison, compare_aligned, document_letter};


/// The source documents in display order: every lifted part is taken out of its place and put where its alignment is shown, so the alignments keep one order in every document. A lifted part goes into the stretch of text between the parts around it in the list. The text stays below parts that came from above and above parts that came from below; where those wishes meet, the parts keep the order of the list and the text goes before the first part that came from below.
pub(super) struct Layout {
    /// Per document, the document line of every line in display order.
    lines: Vec<Vec<usize>>,
    /// Per document, the place in display order of every document line.
    positions: Vec<Vec<usize>>,
    /// The alignments with their parts in display order. None is lifted.
    pub alignments: Vec<ManualAlignment>,
}

impl Layout {
    /// For alignments that passed `validate_alignments`.
    pub fn new(alignments: &[ManualAlignment], line_counts: &[usize]) -> Layout {
        let mut lines = Vec::with_capacity(line_counts.len());
        let mut parts = vec![Vec::with_capacity(line_counts.len()); alignments.len()];
        for (document, count) in line_counts.iter().enumerate() {
            let (document_lines, document_parts) = document_layout(alignments, document, *count);
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

    /// The regions of a comparison of display order, as regions of the documents. A region that reaches over a seam is split there and its parts are classified again.
    fn to_documents(&self, regions: Vec<Region>, sources: &[SourceLines<'_>], engine: &dyn ComparisonEngine) -> Result<Vec<Region>, EngineError> {
        let seams: Vec<Vec<usize>> = (0..self.lines.len()).map(|document| self.seams(document)).collect();
        let mut documents = Vec::with_capacity(regions.len());
        for region in regions {
            let mut pieces = vec![region];
            let mut index = 0;
            let mut was_split = false;
            while index < pieces.len() {
                let Some((first, second)) = split_at_seam(&pieces[index], &seams) else {
                    index += 1;
                    continue;
                };
                pieces[index] = first;
                pieces.insert(index + 1, second);
                was_split = true;
            }
            for mut piece in pieces {
                if was_split {
                    let hunks: Vec<SourceLines<'_>> = sources.iter().zip(&piece.hunks).map(|(source, hunk)| source.slice(hunk.clone())).collect();
                    piece.status = engine.classify(&hunks)?;
                }
                let hunks = piece.hunks.iter().enumerate().map(|(document, hunk)| self.document_hunk(document, hunk)).collect();
                let rows = piece
                    .rows
                    .iter()
                    .map(|row| Row {
                        lines: row.lines.iter().enumerate().map(|(document, line)| line.map(|line| self.lines[document][line])).collect(),
                    })
                    .collect();
                documents.push(Region { hunks, status: piece.status, rows });
            }
        }
        Ok(documents)
    }
}


/// Compares documents whose alignments hold lifted parts: lays them out in display order, compares that like any documents with alignments, and brings the regions back to the documents.
pub(super) fn compare_with_insets(engine: &dyn ComparisonEngine, input: &CompareInput<'_>, job: &Job<'_>, capabilities: &EngineCapabilities) -> Result<Comparison, EngineError> {
    let layout = Layout::new(input.alignments, &input.line_counts());
    let lines: Vec<Vec<&str>> = input.sources.iter().zip(&layout.lines).map(|(source, order)| order.iter().map(|line| source.lines[*line]).collect()).collect();
    let tokens: Vec<Vec<Token>> = input.sources.iter().zip(&layout.lines).map(|(source, order)| order.iter().map(|line| source.tokens[*line]).collect()).collect();
    let sources: Vec<SourceLines<'_>> = lines.iter().zip(&tokens).map(|(lines, tokens)| SourceLines { lines, tokens }).collect();
    let display = CompareInput {
        sources: sources.clone(),
        alignments: &layout.alignments,
        options: input.options,
    };
    let comparison = compare_aligned(engine, &display, job, capabilities)?;
    check_comparison(&comparison, &display)?;
    job.check_canceled()?;
    let regions = layout.to_documents(comparison.regions, &sources, engine)?;
    Ok(Comparison { regions })
}


/// One document's lines in display order, and the place of each alignment's part in them.
fn document_layout(alignments: &[ManualAlignment], document: usize, count: usize) -> (Vec<usize>, Vec<Range<usize>>) {
    let in_place: Vec<usize> = (0..alignments.len()).filter(|index| !alignments[*index].is_lifted(document)).collect();
    let part = |index: usize| alignments[index].parts[document].clone();
    let mut vacated: Vec<Range<usize>> = (0..alignments.len()).filter(|index| alignments[*index].is_lifted(document)).map(part).filter(|part| !part.is_empty()).collect();
    vacated.sort_by_key(|range| range.start);
    let gap = |index: usize| {
        let start = if index == 0 { 0 } else { part(in_place[index - 1]).end };
        let end = if index == in_place.len() { count } else { part(in_place[index]).start };
        start..end
    };
    let mut lines = Vec::with_capacity(count);
    let mut places = vec![0..0; alignments.len()];
    let mut put = |lines: &mut Vec<usize>, index: usize| {
        let start = lines.len();
        lines.extend(part(index));
        places[index] = start..lines.len();
    };
    let mut shown_in: Vec<Vec<(usize, bool)>> = vec![Vec::new(); in_place.len() + 1];
    for lifted in (0..alignments.len()).filter(|index| alignments[*index].is_lifted(document)) {
        let gap_index = in_place.iter().filter(|index| **index < lifted).count();
        let (gap, part) = (gap(gap_index), part(lifted));
        let came_from_below = part.start >= gap.end && !(part.is_empty() && part.start == gap.end);
        shown_in[gap_index].push((lifted, came_from_below));
    }
    for gap_index in 0..=in_place.len() {
        let text_after = shown_in[gap_index].iter().position(|(_, came_from_below)| *came_from_below).unwrap_or(shown_in[gap_index].len());
        for (index, _) in shown_in[gap_index][..text_after].iter() {
            put(&mut lines, *index);
        }
        let gap = gap(gap_index);
        let mut cursor = gap.start;
        for range in vacated.iter().filter(|range| gap.start <= range.start && range.end <= gap.end) {
            lines.extend(cursor..range.start);
            cursor = range.end;
        }
        lines.extend(cursor..gap.end);
        for (index, _) in shown_in[gap_index][text_after..].iter() {
            put(&mut lines, *index);
        }
        if let Some(index) = in_place.get(gap_index) {
            put(&mut lines, *index);
        }
    }
    (lines, places)
}


/// Splits the region at a seam inside one of its hunks, in the first document that has one.
fn split_at_seam(region: &Region, seams: &[Vec<usize>]) -> Option<(Region, Region)> {
    for (document, hunk) in region.hunks.iter().enumerate() {
        let Some(seam) = seams[document].iter().find(|seam| hunk.start < **seam && **seam < hunk.end) else {
            continue;
        };
        let at = region.rows.iter().position(|row| row.lines[document].is_some_and(|line| line >= *seam))?;
        let (first_rows, second_rows) = region.rows.split_at(at);
        let taken = |other: usize| first_rows.iter().filter(|row| row.lines[other].is_some()).count();
        let first_hunks = region.hunks.iter().enumerate().map(|(other, hunk)| hunk.start..hunk.start + taken(other)).collect();
        let second_hunks = region.hunks.iter().enumerate().map(|(other, hunk)| hunk.start + taken(other)..hunk.end).collect();
        let first = Region {
            hunks: first_hunks,
            status: region.status,
            rows: first_rows.to_vec(),
        };
        let second = Region {
            hunks: second_hunks,
            status: region.status,
            rows: second_rows.to_vec(),
        };
        return Some((first, second));
    }
    None
}


#[cfg(test)]
mod tests {
    use proptest::collection::vec;
    use proptest::prelude::*;
    use proptest::sample::select;

    use super::super::test_engine::{Fixture, SimpleEngine, with_job};
    use super::super::{HunkChange, MergeStatus, RegionStatus, run_comparison};
    use super::*;

    fn lifted(parts: &[Range<usize>], lifted: &[bool]) -> ManualAlignment {
        ManualAlignment::with_lifted(parts.to_vec(), lifted.to_vec())
    }

    fn compare(fixture: &Fixture, alignments: &[ManualAlignment]) -> Comparison {
        with_job(|job| run_comparison(&SimpleEngine::default(), &fixture.input(alignments), job)).unwrap()
    }

    fn hunks(comparison: &Comparison) -> Vec<(Vec<Range<usize>>, RegionStatus)> {
        comparison.regions.iter().map(|region| (region.hunks.clone(), region.status)).collect()
    }

    const FUNCTIONS: [&[&str]; 3] = [
        &["pre", "f1a", "f1b", "mid", "f2a", "f2b", "post"],
        &["pre", "f2a", "f2b", "mid", "f1a", "f1b", "post"],
        &["pre", "f1a", "f1b", "mid", "f2a", "f2b", "post"],
    ];

    // The example of Inset alignment in ARCHITECTURE.md: B has the functions in the other order than A and C.
    fn functions_shown_as_in_a() -> [ManualAlignment; 2] {
        [lifted(&[1..3, 4..6, 1..3], &[false, false, false]), lifted(&[4..6, 1..3, 4..6], &[false, true, false])]
    }

    fn functions_shown_as_in_b() -> [ManualAlignment; 2] {
        [lifted(&[4..6, 1..3, 4..6], &[true, false, true]), lifted(&[1..3, 4..6, 1..3], &[false, false, false])]
    }

    #[test]
    fn a_lifted_part_leaves_its_place_and_joins_its_alignment() {
        let layout = Layout::new(&functions_shown_as_in_a(), &[7, 7, 7]);
        assert_eq!(layout.lines[0], [0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(layout.lines[1], [0, 3, 4, 5, 1, 2, 6]);
        assert_eq!(layout.alignments, [ManualAlignment::new(vec![1..3, 2..4, 1..3]), ManualAlignment::new(vec![4..6, 4..6, 4..6])]);
        assert_eq!(layout.seams(1), [1, 4, 6]);
        assert!(layout.seams(0).is_empty());
    }

    #[test]
    fn parts_lifted_in_the_other_documents_come_to_the_top_of_their_gap() {
        let layout = Layout::new(&functions_shown_as_in_b(), &[7, 7, 7]);
        assert_eq!(layout.lines[0], [0, 4, 5, 1, 2, 3, 6]);
        assert_eq!(layout.lines[1], [0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(layout.lines[2], layout.lines[0]);
    }

    #[test]
    fn lifted_anchors_take_the_end_of_their_gap_from_below_and_the_start_from_above() {
        let up = [lifted(&[0..1, 5..5], &[false, true]), lifted(&[2..3, 2..3], &[false, false])];
        let layout = Layout::new(&up, &[4, 6]);
        assert_eq!(layout.alignments[0], ManualAlignment::new(vec![0..1, 2..2]));
        let down = [lifted(&[0..1, 0..1], &[false, false]), lifted(&[1..3, 5..5], &[false, true])];
        let layout = Layout::new(&down, &[4, 6]);
        assert_eq!(layout.alignments[1], ManualAlignment::new(vec![1..3, 1..1]));
    }

    // Found by the property test: an anchor that came from above and a range that came from below meet in one gap.
    #[test]
    fn lifted_parts_of_one_gap_keep_the_order_of_the_list() {
        let alignments = [lifted(&[0..0, 1..2], &[false, true]), lifted(&[0..0, 0..0], &[false, true]), lifted(&[0..0, 1..1], &[false, false])];
        let layout = Layout::new(&alignments, &[0, 3]);
        assert_eq!(layout.alignments.iter().map(|alignment| alignment.parts[1].clone()).collect::<Vec<_>>(), [1..2, 2..2, 2..2]);
        assert_eq!(layout.lines[1], [0, 1, 2]);
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
        let unchanged = RegionStatus::Merge(MergeStatus::Unchanged);
        let changed_in_b = RegionStatus::Merge(MergeStatus::ChangedInB);
        let expected = [
            (vec![0..1, 0..1, 0..1], unchanged),
            (vec![1..1, 3..4, 1..1], changed_in_b),
            (vec![1..3, 4..6, 1..3], unchanged),
            (vec![3..4, 6..6, 3..4], changed_in_b),
            (vec![4..6, 1..3, 4..6], unchanged),
            (vec![6..7, 6..7, 6..7], unchanged),
        ];
        assert_eq!(hunks(&comparison), expected);
    }

    #[test]
    fn the_other_display_order_moves_the_other_documents() {
        let fixture = Fixture::new(&FUNCTIONS);
        let comparison = compare(&fixture, &functions_shown_as_in_b());
        let unchanged = RegionStatus::Merge(MergeStatus::Unchanged);
        let shown = hunks(&comparison);
        let first = shown.iter().position(|(hunks, _)| *hunks == vec![4..6, 1..3, 4..6]).unwrap();
        let second = shown.iter().position(|(hunks, _)| *hunks == vec![1..3, 4..6, 1..3]).unwrap();
        assert!(first < second);
        assert_eq!(shown[first].1, unchanged);
        assert_eq!(shown[second].1, unchanged);
    }

    #[test]
    fn a_region_over_a_seam_is_split_and_classified_again() {
        let fixture = Fixture::new(&[&["p", "G", "q", "F", "r"], &["P", "F", "q", "G", "r"]]);
        let alignments = [lifted(&[1..2, 3..4], &[false, false]), lifted(&[3..4, 1..2], &[false, true])];
        let comparison = compare(&fixture, &alignments);
        let shown = hunks(&comparison);
        assert!(shown.contains(&(vec![0..1, 0..1], RegionStatus::Diff(HunkChange::Modified))));
        assert!(shown.contains(&(vec![1..1, 2..3], RegionStatus::Diff(HunkChange::Added))));
        assert!(shown.contains(&(vec![1..2, 3..4], RegionStatus::Diff(HunkChange::Unchanged))));
    }

    #[test]
    fn lifted_parts_are_validated_like_the_others() {
        let fixture = Fixture::new(&[&["a", "b"], &["a", "b"]]);
        let overlapping = [lifted(&[0..1, 0..2], &[false, true]), lifted(&[1..2, 1..2], &[false, false])];
        let error = with_job(|job| run_comparison(&SimpleEngine::default(), &fixture.input(&overlapping), job)).unwrap_err();
        assert_eq!(error, EngineError::InvalidAlignment("alignments 1 and 2 overlap in B".to_string()));
        let wrong_flags = [ManualAlignment { parts: vec![0..1, 0..1], lifted: vec![true] }];
        let error = with_job(|job| run_comparison(&SimpleEngine::default(), &fixture.input(&wrong_flags), job)).unwrap_err();
        assert_eq!(error, EngineError::InvalidAlignment("alignment 1 has 1 lifted flags, expected 2".to_string()));
    }

    #[test]
    fn the_result_check_reads_regions_in_display_order() {
        let fixture = Fixture::new(&FUNCTIONS);
        let alignments = functions_shown_as_in_a();
        let comparison = compare(&fixture, &alignments);
        assert_eq!(check_comparison(&comparison, &fixture.input(&alignments)), Ok(()));
        let mut regions = comparison.regions.clone();
        regions.swap(2, 3);
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

    /// Random documents with two to four alignments that are shown in list order but sit in any order in every document. The parts that keep the list order in a document are the longest stretch of its parts that is already in order.
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
                let in_order: Vec<Vec<bool>> = orders.iter().map(|order| longest_increasing(order)).collect();
                let alignments = (0..count)
                    .map(|index| {
                        let parts = (0..cuts.len()).map(|document| {
                            let part = orders[document][index];
                            cuts[document][part * 2]..cuts[document][part * 2 + 1]
                        });
                        ManualAlignment::with_lifted(parts.collect(), in_order.iter().map(|document| !document[index]).collect())
                    })
                    .filter(|alignment| alignment.parts.iter().any(|part| !part.is_empty()))
                    .collect();
                (lines, alignments)
            })
    }

    /// Which entries of `order` are in a longest increasing subsequence.
    fn longest_increasing(order: &[usize]) -> Vec<bool> {
        let mut best: Vec<Vec<usize>> = Vec::new();
        for (index, value) in order.iter().enumerate() {
            let previous = (0..index).filter(|before| order[*before] < *value).map(|before| best[before].clone()).max_by_key(|chain| chain.len()).unwrap_or_default();
            let mut chain = previous;
            chain.push(index);
            best.push(chain);
        }
        let chain = best.into_iter().max_by_key(|chain| chain.len()).unwrap_or_default();
        (0..order.len()).map(|index| chain.contains(&index)).collect()
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
    }
}
