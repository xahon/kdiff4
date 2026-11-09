use std::ops::Range;

use kdiff4_protocol::Document;
use kdiff4_protocol::LineCharRuns;
use kdiff4_protocol::engine::{CharRun, HunkChange, MergeStatus, Region, RegionStatus};

/// How a run of characters or a gutter block is marked: red when it differs from every other document of the row, otherwise in the color of the one other document it differs from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharStyle {
    Red,
    Document(Document),
}

/// Where the characters of a run are the same, at the same place of the line in the row: in A, in B, and in C, which is `None` in a 2-way diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SameIn {
    pub a: bool,
    pub b: bool,
    pub c: Option<bool>,
}

/// The char-diff rules: how a run of `document` is marked, from where its characters are the same. `None` is plain text. A document with no line in the row is never the same. A 2-way diff follows the same rules with C missing, so A is marked in B's color and B in A's, like kdiff3.
pub fn char_style(document: Document, same: SameIn) -> Option<CharStyle> {
    if document == Document::D {
        return None;
    }
    let others: Vec<(Document, bool)> = [(Document::A, Some(same.a)), (Document::B, Some(same.b)), (Document::C, same.c)]
        .into_iter()
        .filter(|(other, _)| *other != document)
        .filter_map(|(other, same)| same.map(|same| (other, same)))
        .collect();
    let different: Vec<Document> = others.iter().filter(|(_, same)| !same).map(|(other, _)| other.clone()).collect();
    match different.as_slice() {
        [] => None,
        [other] => Some(CharStyle::Document(other.clone())),
        _ => Some(CharStyle::Red),
    }
}

/// Which hunks of a region are equal: A and B, A and C, B and C.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HunksEqual {
    pub a_b: bool,
    pub a_c: bool,
    pub b_c: bool,
}

impl HunksEqual {
    /// Equal where the region's status says they are. A 3-way merge region takes A as the base.
    pub fn from_status(status: &RegionStatus) -> HunksEqual {
        let (a_b, a_c, b_c) = match status {
            RegionStatus::Diff(HunkChange::Unchanged) | RegionStatus::Merge(MergeStatus::Unchanged) => (true, true, true),
            RegionStatus::Merge(MergeStatus::ChangedInB) => (false, true, false),
            RegionStatus::Merge(MergeStatus::ChangedInC) => (true, false, false),
            RegionStatus::Merge(MergeStatus::ChangedAlike) => (false, false, true),
            RegionStatus::Diff(_) | RegionStatus::Merge(MergeStatus::Conflict | MergeStatus::WhitespaceConflict) => (false, false, false),
        };
        HunksEqual { a_b, a_c, b_c }
    }


    /// Also equal where the char diff of the region marks nothing between two hunks: every run of each is the same in the other. Used when whitespace is ignored, so hunks that differ only in whitespace count as equal. `char_diffs` is per source document, in A, B, C order.
    pub fn or_from_char_diffs(self, region: &Region, char_diffs: &[LineCharRuns]) -> HunksEqual {
        let same = |document: usize, other: usize| {
            let lines = region.hunks.get(document).cloned().unwrap_or_default();
            let runs = char_diffs.get(document);
            runs.is_some_and(|runs| lines.into_iter().all(|line| runs.line(line).iter().all(|run| run.same_in.get(other).cloned().unwrap_or(false))))
        };
        let equal = |left: usize, right: usize| region.hunks.len() > right && same(left, right) && same(right, left);
        HunksEqual {
            a_b: self.a_b || equal(0, 1),
            a_c: self.a_c || equal(0, 2),
            b_c: self.b_c || equal(1, 2),
        }
    }
}


/// How the gutter block of a region is marked in the pane of `document`: by the char-diff rules, with the region's documents being the same where `equal` says. `None` when the hunks are all equal. `sources` is how many source documents the comparison has.
pub fn region_style(document: Document, equal: HunksEqual, sources: usize) -> Option<CharStyle> {
    let three_way = sources > 2;
    let same = match document {
        Document::A => SameIn {
            a: true,
            b: equal.a_b,
            c: three_way.then_some(equal.a_c),
        },
        Document::B => SameIn {
            a: equal.a_b,
            b: true,
            c: three_way.then_some(equal.b_c),
        },
        Document::C => SameIn {
            a: equal.a_c,
            b: equal.b_c,
            c: Some(true),
        },
        Document::D => return None,
    };
    char_style(document, same)
}


