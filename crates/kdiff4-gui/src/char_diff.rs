use std::ops::Range;

use egui::Color32;
use kdiff4_protocol::Document;
use kdiff4_protocol::engine::CharRun;


/// One of the configured char-diff colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharColor {
    Red,
    Blue,
    Green,
}


/// How a run of characters is drawn: its text color, and whether a border goes around it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharStyle {
    pub color: CharColor,
    pub border: bool,
}


/// Where the characters of a run are the same, at the same place of the line in the row: in A, in B, and in C, which is `None` in a 2-way diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SameIn {
    pub a: bool,
    pub b: bool,
    pub c: Option<bool>,
}


/// The char-diff rules: how a run of `document` is drawn, from where its characters are the same. `None` is plain text. A 2-way diff follows the same rules with C missing.
pub fn char_style(document: Document, same: SameIn) -> Option<CharStyle> {
    let bordered = |color: CharColor| Some(CharStyle { color, border: true });
    let plain = |color: CharColor| Some(CharStyle { color, border: false });
    let missing_from_b_or_c = !same.b || same.c == Some(false);
    let same_in_other_side = match document {
        Document::B => same.c.unwrap_or(false),
        Document::C => same.b,
        Document::A | Document::D => false,
    };
    match (document, same.a, same_in_other_side) {
        (Document::A, _, _) if missing_from_b_or_c => bordered(CharColor::Red),
        (Document::A, _, _) => None,
        (Document::B | Document::C, false, false) => bordered(CharColor::Red),
        (Document::B | Document::C, false, true) => bordered(CharColor::Blue),
        (Document::B | Document::C, true, _) => plain(CharColor::Green),
        (Document::D, _, _) => None,
    }
}


/// The colors and border rounding char-diff runs are drawn with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CharDiffLook {
    pub red: Color32,
    pub blue: Color32,
    pub green: Color32,
    pub corner_radius: u8,
}


impl CharDiffLook {
    pub fn color(&self, color: CharColor) -> Color32 {
        match color {
            CharColor::Red => self.red.clone(),
            CharColor::Blue => self.blue.clone(),
            CharColor::Green => self.green.clone(),
        }
    }
}


/// The styled column ranges of a line of `document` with `runs`, in order; neighbors with the same style are joined, so a border goes around all of them. `sources` is how many source documents the row has. Plain runs are left out.
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


    const RED: Option<CharStyle> = Some(CharStyle { color: CharColor::Red, border: true });
    const BLUE: Option<CharStyle> = Some(CharStyle { color: CharColor::Blue, border: true });
    const GREEN: Option<CharStyle> = Some(CharStyle { color: CharColor::Green, border: false });


    fn same(a: bool, b: bool, c: Option<bool>) -> SameIn {
        SameIn { a, b, c }
    }


    #[test]
    fn changes_made_on_one_side_only_are_red() {
        assert_eq!(char_style(Document::B, same(false, true, Some(false))), RED);
        assert_eq!(char_style(Document::C, same(false, false, Some(true))), RED);
    }


    #[test]
    fn changes_made_alike_in_b_and_c_are_blue() {
        assert_eq!(char_style(Document::B, same(false, true, Some(true))), BLUE);
        assert_eq!(char_style(Document::C, same(false, true, Some(true))), BLUE);
    }


    #[test]
    fn text_kept_from_a_is_green() {
        assert_eq!(char_style(Document::B, same(true, true, Some(true))), GREEN);
        assert_eq!(char_style(Document::C, same(true, false, Some(true))), GREEN);
    }


    #[test]
    fn text_of_a_missing_from_b_or_c_is_red_in_a() {
        assert_eq!(char_style(Document::A, same(true, false, Some(true))), RED);
        assert_eq!(char_style(Document::A, same(true, true, Some(false))), RED);
        assert_eq!(char_style(Document::A, same(true, true, Some(true))), None);
    }


    #[test]
    fn two_way_diffs_follow_the_same_rules_without_c() {
        assert_eq!(char_style(Document::A, same(true, false, None)), RED);
        assert_eq!(char_style(Document::A, same(true, true, None)), None);
        assert_eq!(char_style(Document::B, same(false, true, None)), RED);
        assert_eq!(char_style(Document::B, same(true, true, None)), GREEN);
    }


    #[test]
    fn output_has_no_char_diff() {
        assert_eq!(char_style(Document::D, same(false, false, Some(false))), None);
    }


    fn run(columns: Range<usize>, same_in: &[bool]) -> CharRun {
        CharRun { columns, same_in: same_in.to_vec() }
    }


    #[test]
    fn segments_join_neighbors_with_the_same_style() {
        let runs = [run(0..2, &[true, true, true]), run(2..3, &[false, true, false]), run(3..4, &[false, true, false]), run(4..6, &[true, true, false])];
        let red = RED.unwrap();
        let green = GREEN.unwrap();
        assert_eq!(segments(Document::B, &runs, 3), [(0..2, green), (2..4, red), (4..6, green)]);
    }


    #[test]
    fn segments_leave_plain_runs_out() {
        let runs = [run(0..2, &[true, true]), run(2..3, &[true, false]), run(3..5, &[true, true])];
        assert_eq!(segments(Document::A, &runs, 2), [(2..3, RED.unwrap())]);
    }


    #[test]
    fn segments_treat_missing_flags_as_different() {
        assert_eq!(segments(Document::B, &[run(0..1, &[])], 3), [(0..1, RED.unwrap())]);
    }
}
