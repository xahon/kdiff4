use std::ops::Range;

use crate::config::{MAX_TAB_SIZE, TabIndentation};

/// A caret position: line index and column, counted in characters. Orders by line, then column.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

/// The caret (`head`) and the other end of the selection (`anchor`). Equal ends mean no selection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cursor {
    pub anchor: Position,
    pub head: Position,
}

impl Cursor {
    pub fn at(position: Position) -> Cursor {
        Cursor { anchor: position, head: position }
    }

    pub fn has_selection(&self) -> bool {
        self.anchor != self.head
    }

    /// The selection's ends in document order.
    pub fn range(&self) -> (Position, Position) {
        if self.anchor <= self.head { (self.anchor, self.head) } else { (self.head, self.anchor) }
    }
}

/// A view of text split into lines by byte ranges.
#[derive(Debug, Clone, Copy)]
pub struct Lines<'a> {
    pub text: &'a str,
    pub ranges: &'a [Range<usize>],
}

impl<'a> Lines<'a> {
    pub fn count(&self) -> usize {
        self.ranges.len()
    }

    /// Returns an empty string for an index or range that doesn't fit the text.
    pub fn line(&self, index: usize) -> &'a str {
        self.ranges.get(index).and_then(|range| self.text.get(range.clone())).unwrap_or("")
    }

    /// What ends a line in `text`: `"\r\n"`, `"\n"`, `"\r"`, or nothing for the last line or an index that doesn't fit.
    pub fn terminator(&self, index: usize) -> &'a str {
        let rest = match self.ranges.get(index) {
            Some(range) => self.text.get(range.end..).unwrap_or(""),
            None => return "",
        };
        if rest.starts_with("\r\n") {
            "\r\n"
        } else if rest.starts_with('\n') {
            "\n"
        } else if rest.starts_with('\r') {
            "\r"
        } else {
            ""
        }
    }

    pub fn line_chars(&self, index: usize) -> usize {
        self.line(index).chars().count()
    }

    /// Byte offset in `text` of a position, which must be clamped.
    fn offset(&self, position: Position) -> usize {
        let start = self.ranges.get(position.line).map(|range| range.start).unwrap_or(0);
        start + char_to_byte(self.line(position.line), position.column)
    }
}

/// Samples leading whitespace and votes for the most frequent increase in space indentation.
pub fn detect_indentation(lines: Lines<'_>) -> (TabIndentation, Option<u8>) {
    let mut tab_lines = 0;
    let mut space_lines = 0;
    let mut previous_spaces = None;
    let mut steps = [0; MAX_TAB_SIZE as usize + 1];
    for index in 0..lines.count().min(1_000) {
        let line = lines.line(index).as_bytes();
        let length = line.iter().take(MAX_TAB_SIZE as usize + 1).take_while(|byte| matches!(byte, b' ' | b'\t')).count();
        if length == line.len() || length > MAX_TAB_SIZE as usize {
            continue;
        }
        let prefix = &line[..length];
        if prefix.contains(&b'\t') {
            tab_lines += usize::from(!prefix.contains(&b' '));
            previous_spaces = None;
        } else {
            space_lines += usize::from(length > 0);
            if let Some(previous) = previous_spaces
                && length > previous
            {
                steps[length - previous] += 1;
            }
            previous_spaces = Some(length);
        }
    }
    if tab_lines >= space_lines {
        return (TabIndentation::Tabs, None);
    }
    let votes = steps.iter().copied().max().unwrap_or(0);
    let mut widths = (1..steps.len()).filter(|width| steps[*width] == votes);
    let width = widths.next().filter(|_| votes > 0 && widths.next().is_none());
    (TabIndentation::Spaces, width.map(|width| width as u8))
}


/// The effective indentation for the merge output after applying its defaults or file override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Indentation {
    pub spaces: bool,
    pub size: u8,
}


impl Indentation {
    /// Resolves Tab to text at the selection start, where the replacement will be inserted.
    pub fn tab_text(self, lines: Lines<'_>, cursor: Cursor) -> String {
        if !self.spaces {
            return "\t".to_string();
        }
        let width = usize::from(self.size.clamp(1, MAX_TAB_SIZE));
        let start = clamp(lines, cursor.range().0);
        let column = lines.line(start.line).chars().take(start.column).fold(0, |column, character| {
            column + if character == '\t' { width - column % width } else { 1 }
        });
        " ".repeat(width - column % width)
    }
}


/// D's lines: split at `\n` only, and a final `\n` starts an empty last line the caret can go to. Never empty.
pub fn editor_lines(text: &str) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            lines.push(start..index);
            start = index + 1;
        }
    }
    lines.push(start..text.len());
    lines
}

