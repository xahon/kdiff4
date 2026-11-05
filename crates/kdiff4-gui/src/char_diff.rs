use std::ops::Range;

use egui::Color32;
use kdiff4_protocol::Document;
use kdiff4_protocol::engine::CharRun;


/// How a run of characters is marked: red when its characters differ from every other document of the row, otherwise in the color of the one other document they differ from.
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


/// The char-diff rules: how a run of `document` is marked, from where its characters are the same. `None` is plain text. A document with no line in the row is never the same, and a 2-way diff follows the same rules with C missing.
pub fn char_style(document: Document, same: SameIn) -> Option<CharStyle> {
    if document == Document::D {
        return None;
    }
    let others: Vec<(Document, bool)> = [(Document::A, Some(same.a)), (Document::B, Some(same.b)), (Document::C, same.c)]
        .into_iter()
        .filter(|(other, _)| *other != document)
        .filter_map(|(other, same)| same.map(|same| (other, same)))
        .collect();
    let different: Vec<Document> = others
        .iter()
        .filter(|(_, same)| !same)
        .map(|(other, _)| other.clone())
        .collect();
    match different.as_slice() {
        [] => None,
        [other] if others.len() > 1 => Some(CharStyle::Document(other.clone())),
        _ => Some(CharStyle::Red),
    }
}


/// The color red char-diff runs are drawn with; runs of a document are drawn with that document's color of the theme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CharDiffLook {
    pub red: Color32,
}


impl CharDiffLook {
    /// The color of the text of a run: red text for red runs, `plain` for the runs of a document.
    pub fn text_color(&self, style: &CharStyle, plain: Color32) -> Color32 {
        match style {
            CharStyle::Red => self.red.clone(),
            CharStyle::Document(_) => plain,
        }
    }
}


/// The styled column ranges of a line of `document` with `runs`, in order; neighbors with the same style are joined, so one background covers all of them. `sources` is how many source documents the row has. Plain runs are left out.
pub fn segments(document: Document, runs: &[CharRun], sources: usize) -> Vec<(Range<usize>, CharStyle)> {
    let flag = |run: &CharRun, index: usize| run.same_in.get(index).cloned().unwrap_or(false);
    let mut segments: Vec<(Range<usize>, CharStyle)> = Vec::new();
    for run in runs {
        let same = SameIn { a: flag(run, 0), b: flag(run, 1), c: (sources > 2).then(|| flag(run, 2)) };
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
    fn two_way_diffs_follow_the_same_rules_without_c() {
        assert_eq!(char_style(Document::A, same(true, false, None)), RED);
        assert_eq!(char_style(Document::A, same(true, true, None)), None);
        assert_eq!(char_style(Document::B, same(false, true, None)), RED);
        assert_eq!(char_style(Document::B, same(true, true, None)), None);
    }


    #[test]
    fn output_has_no_char_diff() {
        assert_eq!(char_style(Document::D, same(false, false, Some(false))), None);
    }


    #[test]
    fn only_red_text_has_its_own_color() {
        let look = CharDiffLook { red: Color32::RED };
        assert_eq!(look.text_color(&CharStyle::Red, Color32::BLACK), Color32::RED);
        assert_eq!(look.text_color(&CharStyle::Document(Document::B), Color32::BLACK), Color32::BLACK);
    }


    fn run(columns: Range<usize>, same_in: &[bool]) -> CharRun {
        CharRun { columns, same_in: same_in.to_vec() }
    }


    #[test]
    fn segments_join_neighbors_with_the_same_style() {
        let runs = [run(0..2, &[true, true, true]), run(2..3, &[false, true, false]), run(3..4, &[false, true, false]), run(4..6, &[true, true, false]), run(6..7, &[true, true, false])];
        assert_eq!(segments(Document::B, &runs, 3), [(2..4, CharStyle::Red), (4..7, CharStyle::Document(Document::C))]);
    }


    #[test]
    fn segments_keep_neighbors_with_different_styles_apart() {
        let runs = [run(0..1, &[true, false, true]), run(1..2, &[true, true, false])];
        assert_eq!(segments(Document::A, &runs, 3), [(0..1, CharStyle::Document(Document::B)), (1..2, CharStyle::Document(Document::C))]);
    }


    #[test]
    fn a_word_changed_in_one_document_is_that_documents_color_in_the_others_and_red_in_it() {
        let a = [run(0..12, &[true, true, true]), run(12..17, &[true, false, true]), run(17..20, &[true, true, true]), run(20..28, &[true, true, false]), run(28..29, &[true, true, true])];
        let b = [run(0..12, &[true, true, true]), run(12..16, &[false, true, false]), run(16..27, &[true, true, false]), run(27..28, &[true, true, true])];
        let c = [run(0..12, &[true, true, true]), run(12..20, &[true, false, true]), run(20..31, &[false, false, true]), run(31..32, &[true, true, true])];
        let (from_b, from_c) = (CharStyle::Document(Document::B), CharStyle::Document(Document::C));
        assert_eq!(segments(Document::A, &a, 3), [(12..17, from_b), (20..28, from_c)]);
        assert_eq!(segments(Document::B, &b, 3), [(12..16, CharStyle::Red), (16..27, from_c)]);
        assert_eq!(segments(Document::C, &c, 3), [(12..20, from_b), (20..31, CharStyle::Red)]);
    }


    #[test]
    fn segments_leave_plain_runs_out() {
        let runs = [run(0..2, &[true, true]), run(2..3, &[true, false]), run(3..5, &[true, true])];
        assert_eq!(segments(Document::A, &runs, 2), [(2..3, CharStyle::Red)]);
    }


    #[test]
    fn segments_treat_missing_flags_as_different() {
        assert_eq!(segments(Document::B, &[run(0..1, &[])], 3), [(0..1, CharStyle::Red)]);
    }
}
