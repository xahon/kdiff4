//! Folding: the fold ranges the engine found in one document, which of them are folded, and how folding, unfolding and edits of the text change that.

use std::ops::Range;


/// The fold ranges of one document and the folds among them. A range starts at its header line, which stays visible when it is folded; the lines after it up to its end are hidden. Folds may nest: folding one leaves the folds inside it as they are.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Folds {
    ranges: Vec<Range<usize>>,
    folded: Vec<Range<usize>>,
}

impl Folds {
    /// Takes the ranges the engine found, sorted by start. Folds that are no longer a range are unfolded.
    pub fn set_ranges(&mut self, ranges: Vec<Range<usize>>) {
        self.folded.retain(|fold| ranges.contains(fold));
        self.ranges = ranges;
    }


    #[cfg(test)]
    pub fn ranges(&self) -> &[Range<usize>] {
        &self.ranges
    }


    /// The folded ranges, sorted by start.
    pub fn folded(&self) -> &[Range<usize>] {
        &self.folded
    }


    /// The range whose header is `line`.
    pub fn range_at(&self, line: usize) -> Option<Range<usize>> {
        self.ranges.iter().find(|range| range.start == line).cloned()
    }


    /// Whether the range whose header is `line` is folded.
    pub fn is_folded(&self, line: usize) -> bool {
        self.folded.iter().any(|fold| fold.start == line)
    }


    /// Whether a fold hides `line`.
    #[cfg(test)]
    pub fn hides(&self, line: usize) -> bool {
        self.folded.iter().any(|fold| hides(fold, line))
    }


    /// The header of the outermost fold that hides `line`.
    pub fn outermost_hiding(&self, line: usize) -> Option<usize> {
        self.folded
            .iter()
            .filter(|fold| hides(fold, line))
            .min_by_key(|fold| fold.start.clone())
            .map(|fold| fold.start.clone())
    }


    /// The innermost range around `line`, its header included, that is not folded yet: what folding at a caret on `line` folds.
    pub fn innermost_unfolded(&self, line: usize) -> Option<Range<usize>> {
        self.ranges
            .iter()
            .filter(|range| range.contains(&line) && !self.folded.contains(range))
            .max_by_key(|range| (range.start.clone(), std::cmp::Reverse(range.end.clone())))
            .cloned()
    }


    /// The fold whose header row shows `line`'s closing token after its Fold Ellipsis: the outermost fold that hides `line`, when every fold that hides it ends with it.
    pub fn closing_fold(&self, line: usize) -> Option<Range<usize>> {
        let mut hiding = self.folded.iter().filter(|fold| hides(fold, line)).peekable();
        hiding.peek()?;
        let mut outermost: Option<&Range<usize>> = None;
        for fold in hiding {
            if fold.end != line + 1 {
                return None;
            }
            if outermost.is_none_or(|outer| fold.start < outer.start) {
                outermost = Some(fold);
            }
        }
        outermost.cloned()
    }


    /// Folds `range`, which must be one of the ranges. Returns whether anything changed.
    pub fn fold(&mut self, range: Range<usize>) -> bool {
        if !self.ranges.contains(&range) || self.folded.contains(&range) {
            return false;
        }
        let at = self.folded.partition_point(|fold| (fold.start.clone(), std::cmp::Reverse(fold.end.clone())) < (range.start.clone(), std::cmp::Reverse(range.end.clone())));
        self.folded.insert(at, range);
        true
    }


    /// Unfolds the fold whose header is `line`, or else the innermost fold that hides `line`. Returns whether anything changed.
    pub fn unfold_at(&mut self, line: usize) -> bool {
        let index = self.folded.iter().position(|fold| fold.start == line).or_else(|| {
            self.folded
                .iter()
                .enumerate()
                .filter(|(_, fold)| hides(fold, line))
                .max_by_key(|(_, fold)| fold.start.clone())
                .map(|(index, _)| index)
        });
        match index {
            Some(index) => {
                self.folded.remove(index);
                true
            }
            None => false,
        }
    }


    /// Unfolds every fold that hides one of `lines`, so they can all be seen. An empty range reveals the line it starts at. Returns whether anything changed.
    pub fn reveal(&mut self, lines: Range<usize>) -> bool {
        let lines = lines.start.clone()..lines.end.max(lines.start + 1);
        let count = self.folded.len();
        self.folded.retain(|fold| !(fold.start + 1 < lines.end && lines.start < fold.end));
        self.folded.len() != count
    }