/// Updates `lines`, the editor lines of a text, after `old_range` of the text was replaced by `inserted_len` bytes; `text` is the text after the change. Only the lines the change touched are split again; the ones after it are shifted.
pub fn update_editor_lines(lines: &mut Vec<Range<usize>>, text: &str, old_range: Range<usize>, inserted_len: usize) {
    if lines.is_empty() {
        *lines = editor_lines(text);
        return;
    }
    let last_index = lines.len() - 1;
    let first = lines.partition_point(|line| line.end < old_range.start).min(last_index);
    let last = lines.partition_point(|line| line.end < old_range.end).min(last_index);
    let region_start = lines[first].start;
    let region_end = (lines[last].end + inserted_len).saturating_sub(old_range.len()).min(text.len());
    let mut replacement = Vec::new();
    let mut start = region_start;
    for (index, byte) in text.as_bytes()[region_start..region_end].iter().enumerate() {
        if *byte == b'\n' {
            replacement.push(start..region_start + index);
            start = region_start + index + 1;
        }
    }
    replacement.push(start..region_end);
    let shift = inserted_len as isize - old_range.len() as isize;
    for line in &mut lines[last + 1..] {
        *line = line.start.saturating_add_signed(shift)..line.end.saturating_add_signed(shift);
    }
    lines.splice(first..=last, replacement);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    /// To the start of the word left of the caret (Ctrl+Left).
    WordLeft,
    /// To the start of the next word (Ctrl+Right), as on Windows.
    WordRight,
    LineStart,
    LineEnd,
    PageUp(usize),
    PageDown(usize),
    DocumentStart,
    DocumentEnd,
}

/// Brings a position inside the text: the last line at most, and the line's length at most.
pub fn clamp(lines: Lines<'_>, position: Position) -> Position {
    if lines.count() == 0 {
        return Position::default();
    }
    let line = position.line.min(lines.count() - 1);
    Position {
        line,
        column: position.column.min(lines.line_chars(line)),
    }
}

pub fn clamp_cursor(lines: Lines<'_>, cursor: Cursor) -> Cursor {
    Cursor {
        anchor: clamp(lines, cursor.anchor),
        head: clamp(lines, cursor.head),
    }
}