/// The styled column ranges of a line of `document` with `runs`, in order; neighbors with the same style are joined, so one background covers all of them. `sources` is how many source documents the row has. Plain runs are left out.
pub fn segments(document: Document, runs: &[CharRun], sources: usize) -> Vec<(Range<usize>, CharStyle)> {
    let flag = |run: &CharRun, index: usize| run.same_in.get(index).cloned().unwrap_or(false);
    let mut segments: Vec<(Range<usize>, CharStyle)> = Vec::new();
    for run in runs {
        let same = SameIn {
            a: flag(run, 0),
            b: flag(run, 1),
            c: (sources > 2).then(|| flag(run, 2)),
        };
        let Some(style) = char_style(document.clone(), same) else { continue };
        match segments.last_mut() {
            Some((columns, last)) if *last == style && columns.end == run.columns.start => columns.end = run.columns.end.clone(),
            _ => segments.push((run.columns.clone(), style)),
        }
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Option<CharStyle> = Some(CharStyle::Red);
    const FROM_A: Option<CharStyle> = Some(CharStyle::Document(Document::A));
    const FROM_B: Option<CharStyle> = Some(CharStyle::Document(Document::B));
    const FROM_C: Option<CharStyle> = Some(CharStyle::Document(Document::C));

    fn same(a: bool, b: bool, c: Option<bool>) -> SameIn {
        SameIn { a, b, c }
    }

    #[test]
    fn text_equal_in_all_documents_is_plain() {
        assert_eq!(char_style(Document::A, same(true, true, Some(true))), None);
        assert_eq!(char_style(Document::B, same(true, true, Some(true))), None);
        assert_eq!(char_style(Document::C, same(true, true, Some(true))), None);
    }

    #[test]
    fn text_that_differs_from_both_other_documents_is_red() {
        assert_eq!(char_style(Document::A, same(true, false, Some(false))), RED);
        assert_eq!(char_style(Document::B, same(false, true, Some(false))), RED);
        assert_eq!(char_style(Document::C, same(false, false, Some(true))), RED);
    }

    #[test]
    fn text_changed_only_in_b_is_b_colored_where_it_was_kept() {
        assert_eq!(char_style(Document::A, same(true, false, Some(true))), FROM_B);
        assert_eq!(char_style(Document::C, same(true, false, Some(true))), FROM_B);
        assert_eq!(char_style(Document::B, same(false, true, Some(false))), RED);
    }

    #[test]
    fn text_changed_only_in_c_is_c_colored_where_it_was_kept() {
        assert_eq!(char_style(Document::A, same(true, true, Some(false))), FROM_C);
        assert_eq!(char_style(Document::B, same(true, true, Some(false))), FROM_C);
        assert_eq!(char_style(Document::C, same(false, false, Some(true))), RED);
    }

    #[test]
    fn text_changed_only_in_a_is_a_colored_where_b_and_c_kept_it() {
        assert_eq!(char_style(Document::B, same(false, true, Some(true))), FROM_A);
        assert_eq!(char_style(Document::C, same(false, true, Some(true))), FROM_A);
        assert_eq!(char_style(Document::A, same(true, false, Some(false))), RED);
    }

    #[test]
    fn two_way_diffs_are_marked_in_the_color_of_the_other_document() {
        assert_eq!(char_style(Document::A, same(true, false, None)), FROM_B);
        assert_eq!(char_style(Document::A, same(true, true, None)), None);
        assert_eq!(char_style(Document::B, same(false, true, None)), FROM_A);
        assert_eq!(char_style(Document::B, same(true, true, None)), None);
    }

    #[test]
    fn output_has_no_char_diff() {
        assert_eq!(char_style(Document::D, same(false, false, Some(false))), None);
    }

    const UNCHANGED_3: RegionStatus = RegionStatus::Merge(MergeStatus::Unchanged);
    const IN_B: RegionStatus = RegionStatus::Merge(MergeStatus::ChangedInB);
    const IN_C: RegionStatus = RegionStatus::Merge(MergeStatus::ChangedInC);
    const ALIKE: RegionStatus = RegionStatus::Merge(MergeStatus::ChangedAlike);
    const CONFLICT: RegionStatus = RegionStatus::Merge(MergeStatus::Conflict);
    const WHITESPACE: RegionStatus = RegionStatus::Merge(MergeStatus::WhitespaceConflict);

    #[test]
    fn regions_with_equal_hunks_have_no_style() {
        for document in [Document::A, Document::B, Document::C] {
            assert_eq!(region_style(document, HunksEqual::from_status(&UNCHANGED_3), 3), None);
        }
        assert_eq!(region_style(Document::A, HunksEqual::from_status(&RegionStatus::Diff(HunkChange::Unchanged)), 2), None);
        assert_eq!(region_style(Document::B, HunksEqual::from_status(&RegionStatus::Diff(HunkChange::Unchanged)), 2), None);
    }

    #[test]
    fn region_changed_in_b_is_b_colored_in_a_and_c_and_red_in_b() {
        assert_eq!(region_style(Document::A, HunksEqual::from_status(&IN_B), 3), FROM_B);
        assert_eq!(region_style(Document::B, HunksEqual::from_status(&IN_B), 3), RED);
        assert_eq!(region_style(Document::C, HunksEqual::from_status(&IN_B), 3), FROM_B);
    }

    #[test]
    fn region_changed_in_c_is_c_colored_in_a_and_b_and_red_in_c() {
        assert_eq!(region_style(Document::A, HunksEqual::from_status(&IN_C), 3), FROM_C);
        assert_eq!(region_style(Document::B, HunksEqual::from_status(&IN_C), 3), FROM_C);
        assert_eq!(region_style(Document::C, HunksEqual::from_status(&IN_C), 3), RED);
    }

    #[test]
    fn region_changed_alike_in_b_and_c_is_a_colored_in_both_and_red_in_a() {
        assert_eq!(region_style(Document::A, HunksEqual::from_status(&ALIKE), 3), RED);
        assert_eq!(region_style(Document::B, HunksEqual::from_status(&ALIKE), 3), FROM_A);
        assert_eq!(region_style(Document::C, HunksEqual::from_status(&ALIKE), 3), FROM_A);
    }

    #[test]
    fn conflicting_regions_are_red_everywhere() {
        for status in [CONFLICT, WHITESPACE] {
            for document in [Document::A, Document::B, Document::C] {
                assert_eq!(region_style(document, HunksEqual::from_status(&status), 3), RED);
            }
        }
    }

    #[test]
    fn two_way_regions_are_marked_in_the_color_of_the_other_document() {
        for change in [HunkChange::Added, HunkChange::Removed, HunkChange::Modified] {
            assert_eq!(region_style(Document::A, HunksEqual::from_status(&RegionStatus::Diff(change)), 2), FROM_B);
            assert_eq!(region_style(Document::B, HunksEqual::from_status(&RegionStatus::Diff(change)), 2), FROM_A);
        }
    }

    #[test]
    fn output_regions_have_no_style() {
        assert_eq!(region_style(Document::D, HunksEqual::from_status(&CONFLICT), 3), None);
    }

    fn run(columns: Range<usize>, same_in: &[bool]) -> CharRun {
        CharRun {
            columns,
            same_in: same_in.to_vec(),
        }
    }

    #[test]
    fn segments_join_neighbors_with_the_same_style() {
        let runs = [
            run(0..2, &[true, true, true]),
            run(2..3, &[false, true, false]),
            run(3..4, &[false, true, false]),
            run(4..6, &[true, true, false]),
            run(6..7, &[true, true, false]),
        ];
        assert_eq!(segments(Document::B, &runs, 3), [(2..4, CharStyle::Red), (4..7, CharStyle::Document(Document::C))]);
    }

    #[test]
    fn segments_keep_neighbors_with_different_styles_apart() {
        let runs = [run(0..1, &[true, false, true]), run(1..2, &[true, true, false])];
        assert_eq!(
            segments(Document::A, &runs, 3),
            [(0..1, CharStyle::Document(Document::B)), (1..2, CharStyle::Document(Document::C))]
        );
    }

    #[test]
    fn a_word_changed_in_one_document_is_that_documents_color_in_the_others_and_red_in_it() {
        let a = [
            run(0..12, &[true, true, true]),
            run(12..17, &[true, false, true]),
            run(17..20, &[true, true, true]),
            run(20..28, &[true, true, false]),
            run(28..29, &[true, true, true]),
        ];
        let b = [
            run(0..12, &[true, true, true]),
            run(12..16, &[false, true, false]),
            run(16..27, &[true, true, false]),
            run(27..28, &[true, true, true]),
        ];
        let c = [
            run(0..12, &[true, true, true]),
            run(12..20, &[true, false, true]),
            run(20..31, &[false, false, true]),
            run(31..32, &[true, true, true]),
        ];
        let (from_b, from_c) = (CharStyle::Document(Document::B), CharStyle::Document(Document::C));
        assert_eq!(segments(Document::A, &a, 3), [(12..17, from_b), (20..28, from_c)]);
        assert_eq!(segments(Document::B, &b, 3), [(12..16, CharStyle::Red), (16..27, from_c)]);
        assert_eq!(segments(Document::C, &c, 3), [(12..20, from_b), (20..31, CharStyle::Red)]);
    }

    #[test]
    fn segments_leave_plain_runs_out() {
        let runs = [run(0..2, &[true, true]), run(2..3, &[true, false]), run(3..5, &[true, true])];
        assert_eq!(segments(Document::A, &runs, 2), [(2..3, CharStyle::Document(Document::B))]);
    }

    #[test]
    fn segments_treat_missing_flags_as_different() {
        assert_eq!(segments(Document::B, &[run(0..1, &[])], 3), [(0..1, CharStyle::Red)]);
    }


    fn reflowed_region() -> Region {
        Region {
            hunks: vec![1..2, 1..4, 1..2],
            status: RegionStatus::Merge(MergeStatus::Conflict),
            rows: Vec::new(),
        }
    }


    fn lines(lines: &[Vec<CharRun>]) -> LineCharRuns {
        let mut all = vec![Vec::new(); 1];
        all.extend(lines.iter().cloned());
        LineCharRuns { lines: all }
    }


    #[test]
    fn hunks_whose_runs_mark_nothing_between_them_are_equal() {
        let a = lines(&[vec![run(0..5, &[true, true, false])]]);
        let b = lines(&[vec![run(0..2, &[true, true, false])], vec![], vec![run(2..4, &[true, true, false])]]);
        let c = lines(&[vec![run(0..5, &[false, false, true])]]);
        let equal = HunksEqual::from_status(&RegionStatus::Merge(MergeStatus::Conflict)).or_from_char_diffs(&reflowed_region(), &[a, b, c]);
        assert_eq!(equal, HunksEqual { a_b: true, a_c: false, b_c: false });
        assert_eq!(region_style(Document::A, equal, 3), FROM_C);
        assert_eq!(region_style(Document::C, equal, 3), RED);
    }


    #[test]
    fn a_change_marked_on_one_side_only_keeps_hunks_different() {
        let a = lines(&[vec![run(0..5, &[true, true, true])]]);
        let b = lines(&[vec![run(0..2, &[true, true, true])], vec![run(0..1, &[false, true, false])], vec![]]);
        let c = lines(&[vec![run(0..5, &[true, true, true])]]);
        let equal = HunksEqual::from_status(&RegionStatus::Merge(MergeStatus::Conflict)).or_from_char_diffs(&reflowed_region(), &[a, b, c]);
        assert_eq!(equal, HunksEqual { a_b: false, a_c: true, b_c: false });
    }


    #[test]
    fn status_equality_holds_whatever_the_runs_say() {
        let equal = HunksEqual::from_status(&RegionStatus::Merge(MergeStatus::ChangedInB)).or_from_char_diffs(&reflowed_region(), &[]);
        assert_eq!(equal, HunksEqual { a_b: false, a_c: true, b_c: false });
    }


    #[test]
    fn a_two_way_region_has_no_c_to_be_equal_to() {
        let region = Region {
            hunks: vec![1..2, 1..2],
            status: RegionStatus::Diff(HunkChange::Modified),
            rows: Vec::new(),
        };
        let both = lines(&[vec![run(0..1, &[true, true])]]);
        let equal = HunksEqual::from_status(&region.status).or_from_char_diffs(&region, &[both.clone(), both]);
        assert_eq!(equal, HunksEqual { a_b: true, a_c: false, b_c: false });
        assert_eq!(region_style(Document::A, equal, 2), None);
    }
}