    /// Folds the outermost ranges, those inside no other range; the ranges inside them keep their state. Returns whether anything changed.
    pub fn fold_all(&mut self) -> bool {
        let outermost: Vec<Range<usize>> = self
            .ranges
            .iter()
            .filter(|range| !self.ranges.iter().any(|outer| outer != *range && outer.start <= range.start && range.end <= outer.end))
            .cloned()
            .collect();
        let mut changed = false;
        for range in outermost {
            changed |= self.fold(range);
        }
        changed
    }


    /// Returns whether anything changed.
    pub fn unfold_all(&mut self) -> bool {
        let changed = !self.folded.is_empty();
        self.folded.clear();
        changed
    }


    /// Follows an edit that replaced the text from line `first` to line `old_last` with text that ends on line `new_last`. Ranges below the edit move with their lines; those it touches are dropped, except a range whose header line alone was edited in place. The engine is asked for the ranges again after an edit, so dropped ranges come back where they still are.
    pub fn edited(&mut self, first: usize, old_last: usize, new_last: usize) {
        let follow = |ranges: &mut Vec<Range<usize>>| {
            *ranges = ranges
                .iter()
                .filter_map(|range| shifted(range, first.clone(), old_last.clone(), new_last.clone()))
                .collect();
        };
        follow(&mut self.ranges);
        follow(&mut self.folded);
    }
}


/// The columns, in characters, of the closing token a fold that ends with `line` shows after its Fold Ellipsis: the closing brackets the line starts with, with the `;` and `,` among and right after them, as in `}`, `};` or `});`.
pub fn closing_token(line: &str) -> Option<Range<usize>> {
    let indent = line.chars().take_while(|character| character.is_whitespace()).count();
    let mut token = line.chars().skip(indent.clone());
    if !matches!(token.next(), Some('}' | ')' | ']')) {
        return None;
    }
    let length = 1 + token.take_while(|character| matches!(character, '}' | ')' | ']' | ';' | ',')).count();
    Some(indent.clone()..indent + length)
}


/// Whether the caret at `column` of `line`, whose text is `text`, is on a closing token a Fold Ellipsis shows, where it stays without unfolding.
pub fn on_shown_closing(folds: &Folds, line: usize, text: &str, column: usize) -> bool {
    folds.closing_fold(line).is_some() && closing_token(text).is_some_and(|token| token.start <= column && column <= token.end)
}


fn hides(fold: &Range<usize>, line: usize) -> bool {
    fold.start < line && line < fold.end
}


fn shifted(range: &Range<usize>, first: usize, old_last: usize, new_last: usize) -> Option<Range<usize>> {
    if first >= range.end {
        return Some(range.clone());
    }
    if old_last < range.start {
        let moved = |line: usize| (line + new_last).checked_sub(old_last);
        return Some(moved(range.start.clone())?..moved(range.end.clone())?);
    }
    let in_header = first == range.start && old_last == range.start && new_last == old_last;
    in_header.then(|| range.clone())
}


#[cfg(test)]
mod tests {
    use super::*;

    fn folds(ranges: &[Range<usize>]) -> Folds {
        let mut folds = Folds::default();
        folds.set_ranges(ranges.to_vec());
        folds
    }


    #[test]
    fn folding_at_a_line_takes_the_innermost_range_around_it_that_is_not_folded() {
        let mut folds = folds(&[0..10, 2..6, 3..5]);
        assert_eq!(folds.innermost_unfolded(4), Some(3..5));
        assert_eq!(folds.innermost_unfolded(2), Some(2..6));
        assert_eq!(folds.innermost_unfolded(8), Some(0..10));
        assert_eq!(folds.innermost_unfolded(10), None);
        assert!(folds.fold(2..6));
        assert_eq!(folds.innermost_unfolded(2), Some(0..10));
        assert!(!folds.fold(2..6));
        assert!(!folds.fold(1..4));
    }


