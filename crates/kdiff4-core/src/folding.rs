//! Fold ranges: the parts of a document a pane can fold, from the entities tree-sitter finds where it parses the language, and from the indentation otherwise.

use std::ops::Range;

use crate::heuristics::{self, DocumentLines};

const TAB_WIDTH: usize = 4;


/// The fold ranges of one document of `language` (a name the language selector shows), sorted by start. Each starts at its header line, which stays visible when it is folded, and holds at least one line after it. Of the ranges that start on the same line only the longest is kept.
pub fn fold_ranges(language: &str, text: &str, lines: &[Range<usize>]) -> Vec<Range<usize>> {
    let document = DocumentLines { text, lines };
    let ranges = heuristics::entity_spans(language, &document).unwrap_or_else(|| indent_ranges(text, lines));
    let mut kept: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if range.end > lines.len() || range.len() < 2 {
            continue;
        }
        match kept.last_mut() {
            Some(last) if last.start == range.start => last.end = last.end.clone().max(range.end),
            _ => kept.push(range),
        }
    }
    kept.sort_by_key(|range| range.start.clone());
    kept
}


/// A line with text and the lines below it that are more indented or blank, up to the last more indented one, the way VS Code folds by indentation. A closing bracket line right after them at the indentation of the first line belongs to it too, so a folded block can show its closing bracket.
fn indent_ranges(text: &str, lines: &[Range<usize>]) -> Vec<Range<usize>> {
    let indents: Vec<Option<usize>> = lines
        .iter()
        .map(|range| indent_of(text.get(range.clone()).unwrap_or("")))
        .collect();
    let mut ranges = Vec::new();
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut last_text: Option<usize> = None;
    for (line, indent) in indents.iter().enumerate() {
        let Some(indent) = indent.clone() else {
            continue;
        };
        while let Some((start, open_indent)) = open.last().cloned() {
            if open_indent < indent {
                break;
            }
            open.pop();
            close(&mut ranges, start, last_text);
        }
        open.push((line, indent));
        last_text = Some(line);
    }
    while let Some((start, _)) = open.pop() {
        close(&mut ranges, start, last_text);
    }
    for range in ranges.iter_mut() {
        let next = (range.end..lines.len()).find(|line| indents[*line].is_some());
        if let Some(next) = next
            && indents[next] == indents[range.start]
            && text.get(lines[next].clone()).unwrap_or("").trim_start().starts_with(['}', ')', ']'])
        {
            range.end = next + 1;
        }
    }
    ranges.sort_by_key(|range: &Range<usize>| range.start.clone());
    ranges
}


fn close(ranges: &mut Vec<Range<usize>>, start: usize, last_text: Option<usize>) {
    if let Some(last) = last_text
        && last > start
    {
        ranges.push(start..last + 1);
    }
}


/// The width of the leading whitespace of `line`, or `None` for a blank line.
fn indent_of(line: &str) -> Option<usize> {
    if line.trim().is_empty() {
        return None;
    }
    let mut width = 0;
    for character in line.chars() {
        match character {
            ' ' => width += 1,
            '\t' => width += TAB_WIDTH - width % TAB_WIDTH,
            _ => break,
        }
    }
    Some(width)
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::split_lines;

    fn ranges(language: &str, text: &str) -> Vec<Range<usize>> {
        fold_ranges(language, text, &split_lines(text))
    }


    #[test]
    fn indentation_folds_the_more_indented_lines_below_a_line() {
        let text = "def f():\n    a = 1\n\n    if a:\n        b()\n\nx = 2\n";
        assert_eq!(ranges("Python", text), [0..5, 3..5]);
    }


    #[test]
    fn a_closing_bracket_line_at_the_header_indentation_ends_an_indented_fold() {
        let text = "impl S {\n    fn f() {\n        g();\n\n    }\n}\nx\n  y\nz\n";
        assert_eq!(ranges("Rust", text), [0..6, 1..5, 6..8]);
    }


    #[test]
    fn tabs_count_to_the_next_tab_stop() {
        let text = "a:\n\tb\n  \tc\nd\n";
        assert_eq!(ranges("YAML", text), [0..3]);
    }


    #[test]
    fn a_line_without_more_indented_lines_below_does_not_fold() {
        assert_eq!(ranges("Plain text", "a\nb\n\n  \nc\n"), Vec::<Range<usize>>::new());
    }


    #[test]
    fn cpp_folds_functions_types_and_blocks_from_the_line_their_signature_ends_on() {
        let text = "\
// comment
int f(int n)
{
    if (n > 0) {
        n--;
        n--;
    }
    return n;
}
struct S {
    int g() {
        return 1;
    }
};
";
        assert_eq!(ranges("C++", text), [1..9, 3..7, 9..14, 10..13]);
    }


    #[test]
    fn cpp_keeps_the_longest_range_of_those_that_start_on_one_line() {
        let text = "void f() { if (x) {\n    y();\n    z();\n} }\n";
        assert_eq!(ranges("C++", text), [0..4]);
    }


    #[test]
    fn c_folds_through_tree_sitter_and_other_languages_by_indentation() {
        let text = "int f(void) {\n    return 0;\n}\n";
        assert_eq!(ranges("C", text), [0..3]);
        assert_eq!(ranges("Go", text), [0..3]);
    }
}