/// Moves the caret. With `extend` (Shift) the anchor stays, growing or shrinking the selection; without it the selection is dropped, and Left or Right on a selection go to its start or end.
pub fn move_cursor(lines: Lines<'_>, cursor: Cursor, motion: Motion, extend: bool) -> Cursor {
    let cursor = clamp_cursor(lines, cursor);
    if !extend && cursor.has_selection() {
        let (start, end) = cursor.range();
        match motion {
            Motion::Left => return Cursor::at(start),
            Motion::Right => return Cursor::at(end),
            _ => {}
        }
    }
    let head = move_caret(lines, cursor.head, motion);
    if extend { Cursor { anchor: cursor.anchor, head } } else { Cursor::at(head) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Word,
    Space,
    Other,
}

fn char_class(character: char) -> CharClass {
    if character.is_alphanumeric() || character == '_' {
        CharClass::Word
    } else if character.is_whitespace() {
        CharClass::Space
    } else {
        CharClass::Other
    }
}

/// What a double-click selects: the word, run of whitespace, or single punctuation character at `position`. At the end of a line it is the one before.
pub fn word_at(lines: Lines<'_>, position: Position) -> Cursor {
    let position = clamp(lines, position);
    let chars: Vec<char> = lines.line(position.line).chars().collect();
    if chars.is_empty() {
        return Cursor::at(position);
    }
    let index = position.column.min(chars.len() - 1);
    let class = char_class(chars[index]);
    let (mut start, mut end) = (index, index + 1);
    if class != CharClass::Other {
        while start > 0 && char_class(chars[start - 1]) == class {
            start -= 1;
        }
        while end < chars.len() && char_class(chars[end]) == class {
            end += 1;
        }
    }
    Cursor {
        anchor: Position {
            line: position.line,
            column: start,
        },
        head: Position { line: position.line, column: end },
    }
}

/// What a triple-click selects: the whole line with its line break.
pub fn line_at(lines: Lines<'_>, line: usize) -> Cursor {
    let start = clamp(lines, Position { line, column: 0 });
    let end = if start.line + 1 < lines.count() {
        Position { line: start.line + 1, column: 0 }
    } else {
        Position {
            line: start.line,
            column: lines.line_chars(start.line),
        }
    };
    Cursor { anchor: start, head: end }
}

/// The text of `line` without its line break.
pub fn line_text_at(lines: Lines<'_>, line: usize) -> Cursor {
    let start = clamp(lines, Position { line, column: 0 });
    Cursor {
        anchor: start,
        head: Position {
            line: start.line,
            column: lines.line_chars(start.line),
        },
    }
}

/// The lines a selection touches, for marking them: the caret's line without a selection. A selection that ends at the start of a later line does not include that line. `None` when the text has no such line.
pub fn selected_lines(cursor: Cursor, line_count: usize) -> Option<Range<usize>> {
    let (start, end) = cursor.range();
    if start.line >= line_count {
        return None;
    }
    let last = if end.column == 0 && end.line > start.line { end.line - 1 } else { end.line };
    Some(start.line..last.min(line_count - 1) + 1)
}

fn word_left(lines: Lines<'_>, position: Position) -> Position {
    if position.column == 0 {
        return move_caret(lines, position, Motion::Left);
    }
    let chars: Vec<char> = lines.line(position.line).chars().collect();
    let mut column = position.column.min(chars.len());
    while column > 0 && char_class(chars[column - 1]) == CharClass::Space {
        column -= 1;
    }
    if column > 0 {
        let class = char_class(chars[column - 1]);
        while column > 0 && char_class(chars[column - 1]) == class {
            column -= 1;
        }
    }
    Position { line: position.line, column }
}

fn word_right(lines: Lines<'_>, position: Position) -> Position {
    let chars: Vec<char> = lines.line(position.line).chars().collect();
    if position.column >= chars.len() {
        return move_caret(lines, position, Motion::Right);
    }
    let mut column = position.column;
    let class = char_class(chars[column]);
    if class != CharClass::Space {
        while column < chars.len() && char_class(chars[column]) == class {
            column += 1;
        }
    }
    while column < chars.len() && char_class(chars[column]) == CharClass::Space {
        column += 1;
    }
    Position { line: position.line, column }
}

/// The position of byte `offset` of the text, clamped to the text.
pub fn position_of(lines: Lines<'_>, offset: usize) -> Position {
    if lines.count() == 0 {
        return Position::default();
    }
    let line = lines.ranges.partition_point(|range| range.end < offset).min(lines.count() - 1);
    let start = lines.ranges[line].start;
    let column = lines
        .text
        .get(start..offset.clamp(start, lines.ranges[line].end))
        .map(|text| text.chars().count())
        .unwrap_or(0);
    Position { line, column }
}

pub fn select_all(lines: Lines<'_>) -> Cursor {
    let end = move_caret(lines, Position::default(), Motion::DocumentEnd);
    Cursor {
        anchor: Position::default(),
        head: end,
    }
}

/// The selected text as it is in the document, line terminators included.
pub fn selected_text(lines: Lines<'_>, cursor: Cursor) -> String {
    let (start, end) = clamp_cursor(lines, cursor).range();
    lines.text.get(lines.offset(start)..lines.offset(end)).unwrap_or("").to_string()
}

pub fn move_caret(lines: Lines<'_>, position: Position, motion: Motion) -> Position {
    let position = clamp(lines, position);
    let last_line = lines.count().saturating_sub(1);
    let at = |line: usize, column: usize| clamp(lines, Position { line, column });
    match motion {
        Motion::Left if position.column > 0 => at(position.line, position.column - 1),
        Motion::Left if position.line > 0 => at(position.line - 1, lines.line_chars(position.line - 1)),
        Motion::Left => position,
        Motion::Right if position.column < lines.line_chars(position.line) => at(position.line, position.column + 1),
        Motion::Right if position.line < last_line => at(position.line + 1, 0),
        Motion::Right => position,
        Motion::WordLeft => word_left(lines, position),
        Motion::WordRight => word_right(lines, position),
        Motion::Up => at(position.line.saturating_sub(1), position.column),
        Motion::Down => at((position.line + 1).min(last_line), position.column),
        Motion::LineStart => at(position.line, 0),
        Motion::LineEnd => at(position.line, lines.line_chars(position.line)),
        Motion::PageUp(rows) => at(position.line.saturating_sub(rows), position.column),
        Motion::PageDown(rows) => at(position.line.saturating_add(rows).min(last_line), position.column),
        Motion::DocumentStart => Position::default(),
        Motion::DocumentEnd => at(last_line, lines.line_chars(last_line)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditKey {
    /// Typed or pasted text; any line ends become `\n`.
    Insert(String),
    Newline,
    Backspace,
    Delete,
    /// Ctrl+Backspace: deletes to the start of the word left of the caret.
    DeleteWordLeft,
    /// Ctrl+Delete: deletes to the start of the next word.
    DeleteWordRight,
}

/// Replaces a byte range of the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub text: String,
}

/// The edit a key makes at `cursor`, and where the caret goes. A selection is replaced by inserted text, or removed by Backspace and Delete. `None` when the key changes nothing.
pub fn edit_at(lines: Lines<'_>, cursor: Cursor, key: &EditKey) -> Option<(Edit, Position)> {
    let cursor = clamp_cursor(lines, cursor);
    if cursor.has_selection() {
        let (start, end) = cursor.range();
        let selected = lines.offset(start)..lines.offset(end);
        return match key {
            EditKey::Insert(text) => insert(start, selected, text),
            EditKey::Newline => insert(start, selected, "\n"),
            EditKey::Backspace | EditKey::Delete | EditKey::DeleteWordLeft | EditKey::DeleteWordRight => Some((
                Edit {
                    range: selected,
                    text: String::new(),
                },
                start,
            )),
        };
    }
    let position = cursor.head;
    let offset = lines.offset(position);
    match key {
        EditKey::Insert(text) => insert(position, offset..offset, text),
        EditKey::Newline => insert(position, offset..offset, "\n"),
        EditKey::Backspace if position.column > 0 => {
            let start = lines.offset(Position {
                line: position.line,
                column: position.column - 1,
            });
            Some((
                Edit {
                    range: start..offset,
                    text: String::new(),
                },
                Position {
                    line: position.line,
                    column: position.column - 1,
                },
            ))
        }
        EditKey::Backspace if position.line > 0 => {
            let previous = position.line - 1;
            let caret = Position {
                line: previous,
                column: lines.line_chars(previous),
            };
            Some((
                Edit {
                    range: offset - 1..offset,
                    text: String::new(),
                },
                caret,
            ))
        }
        EditKey::Backspace => None,
        EditKey::Delete if position.column < lines.line_chars(position.line) => {
            let end = lines.offset(Position {
                line: position.line,
                column: position.column + 1,
            });
            Some((
                Edit {
                    range: offset..end,
                    text: String::new(),
                },
                position,
            ))
        }
        EditKey::Delete if position.line + 1 < lines.count() => Some((
            Edit {
                range: offset..offset + 1,
                text: String::new(),
            },
            position,
        )),
        EditKey::Delete => None,
        EditKey::DeleteWordLeft => {
            let target = word_left(lines, position);
            (target != position).then(|| {
                (
                    Edit {
                        range: lines.offset(target)..offset,
                        text: String::new(),
                    },
                    target,
                )
            })
        }
        EditKey::DeleteWordRight => {
            let target = word_right(lines, position);
            (target != position).then(|| {
                (
                    Edit {
                        range: offset..lines.offset(target),
                        text: String::new(),
                    },
                    position,
                )
            })
        }
    }
}

/// The edit a key makes on the void row of a region without lines in D, with the caret at `position`, the start of the line after the row: typed text becomes the region's lines, ending with a line break, and the caret goes to the end of the typed text. `None` for keys that don't type, or when `position` doesn't start a line.
pub fn edit_on_void(lines: Lines<'_>, position: Position, key: &EditKey) -> Option<(Edit, Position)> {
    let typed = match key {
        EditKey::Insert(text) => text.as_str(),
        EditKey::Newline => "\n",
        _ => return None,
    };
    let position = clamp(lines, position);
    if position.column != 0 {
        return None;
    }
    let offset = lines.offset(position);
    let (mut edit, caret) = insert(position, offset..offset, typed)?;
    if !edit.text.ends_with('\n') {
        edit.text.push('\n');
    }
    Some((edit, caret))
}

/// Replaces `range`, which starts at `position`, with `text`.
fn insert(position: Position, range: Range<usize>, text: &str) -> Option<(Edit, Position)> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    if text.is_empty() {
        return None;
    }
    let caret = match text.rfind('\n') {
        Some(last_newline) => Position {
            line: position.line + text.matches('\n').count(),
            column: text[last_newline + 1..].chars().count(),
        },
        None => Position {
            line: position.line,
            column: position.column + text.chars().count(),
        },
    };
    Some((Edit { range, text }, caret))
}

/// Byte index of the `column`-th character, or the line's length past its end.
pub fn char_to_byte(line: &str, column: usize) -> usize {
    line.char_indices().nth(column).map(|(index, _)| index).unwrap_or(line.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line: usize, column: usize) -> Position {
        Position { line, column }
    }

    /// Applies `key` to `text` at `position` the way the merge pane does, returning the new text and caret.
    fn apply(text: &str, position: Position, key: EditKey) -> Option<(String, Position)> {
        let ranges = editor_lines(text);
        let lines = Lines { text, ranges: &ranges };
        let (edit, caret) = edit_at(lines, Cursor::at(position), &key)?;
        let mut result = text.to_string();
        result.replace_range(edit.range, &edit.text);
        Some((result, caret))
    }

    fn apply_on_void(text: &str, position: Position, key: EditKey) -> Option<(String, Position)> {
        let ranges = editor_lines(text);
        let (edit, caret) = edit_on_void(Lines { text, ranges: &ranges }, position, &key)?;
        let mut result = text.to_string();
        result.replace_range(edit.range, &edit.text);
        Some((result, caret))
    }

    #[test]
    fn typing_on_a_void_row_makes_whole_lines() {
        assert_eq!(
            apply_on_void("a\nb\n", at(1, 0), EditKey::Insert("x".to_string())),
            Some(("a\nx\nb\n".to_string(), at(1, 1)))
        );
        assert_eq!(
            apply_on_void("a\n", at(1, 0), EditKey::Insert("x\r\ny".to_string())),
            Some(("a\nx\ny\n".to_string(), at(2, 1)))
        );
        assert_eq!(apply_on_void("", at(0, 0), EditKey::Insert("x\n".to_string())), Some(("x\n".to_string(), at(1, 0))));
        assert_eq!(apply_on_void("a\n", at(1, 0), EditKey::Newline), Some(("a\n\n".to_string(), at(2, 0))));
        assert_eq!(apply_on_void("a\nb\n", at(1, 0), EditKey::Backspace), None);
        assert_eq!(apply_on_void("a\nb\n", at(1, 0), EditKey::Delete), None);
        assert_eq!(apply_on_void("a", at(0, 1), EditKey::Insert("x".to_string())), None, "not at a line start");
    }

    fn moved(text: &str, position: Position, motion: Motion) -> Position {
        let ranges = editor_lines(text);
        move_caret(Lines { text, ranges: &ranges }, position, motion)
    }

    fn selection(anchor: Position, head: Position) -> Cursor {
        Cursor { anchor, head }
    }

    fn apply_on_selection(text: &str, cursor: Cursor, key: EditKey) -> Option<(String, Position)> {
        let ranges = editor_lines(text);
        let lines = Lines { text, ranges: &ranges };
        let (edit, caret) = edit_at(lines, cursor, &key)?;
        let mut result = text.to_string();
        result.replace_range(edit.range, &edit.text);
        Some((result, caret))
    }

    #[test]
    fn tab_spaces_advance_to_the_next_stop_after_existing_tabs_and_unicode() {
        let text = "\téxy";
        let ranges = editor_lines(text);
        let lines = Lines { text, ranges: &ranges };
        let cursor = selection(at(0, 4), at(0, 2));
        let spaces = Indentation { spaces: true, size: 4 };
        let tabs = Indentation { spaces: false, size: 4 };
        assert_eq!(spaces.tab_text(lines, cursor), "   ");
        assert_eq!(spaces.tab_text(lines, Cursor::at(at(0, 1))), "    ");
        assert_eq!(tabs.tab_text(lines, cursor), "\t");
    }


    #[test]
    fn auto_indentation_detects_style_and_width_and_falls_back_when_ambiguous() {
        for (text, expected) in [
            ("root\n  child\n    leaf\n  child\n", (TabIndentation::Spaces, Some(2))),
            ("root\n\tchild\n\t\tleaf\n", (TabIndentation::Tabs, None)),
            ("root\n  child\n     leaf\n", (TabIndentation::Spaces, None)),
            ("root\n\tchild\n  child\n", (TabIndentation::Tabs, None)),
            ("root\n   \nplain\n", (TabIndentation::Tabs, None)),
        ] {
            let ranges = editor_lines(text);
            assert_eq!(detect_indentation(Lines { text, ranges: &ranges }), expected, "{text:?}");
        }
    }


    fn lines_of(text: &str) -> (String, Vec<Range<usize>>) {
        (text.to_string(), editor_lines(text))
    }

    #[test]
    fn terminators_are_read_from_the_text_after_each_line() {
        let text = "a\r\nb\nc\rd";
        let ranges = [0..1, 3..4, 5..6, 7..8];
        let lines = Lines { text, ranges: &ranges };
        assert_eq!(lines.terminator(0), "\r\n");
        assert_eq!(lines.terminator(1), "\n");
        assert_eq!(lines.terminator(2), "\r");
        assert_eq!(lines.terminator(3), "");
        assert_eq!(lines.terminator(9), "");
        let (text, ranges) = lines_of("x\n");
        let lines = Lines { text: &text, ranges: &ranges };
        assert_eq!((lines.terminator(0), lines.terminator(1)), ("\n", ""));
    }

    #[test]
    fn double_click_selects_word_space_run_or_punctuation() {
        let (text, ranges) = lines_of("let foo_bar = a+b;   x");
        let lines = Lines { text: &text, ranges: &ranges };
        assert_eq!(word_at(lines, at(0, 5)), selection(at(0, 4), at(0, 11)));
        assert_eq!(word_at(lines, at(0, 4)), selection(at(0, 4), at(0, 11)));
        assert_eq!(word_at(lines, at(0, 15)), selection(at(0, 15), at(0, 16)));
        assert_eq!(word_at(lines, at(0, 19)), selection(at(0, 18), at(0, 21)));
        assert_eq!(word_at(lines, at(0, 22)), selection(at(0, 21), at(0, 22)));
    }

    #[test]
    fn double_click_on_empty_line_selects_nothing() {
        let (text, ranges) = lines_of("a\n\nb");
        let lines = Lines { text: &text, ranges: &ranges };
        assert_eq!(word_at(lines, at(1, 0)), Cursor::at(at(1, 0)));
    }

    #[test]
    fn triple_click_selects_line_with_its_break() {
        let (text, ranges) = lines_of("ab\ncd");
        let lines = Lines { text: &text, ranges: &ranges };
        assert_eq!(line_at(lines, 0), selection(at(0, 0), at(1, 0)));
        assert_eq!(line_at(lines, 1), selection(at(1, 0), at(1, 2)));
        assert_eq!(line_at(lines, 9), selection(at(1, 0), at(1, 2)));
    }

    #[test]
    fn word_motions_jump_like_windows() {
        let text = "foo  bar.baz\nnext";
        assert_eq!(moved(text, at(0, 0), Motion::WordRight), at(0, 5));
        assert_eq!(moved(text, at(0, 5), Motion::WordRight), at(0, 8));
        assert_eq!(moved(text, at(0, 8), Motion::WordRight), at(0, 9));
        assert_eq!(moved(text, at(0, 12), Motion::WordRight), at(1, 0));
        assert_eq!(moved(text, at(0, 12), Motion::WordLeft), at(0, 9));
        assert_eq!(moved(text, at(0, 8), Motion::WordLeft), at(0, 5));
        assert_eq!(moved(text, at(0, 5), Motion::WordLeft), at(0, 0));
        assert_eq!(moved(text, at(1, 0), Motion::WordLeft), at(0, 12));
    }

    #[test]
    fn word_deletion() {
        assert_eq!(apply("foo bar", at(0, 7), EditKey::DeleteWordLeft), Some(("foo ".to_string(), at(0, 4))));
        assert_eq!(apply("foo bar", at(0, 4), EditKey::DeleteWordLeft), Some(("bar".to_string(), at(0, 0))));
        assert_eq!(apply("foo bar", at(0, 0), EditKey::DeleteWordRight), Some(("bar".to_string(), at(0, 0))));
        assert_eq!(apply("ab\ncd", at(1, 0), EditKey::DeleteWordLeft), Some(("abcd".to_string(), at(0, 2))));
        assert_eq!(apply("ab", at(0, 0), EditKey::DeleteWordLeft), None);
        assert_eq!(apply("ab", at(0, 2), EditKey::DeleteWordRight), None);
    }

    #[test]
    fn position_of_byte_offset() {
        let (text, ranges) = lines_of("ab\néc\n");
        let lines = Lines { text: &text, ranges: &ranges };
        assert_eq!(position_of(lines, 0), at(0, 0));
        assert_eq!(position_of(lines, 2), at(0, 2));
        assert_eq!(position_of(lines, 3), at(1, 0));
        assert_eq!(position_of(lines, 6), at(1, 2));
        assert_eq!(position_of(lines, 7), at(2, 0));
        assert_eq!(position_of(lines, 99), at(2, 0));
    }

    #[test]
    fn cursor_range_is_in_document_order() {
        let backwards = selection(at(2, 1), at(0, 3));
        assert_eq!(backwards.range(), (at(0, 3), at(2, 1)));
        assert!(backwards.has_selection());
        assert!(!Cursor::at(at(1, 1)).has_selection());
    }

    #[test]
    fn typing_replaces_selection() {
        let text = "hello\nworld";
        assert_eq!(
            apply_on_selection(text, selection(at(1, 2), at(0, 1)), EditKey::Insert("X".to_string())),
            Some(("hXrld".to_string(), at(0, 2)))
        );
        assert_eq!(
            apply_on_selection(text, selection(at(0, 1), at(0, 4)), EditKey::Newline),
            Some(("h\no\nworld".to_string(), at(1, 0)))
        );
    }

    #[test]
    fn backspace_and_delete_remove_selection() {
        let text = "héllo\nworld";
        assert_eq!(
            apply_on_selection(text, selection(at(0, 1), at(1, 1)), EditKey::Backspace),
            Some(("horld".to_string(), at(0, 1)))
        );
        assert_eq!(
            apply_on_selection(text, selection(at(1, 1), at(0, 1)), EditKey::Delete),
            Some(("horld".to_string(), at(0, 1)))
        );
    }

    #[test]
    fn shift_motion_extends_and_plain_motion_collapses() {
        let text = "abc\ndef";
        let ranges = editor_lines(text);
        let lines = Lines { text, ranges: &ranges };
        let extended = move_cursor(lines, Cursor::at(at(0, 1)), Motion::Down, true);
        assert_eq!(extended, selection(at(0, 1), at(1, 1)));
        let extended = move_cursor(lines, extended, Motion::LineEnd, true);
        assert_eq!(extended, selection(at(0, 1), at(1, 3)));
        assert_eq!(move_cursor(lines, extended, Motion::Left, false), Cursor::at(at(0, 1)));
        assert_eq!(move_cursor(lines, extended, Motion::Right, false), Cursor::at(at(1, 3)));
        assert_eq!(move_cursor(lines, extended, Motion::Up, false), Cursor::at(at(0, 3)));
    }

    #[test]
    fn select_all_and_selected_text_keep_original_terminators() {
        let text = "ab\r\ncd\r\n";
        let ranges = [0..2, 4..6];
        let lines = Lines { text, ranges: &ranges };
        let all = select_all(lines);
        assert_eq!(all, selection(at(0, 0), at(1, 2)));
        assert_eq!(selected_text(lines, all), "ab\r\ncd");
        assert_eq!(selected_text(lines, selection(at(1, 1), at(0, 1))), "b\r\nc");
        assert_eq!(selected_text(lines, Cursor::at(at(0, 1))), "");
    }

    #[test]
    fn update_editor_lines_handles_line_breaks() {
        let cases = [
            ("abc\ndef", 1..1, "X\nY"),
            ("abc\ndef", 2..5, ""),
            ("a\n\nb", 1..3, "\n\n\n"),
            ("", 0..0, "\n"),
            ("ab\n", 3..3, "c"),
            ("ab\n", 0..3, ""),
        ];
        for (before, range, inserted) in cases {
            let mut lines = editor_lines(before);
            let mut after = before.to_string();
            after.replace_range(range.clone(), inserted);
            update_editor_lines(&mut lines, &after, range.clone(), inserted.len());
            assert_eq!(lines, editor_lines(&after), "{before:?} {range:?} {inserted:?}");
        }
    }

    proptest::proptest! {
        #[test]
        fn update_editor_lines_matches_a_full_split(before in "[ab\n]{0,40}", start in 0usize..41, length in 0usize..41, inserted in "[xy\n]{0,10}") {
            let start = start.min(before.len());
            let end = (start + length).min(before.len());
            let mut lines = editor_lines(&before);
            let mut after = before.clone();
            after.replace_range(start..end, &inserted);
            update_editor_lines(&mut lines, &after, start..end, inserted.len());
            proptest::prop_assert_eq!(lines, editor_lines(&after));
        }
    }

    #[test]
    fn editor_lines_keep_a_final_empty_line() {
        assert_eq!(editor_lines(""), [0..0]);
        assert_eq!(editor_lines("a"), [0..1]);
        assert_eq!(editor_lines("a\n"), [0..1, 2..2]);
        assert_eq!(editor_lines("a\n\nbc"), [0..1, 2..2, 3..5]);
    }

    #[test]
    fn typing_inserts_at_caret() {
        assert_eq!(apply("ac", at(0, 1), EditKey::Insert("b".to_string())), Some(("abc".to_string(), at(0, 2))));
        assert_eq!(apply("", at(0, 0), EditKey::Insert("é".to_string())), Some(("é".to_string(), at(0, 1))));
        assert_eq!(apply("ab", at(0, 1), EditKey::Insert(String::new())), None);
    }

    #[test]
    fn pasting_multiple_lines_moves_caret_to_end_of_paste() {
        assert_eq!(apply("xy", at(0, 1), EditKey::Insert("1\r\n22\r3".to_string())), Some(("x1\n22\n3y".to_string(), at(2, 1))));
    }

    #[test]
    fn newline_splits_line() {
        assert_eq!(apply("ab", at(0, 1), EditKey::Newline), Some(("a\nb".to_string(), at(1, 0))));
        assert_eq!(apply("ab", at(0, 2), EditKey::Newline), Some(("ab\n".to_string(), at(1, 0))));
    }

    #[test]
    fn backspace_deletes_previous_character_or_joins_lines() {
        assert_eq!(apply("héllo", at(0, 2), EditKey::Backspace), Some(("hllo".to_string(), at(0, 1))));
        assert_eq!(apply("ab\ncd", at(1, 0), EditKey::Backspace), Some(("abcd".to_string(), at(0, 2))));
        assert_eq!(apply("ab", at(0, 0), EditKey::Backspace), None);
    }

    #[test]
    fn delete_removes_next_character_or_joins_lines() {
        assert_eq!(apply("héllo", at(0, 1), EditKey::Delete), Some(("hllo".to_string(), at(0, 1))));
        assert_eq!(apply("ab\ncd", at(0, 2), EditKey::Delete), Some(("abcd".to_string(), at(0, 2))));
        assert_eq!(apply("ab", at(0, 2), EditKey::Delete), None);
    }

    #[test]
    fn edits_clamp_an_out_of_range_caret() {
        assert_eq!(apply("ab\ncd", at(9, 9), EditKey::Insert("!".to_string())), Some(("ab\ncd!".to_string(), at(1, 3))));
    }

    #[test]
    fn left_and_right_wrap_across_lines() {
        let text = "ab\ncd";
        assert_eq!(moved(text, at(1, 0), Motion::Left), at(0, 2));
        assert_eq!(moved(text, at(0, 2), Motion::Right), at(1, 0));
        assert_eq!(moved(text, at(0, 0), Motion::Left), at(0, 0));
        assert_eq!(moved(text, at(1, 2), Motion::Right), at(1, 2));
        assert_eq!(moved("é", at(0, 0), Motion::Right), at(0, 1));
    }

    #[test]
    fn up_and_down_clamp_the_column() {
        let text = "long line\nab\nlonger line";
        assert_eq!(moved(text, at(0, 8), Motion::Down), at(1, 2));
        assert_eq!(moved(text, at(1, 2), Motion::Down), at(2, 2));
        assert_eq!(moved(text, at(0, 3), Motion::Up), at(0, 3));
        assert_eq!(moved(text, at(2, 3), Motion::Down), at(2, 3));
    }

    #[test]
    fn line_page_and_document_motions() {
        let text = "a\nbb\nccc\ndddd";
        assert_eq!(moved(text, at(2, 2), Motion::LineStart), at(2, 0));
        assert_eq!(moved(text, at(2, 0), Motion::LineEnd), at(2, 3));
        assert_eq!(moved(text, at(3, 4), Motion::PageUp(2)), at(1, 2));
        assert_eq!(moved(text, at(0, 1), Motion::PageDown(10)), at(3, 1));
        assert_eq!(moved(text, at(2, 2), Motion::DocumentStart), at(0, 0));
        assert_eq!(moved(text, at(0, 0), Motion::DocumentEnd), at(3, 4));
    }

    #[test]
    fn read_only_text_without_lines_keeps_caret_at_origin() {
        let lines = Lines { text: "", ranges: &[] };
        assert_eq!(move_caret(lines, at(3, 3), Motion::Down), at(0, 0));
        assert_eq!(clamp(lines, at(5, 5)), at(0, 0));
    }

    #[test]
    fn a_selection_marks_the_lines_it_touches() {
        let cursor = |anchor: Position, head: Position| Cursor { anchor, head };
        assert_eq!(selected_lines(Cursor::at(at(3, 2)), 10), Some(3..4));
        assert_eq!(selected_lines(cursor(at(3, 1), at(3, 4)), 10), Some(3..4));
        assert_eq!(selected_lines(cursor(at(3, 1), at(5, 2)), 10), Some(3..6));
        assert_eq!(selected_lines(cursor(at(5, 2), at(3, 1)), 10), Some(3..6), "backwards");
        assert_eq!(selected_lines(cursor(at(3, 1), at(5, 0)), 10), Some(3..5), "ending at a line start leaves that line out");
        assert_eq!(selected_lines(cursor(at(3, 0), at(4, 0)), 10), Some(3..4), "a triple-clicked line");
        assert_eq!(selected_lines(cursor(at(3, 0), at(3, 0)), 10), Some(3..4));
        assert_eq!(selected_lines(cursor(at(8, 0), at(30, 4)), 10), Some(8..10), "clamped to the text");
        assert_eq!(selected_lines(Cursor::at(at(10, 0)), 10), None);
        assert_eq!(selected_lines(Cursor::at(at(0, 0)), 0), None);
    }

    #[test]
    fn char_to_byte_handles_multibyte_and_overflow() {
        assert_eq!(char_to_byte("héllo", 2), 3);
        assert_eq!(char_to_byte("héllo", 99), 6);
        assert_eq!(char_to_byte("", 0), 0);
    }
}