    #[test]
    fn a_fold_hides_the_lines_after_its_header() {
        let mut folds = folds(&[2..6]);
        folds.fold(2..6);
        assert_eq!((folds.hides(2), folds.hides(3), folds.hides(5), folds.hides(6)), (false, true, true, false));
        assert!(folds.is_folded(2));
    }


    #[test]
    fn unfolding_takes_the_fold_of_the_header_or_else_the_innermost_that_hides_the_line() {
        let mut folds = folds(&[0..10, 2..6]);
        folds.fold(0..10);
        folds.fold(2..6);
        assert!(folds.unfold_at(4));
        assert_eq!(folds.folded(), [0..10]);
        assert!(!folds.unfold_at(12));
        assert!(folds.unfold_at(0));
        assert!(folds.folded().is_empty());
    }


    #[test]
    fn revealing_lines_unfolds_every_fold_that_hides_one_of_them() {
        let mut folds = folds(&[0..10, 2..6, 3..5, 7..9]);
        folds.fold(2..6);
        folds.fold(3..5);
        folds.fold(7..9);
        assert!(!folds.reveal(2..3));
        assert!(folds.reveal(4..4));
        assert_eq!(folds.folded(), [7..9]);
        folds.fold(0..10);
        assert!(folds.reveal(1..2));
        assert_eq!(folds.folded(), [7..9]);
    }


    #[test]
    fn folding_all_folds_only_the_outermost_ranges() {
        let mut folds = folds(&[0..4, 1..3, 5..9, 6..8]);
        assert!(folds.fold_all());
        assert_eq!(folds.folded(), [0..4, 5..9]);
        assert!(!folds.fold_all());
        assert!(folds.unfold_all());
        assert!(!folds.unfold_all());
    }


    #[test]
    fn new_ranges_drop_the_folds_that_are_gone() {
        let mut folds = folds(&[0..4, 5..9]);
        folds.fold_all();
        folds.set_ranges(vec![0..4, 5..8]);
        assert_eq!(folds.folded(), [0..4]);
    }


    #[test]
    fn the_closing_token_is_the_leading_closing_brackets_with_their_punctuation() {
        assert_eq!(closing_token("}"), Some(0..1));
        assert_eq!(closing_token("    }; // end"), Some(4..6));
        assert_eq!(closing_token("\t});"), Some(1..4));
        assert_eq!(closing_token("} while (x);"), Some(0..1));
        assert_eq!(closing_token("    return 0;"), None);
        assert_eq!(closing_token(""), None);
    }


    #[test]
    fn a_closing_line_shows_on_the_outermost_fold_that_ends_with_it() {
        let mut folds = folds(&[0..10, 2..10, 3..6]);
        assert_eq!(folds.closing_fold(9), None);
        folds.fold(2..10);
        assert_eq!(folds.closing_fold(9), Some(2..10));
        assert_eq!(folds.closing_fold(8), None);
        folds.fold(0..10);
        assert_eq!(folds.closing_fold(9), Some(0..10));
        folds.fold(3..6);
        assert_eq!(folds.closing_fold(5), None);
    }


    #[test]
    fn the_caret_stays_folded_only_on_a_shown_closing_token() {
        let mut folds = folds(&[0..3]);
        assert!(!on_shown_closing(&folds, 2, "  };", 2));
        folds.fold(0..3);
        assert_eq!((on_shown_closing(&folds, 2, "  };", 1), on_shown_closing(&folds, 2, "  };", 2), on_shown_closing(&folds, 2, "  };", 4)), (false, true, true));
        assert!(!on_shown_closing(&folds, 2, "  x;", 2));
        assert!(!on_shown_closing(&folds, 1, "}", 0));
    }


    #[test]
    fn edits_move_the_folds_below_them_and_drop_those_they_touch() {
        let mut folds = folds(&[0..3, 5..8, 10..12]);
        folds.fold_all();
        folds.edited(4, 4, 6);
        assert_eq!(folds.folded(), [0..3, 7..10, 12..14]);
        folds.edited(8, 8, 8);
        assert_eq!(folds.folded(), [0..3, 12..14]);
        folds.edited(0, 0, 0);
        assert_eq!(folds.folded(), [0..3, 12..14]);
        folds.edited(1, 5, 1);
        assert_eq!(folds.folded(), [8..10]);
        assert_eq!(folds.ranges(), [8..10]);
    }
}
