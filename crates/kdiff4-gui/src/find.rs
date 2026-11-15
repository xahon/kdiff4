use egui::text::{CCursor, CCursorRange};
use egui::{Align2, Area, Context, Frame, Id, Key, Order, Pos2, Rect, Response, Sense, TextEdit, Ui, Vec2, Role, WidgetInfo, vec2};
use kdiff4_protocol::{Document, OutputReplacement};
use regex::{Captures, Regex, RegexBuilder};
use thiserror::Error;

use crate::editing::{Cursor, Lines, Position};
use crate::icons::{self, Icon};

const FIELD_ID: &str = "find_field";
const REPLACE_FIELD_ID: &str = "find_replace_field";
const FIELD_WIDTH: f32 = 220.0;
const ICON_SIZE: f32 = 10.0;
/// Upper bound on the compiled size of a search expression, so a pathological one fails instead of eating memory.
const REGEX_SIZE_LIMIT: usize = 1 << 20;
pub const ALL_PANES_LABEL: &str = "All";
pub const REPLACE_LABEL: &str = "Replace";
pub const REPLACE_ALL_LABEL: &str = "Replace All";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub match_case: bool,
    pub whole_word: bool,
    pub regex: bool,
}

/// One occurrence of the search text in a line; the columns count characters, like caret positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

impl Match {
    pub fn cursor(&self) -> Cursor {
        Cursor {
            anchor: Position { line: self.line, column: self.start },
            head: Position { line: self.line, column: self.end },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FindError {
    #[error("{0}")]
    InvalidRegex(String),
}

fn build_regex(query: &str, options: FindOptions) -> Result<Regex, FindError> {
    let body = if options.regex { query.to_string() } else { regex::escape(query) };
    let pattern = if options.whole_word { format!(r"\b(?:{body})\b") } else { body };
    RegexBuilder::new(&pattern)
        .case_insensitive(!options.match_case)
        .size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|error| FindError::InvalidRegex(error.to_string().lines().last().unwrap_or_default().trim().to_string()))
}


/// Every non-empty occurrence of `query` in the lines of `lines`, in order. Matches never span lines. An empty query matches nothing.
pub fn find_all(lines: Lines<'_>, query: &str, options: FindOptions) -> Result<Vec<Match>, FindError> {
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let regex = build_regex(query, options)?;
    let mut matches = Vec::new();
    for line in 0..lines.count() {
        let text = lines.line(line);
        for found in regex.find_iter(text).filter(|found| !found.is_empty()) {
            matches.push(Match {
                line,
                start: text[..found.start()].chars().count(),
                end: text[..found.end()].chars().count(),
            });
        }
    }
    Ok(matches)
}

/// What a match is replaced with: `replacement` as typed, or, for a regular expression, with its `$1` and `${name}` groups filled in.
fn expanded(captures: &Captures<'_>, replacement: &str, options: FindOptions) -> String {
    if !options.regex {
        return replacement.to_string();
    }
    let mut text = String::new();
    captures.expand(replacement, &mut text);
    text
}


/// The replacement of every match `find_all` finds, in order, as byte ranges of the whole text.
pub fn replace_all(lines: Lines<'_>, query: &str, replacement: &str, options: FindOptions) -> Result<Vec<OutputReplacement>, FindError> {
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let regex = build_regex(query, options)?;
    let mut replacements = Vec::new();
    for (line, range) in lines.ranges.iter().enumerate() {
        for captures in regex.captures_iter(lines.line(line)) {
            let found = match captures.get(0) {
                Some(found) if !found.is_empty() => found,
                _ => continue,
            };
            replacements.push(OutputReplacement {
                range: range.start + found.start()..range.start + found.end(),
                text: expanded(&captures, replacement, options),
            });
        }
    }
    Ok(replacements)
}


/// The text the match `found` is replaced with, or `None` when the line no longer has that match.
pub fn replace_one(lines: Lines<'_>, found: Match, query: &str, replacement: &str, options: FindOptions) -> Result<Option<String>, FindError> {
    if query.is_empty() {
        return Ok(None);
    }
    let regex = build_regex(query, options)?;
    let text = lines.line(found.line);
    let byte_of = |column: usize| text.char_indices().nth(column).map_or(text.len(), |(index, _)| index);
    let (start, end) = (byte_of(found.start), byte_of(found.end));
    let replaced = regex
        .captures_at(text, start)
        .filter(|captures| captures.get(0).is_some_and(|whole| whole.start() == start && whole.end() == end && !whole.is_empty()))
        .map(|captures| expanded(&captures, replacement, options));
    Ok(replaced)
}


/// The first match starting at or after `from`, wrapping to the first one.
pub fn first_from(matches: &[Match], from: Position) -> Option<usize> {
    let index = matches.partition_point(|found| (found.line, found.start) < (from.line, from.column));
    match index {
        _ if matches.is_empty() => None,
        index if index == matches.len() => Some(0),
        index => Some(index),
    }
}

/// What a search was run over, so its matches are only found again when something changed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Searched {
    document: Document,
    query: String,
    options: FindOptions,
    text_address: usize,
    text_len: usize,
    line_count: usize,
}

/// The matches of a pane other than the searched one, when every pane is searched.
#[derive(Debug, Clone)]
pub struct PaneMatches {
    pub document: Document,
    pub matches: Vec<Match>,
    searched: Searched,
}

/// The find bar and the search it runs: in the pane that has the caret, or in D while the replace row is shown.
#[derive(Debug, Clone, Default)]
pub struct FindState {
    pub open: bool,
    pub query: String,
    pub options: FindOptions,
    /// Search every pane: Next and Previous go on into the next pane that has a match. The replace row turns it off.
    pub all_panes: bool,
    /// The replace row is shown.
    pub replace_open: bool,
    pub replacement: String,
    /// Give the field the keyboard on the next frame, with its text selected.
    pub focus: bool,
    /// The text of the searched document changed.
    pub stale: bool,
    pub document: Option<Document>,
    pub matches: Vec<Match>,
    pub error: Option<FindError>,
    pub current: Option<usize>,
    searched: Option<Searched>,
    /// The other searched panes, in pane order.
    pub others: Vec<PaneMatches>,
    /// How many of `others` come before the searched pane.
    others_before: usize,
    /// The height the bar had when it was last shown.
    pub height: f32,
}

impl FindState {
    /// Whether every pane is searched now.
    pub fn searches_all_panes(&self) -> bool {
        self.all_panes && !self.replace_open
    }

    /// Finds the matches again where the query, the options or the text changed: in `document`, the searched pane, and in the other `panes`, which are in pane order. Returns whether any were found again.
    pub fn refresh(&mut self, document: Document, panes: &[(Document, Lines<'_>)]) -> bool {
        let stale = std::mem::take(&mut self.stale);
        let mut changed = false;
        let mut others = Vec::new();
        let mut others_before = 0;
        for (pane, lines) in panes {
            let searched = Searched {
                document: pane.clone(),
                query: self.query.clone(),
                options: self.options,
                text_address: lines.text.as_ptr() as usize,
                text_len: lines.text.len(),
                line_count: lines.count(),
            };
            if *pane == document {
                others_before = others.len();
                if stale || self.searched.as_ref() != Some(&searched) {
                    changed = true;
                    self.document = Some(document);
                    (self.matches, self.error) = match find_all(*lines, &self.query, self.options) {
                        Ok(matches) => (matches, None),
                        Err(error) => (Vec::new(), Some(error)),
                    };
                    self.searched = Some(searched);
                }
                continue;
            }
            let kept = self.others
                .iter_mut()
                .find(|other| !stale && other.searched == searched)
                .map(|other| std::mem::take(&mut other.matches));
            let matches = match kept {
                Some(matches) => matches,
                None => {
                    changed = true;
                    find_all(*lines, &self.query, self.options).unwrap_or_default()
                }
            };
            others.push(PaneMatches { document: pane.clone(), matches, searched });
        }
        changed |= others.len() != self.others.len();
        self.others = others;
        self.others_before = others_before;
        changed
    }

    /// Where typing in the bar goes: the first match at or after `from` in the searched pane, wrapping to its first one, or the first match of the next pane that has one.
    pub fn first(&self, from: Position) -> Option<(Document, Match)> {
        let document = self.document?;
        match first_from(&self.matches, from).and_then(|index| self.matches.get(index)) {
            Some(found) => Some((document, found.clone())),
            None => self.elsewhere(true),
        }
    }

    /// Where Next or Previous goes from a selection (or a caret) in the searched pane: to the first match starting at or after its end, or the last starting before its start; past the pane's matches, into the next or previous pane that has one; and around to the searched pane's first or last.
    pub fn step(&self, cursor: Cursor, forward: bool) -> Option<(Document, Match)> {
        let document = self.document?;
        let from = if forward { cursor.anchor.max(cursor.head) } else { cursor.anchor.min(cursor.head) };
        let index = self.matches.partition_point(|found| (found.line, found.start) < (from.line, from.column));
        let here = if forward { self.matches.get(index) } else { index.checked_sub(1).and_then(|index| self.matches.get(index)) };
        let around = if forward { self.matches.first() } else { self.matches.last() };
        here.map(|found| (document, found.clone()))
            .or_else(|| self.elsewhere(forward))
            .or_else(|| around.map(|found| (document, found.clone())))
    }

    /// The first match of the next pane that has one, or the last match of the previous one.
    fn elsewhere(&self, forward: bool) -> Option<(Document, Match)> {
        let (before, after) = self.others.split_at(self.others_before.min(self.others.len()));
        if forward {
            after
                .iter()
                .chain(before)
                .find_map(|pane| pane.matches.first().map(|found| (pane.document, found.clone())))
        } else {
            before
                .iter()
                .rev()
                .chain(after.iter().rev())
                .find_map(|pane| pane.matches.last().map(|found| (pane.document, found.clone())))
        }
    }

    /// Follows the selection: the current match is the one the selection covers.
    pub fn follow(&mut self, cursor: Option<Cursor>) {
        self.current = cursor.and_then(|cursor| {
            let (start, end) = (cursor.anchor.min(cursor.head), cursor.anchor.max(cursor.head));
            self.matches
                .iter()
                .position(|found| start == (Position { line: found.line, column: found.start }) && end == (Position { line: found.line, column: found.end }))
        });
    }

    pub fn clear(&mut self) {
        self.matches.clear();
        self.others.clear();
        self.current = None;
        self.error = None;
        self.searched = None;
    }

    /// The place of the current match among the matches of all searched panes, which are counted in pane order.
    pub fn summary(&self) -> String {
        let count = |panes: &[PaneMatches]| panes.iter().map(|pane| pane.matches.len()).sum::<usize>();
        let before = count(&self.others[..self.others_before.min(self.others.len())]);
        match (&self.error, self.matches.len() + count(&self.others), self.current) {
            (Some(error), _, _) => error.to_string(),
            (None, _, _) if self.query.is_empty() => String::new(),
            (None, 0, _) => "No results".to_string(),
            (None, total, Some(current)) => format!("{} of {total}", before + current + 1),
            (None, total, None) => format!("? of {total}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindEvent {
    /// The query or an option changed.
    Changed,
    Next,
    Previous,
    /// Replace the current match and go to the next one.
    Replace,
    ReplaceAll,
    Close,
}

/// The find bar, floating at the top right of `top_right`: a field, the option toggles, the result count, previous and next, and close; and under them, when it is shown, the replace row: a field, Replace and Replace All.
pub fn show_bar(ctx: &Context, tab: usize, top_right: Pos2, state: &mut FindState) -> Vec<FindEvent> {
    let mut events = Vec::new();
    let field_id = Id::unique((FIELD_ID, tab));
    let replace_field_id = Id::unique((REPLACE_FIELD_ID, tab));
    let (pressed_enter, pressed_escape, pressed_tab, modifiers) = ctx.input(|input| (input.key_pressed(Key::Enter), input.key_pressed(Key::Escape), input.key_pressed(Key::Tab), input.modifiers));
    let replaces_all = modifiers.command && modifiers.alt;
    let mut tabbed_to = None;
    let area = Area::new(Id::unique(("find_bar", tab)))
        .order(Order::Foreground)
        .fixed_pos(top_right)
        .pivot(Align2::RIGHT_TOP)
        .show(ctx, |ui| {
            Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let field = ui.add(TextEdit::singleline(&mut state.query).id(field_id).hint_text("Find").desired_width(FIELD_WIDTH));
                    if state.focus {
                        state.focus = false;
                        field.request_focus();
                        let mut edit = TextEdit::load_state(ctx, field_id).unwrap_or_default();
                        edit.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(state.query.chars().count()))));
                        edit.store(ctx, field_id);
                    }
                    if field.changed() {
                        events.push(FindEvent::Changed);
                    }
                    if state.replace_open && replaces_all && pressed_enter && (field.has_focus() || field.lost_focus()) {
                        events.push(FindEvent::ReplaceAll);
                        field.request_focus();
                    } else if field.lost_focus() && pressed_enter {
                        events.push(if modifiers.shift { FindEvent::Previous } else { FindEvent::Next });
                        field.request_focus();
                    }
                    if (field.has_focus() || field.lost_focus()) && pressed_escape {
                        events.push(FindEvent::Close);
                    }
                    // Tab goes straight to the replacement text, not through the toggles and buttons between the two fields.
                    if state.replace_open && (field.has_focus() || field.lost_focus()) && pressed_tab && !modifiers.shift {
                        tabbed_to = Some(replace_field_id);
                    }
                    for (enabled, label, hint) in [
                        (&mut state.options.match_case, "Aa", "Match Case"),
                        (&mut state.options.whole_word, "ab", "Match Whole Word"),
                        (&mut state.options.regex, ".*", "Use Regular Expression"),
                    ] {
                        if ui.selectable_label(*enabled, label).on_hover_text(hint).clicked() {
                            *enabled = !*enabled;
                            events.push(FindEvent::Changed);
                        }
                    }
                    let all_panes = ui
                        .add_enabled_ui(!state.replace_open, |ui| ui.selectable_label(state.searches_all_panes(), ALL_PANES_LABEL))
                        .inner
                        .on_hover_text("Search All Panes")
                        .on_disabled_hover_text("Replacing searches the output only");
                    if all_panes.clicked() {
                        state.all_panes = !state.all_panes;
                        events.push(FindEvent::Changed);
                    }
                    let summary = state.summary();
                    let color = if state.error.is_some() { ui.visuals().error_fg_color } else { ui.visuals().text_color() };
                    ui.add_sized(vec2(90.0, ui.spacing().interact_size.y), egui::Label::new(egui::RichText::new(summary).color(color)).truncate());
                    if icon_button(ui, "Previous Match", "Previous Match (Shift+F3)", icons::PREVIOUS_MATCH).clicked() {
                        events.push(FindEvent::Previous);
                    }
                    if icon_button(ui, "Next Match", "Next Match (F3)", icons::NEXT_MATCH).clicked() {
                        events.push(FindEvent::Next);
                    }
                    if icon_button(ui, "Close", "Close (Escape)", icons::CLOSE).clicked() {
                        events.push(FindEvent::Close);
                    }
                });
                if state.replace_open {
                    ui.horizontal(|ui| {
                        let field = ui.add(TextEdit::singleline(&mut state.replacement).id(replace_field_id).hint_text("Replace").desired_width(FIELD_WIDTH));
                        // egui moves the focus to the widget before this one at the start of the next frame; the find field takes it back then.
                        if (field.has_focus() || field.lost_focus()) && pressed_tab && modifiers.shift {
                            state.focus = true;
                        }
                        if replaces_all && pressed_enter && (field.has_focus() || field.lost_focus()) {
                            events.push(FindEvent::ReplaceAll);
                            field.request_focus();
                        } else if field.lost_focus() && pressed_enter {
                            events.push(FindEvent::Replace);
                            field.request_focus();
                        }
                        if (field.has_focus() || field.lost_focus()) && pressed_escape {
                            events.push(FindEvent::Close);
                        }
                        if ui.button(REPLACE_LABEL).on_hover_text("Replace the selected match and go to the next (Enter)").clicked() {
                            events.push(FindEvent::Replace);
                        }
                        if ui.button(REPLACE_ALL_LABEL).on_hover_text("Replace every match in the output (Ctrl+Alt+Enter)").clicked() {
                            events.push(FindEvent::ReplaceAll);
                        }
                    });
                }
            });
        });
    // After every widget of the bar, so that it is not egui's own move to the next widget that wins.
    if let Some(id) = tabbed_to {
        ctx.memory_mut(|memory| memory.request_focus(id));
    }
    state.height = area.response.rect.height();
    events
}


/// A square button with a centered icon, named `label` for screen readers.
fn icon_button(ui: &mut Ui, label: &str, hint: &str, image: Icon) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(ui.spacing().interact_size.y), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), label));
    if response.hovered() {
        ui.painter().rect_filled(rect, ui.visuals().widgets.hovered.corner_radius, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let icon = Rect::from_center_size(rect.center(), Vec2::splat(ICON_SIZE));
    image.image().tint(ui.visuals().text_color()).paint_at(ui, icon);
    response.on_hover_text(hint)
}


#[cfg(test)]
mod tests {
    use std::ops::Range;

    use egui::Pos2;
    use egui_kittest::kittest::{NodeT, Queryable};

    use super::*;

    fn ranges(text: &str) -> Vec<Range<usize>> {
        let mut start = 0;
        let mut ranges = Vec::new();
        for line in text.split_inclusive('\n') {
            ranges.push(start..start + line.trim_end_matches(['\r', '\n']).len());
            start += line.len();
        }
        ranges
    }

    fn found(text: &str, query: &str, options: FindOptions) -> Vec<(usize, usize, usize)> {
        let ranges = ranges(text);
        let lines = Lines { text, ranges: &ranges };
        find_all(lines, query, options)
            .expect("query is valid")
            .into_iter()
            .map(|found| (found.line, found.start, found.end))
            .collect()
    }

    fn at(line: usize, column: usize) -> Position {
        Position { line, column }
    }

    fn three() -> Vec<Match> {
        vec![Match { line: 0, start: 1, end: 3 }, Match { line: 0, start: 5, end: 7 }, Match { line: 2, start: 0, end: 2 }]
    }

    #[test]
    fn finds_every_occurrence_in_every_line() {
        assert_eq!(found("abab\nxab\n", "ab", FindOptions::default()), vec![(0, 0, 2), (0, 2, 4), (1, 1, 3)]);
    }

    #[test]
    fn an_empty_query_matches_nothing() {
        assert!(found("abc\n", "", FindOptions::default()).is_empty());
    }

    #[test]
    fn ignores_case_unless_asked_to_match_it() {
        assert_eq!(found("Ab aB\n", "ab", FindOptions::default()), vec![(0, 0, 2), (0, 3, 5)]);
        assert_eq!(found("Ab aB\n", "ab", FindOptions { match_case: true, ..Default::default() }), vec![]);
    }

    #[test]
    fn plain_text_is_not_a_pattern() {
        assert_eq!(found("a.c abc\n", "a.c", FindOptions::default()), vec![(0, 0, 3)]);
    }

    #[test]
    fn whole_word_skips_matches_inside_words() {
        let options = FindOptions { whole_word: true, ..Default::default() };
        assert_eq!(found("cat concat cat_x cat\n", "cat", options), vec![(0, 0, 3), (0, 17, 20)]);
    }

    #[test]
    fn regex_matches_patterns_and_skips_empty_matches() {
        let options = FindOptions { regex: true, ..Default::default() };
        assert_eq!(found("a1 b22\n", r"\d+", options), vec![(0, 1, 2), (0, 4, 6)]);
        assert_eq!(found("bc\n", "a*", options), vec![]);
    }

    #[test]
    fn an_invalid_regex_is_an_error_with_a_one_line_message() {
        let ranges = ranges("a\n");
        let lines = Lines { text: "a\n", ranges: &ranges };
        let error = find_all(lines, "(", FindOptions { regex: true, ..Default::default() }).expect_err("unbalanced group");
        let FindError::InvalidRegex(message) = error;
        assert!(!message.is_empty() && !message.contains('\n'), "{message}");
    }

    #[test]
    fn columns_count_characters_not_bytes() {
        assert_eq!(found("héllo wörld\n", "wör", FindOptions::default()), vec![(0, 6, 9)]);
    }

    #[test]
    fn matches_stop_at_the_line_end() {
        assert_eq!(found("ab\r\nab", "b\r", FindOptions::default()), vec![]);
        assert_eq!(found("ab\r\nab", "ab", FindOptions::default()), vec![(0, 0, 2), (1, 0, 2)]);
    }

    /// A search of B, with the matches of `three` there.
    fn searching_b() -> FindState {
        FindState { document: Some(Document::B), matches: three(), ..Default::default() }
    }

    fn next(state: &FindState, cursor: Cursor) -> Option<(Document, Match)> {
        state.step(cursor, true)
    }

    fn previous(state: &FindState, cursor: Cursor) -> Option<(Document, Match)> {
        state.step(cursor, false)
    }

    fn in_b(index: usize) -> Option<(Document, Match)> {
        Some((Document::B, three()[index]))
    }

    /// A search of every pane with the caret in B: A has one match, C none, D two.
    fn searching_all() -> FindState {
        let texts = [(Document::A, "ab\n"), (Document::B, "xab  ab\n\nab\n"), (Document::C, "\n"), (Document::D, "ab\nab\n")];
        let ranges: Vec<Vec<Range<usize>>> = texts.iter().map(|(_, text)| ranges(text)).collect();
        let panes: Vec<(Document, Lines<'_>)> = texts.iter().zip(&ranges).map(|((document, text), ranges)| (*document, Lines { text, ranges })).collect();
        let mut state = FindState { query: "ab".to_string(), all_panes: true, ..Default::default() };
        assert!(state.refresh(Document::B, &panes));
        assert_eq!(state.matches, three());
        state
    }

    #[test]
    fn next_goes_to_the_first_match_after_the_caret_or_selection() {
        let state = searching_b();
        assert_eq!(next(&state, Cursor::at(at(0, 0))), in_b(0));
        assert_eq!(next(&state, Cursor::at(at(0, 1))), in_b(0));
        assert_eq!(next(&state, Cursor::at(at(0, 2))), in_b(1));
        assert_eq!(next(&state, three()[0].cursor()), in_b(1));
        assert_eq!(next(&state, three()[1].cursor()), in_b(2));
    }

    #[test]
    fn next_wraps_to_the_first_match() {
        assert_eq!(next(&searching_b(), three()[2].cursor()), in_b(0));
        assert_eq!(next(&searching_b(), Cursor::at(at(5, 0))), in_b(0));
    }

    #[test]
    fn previous_goes_to_the_last_match_before_the_caret_or_selection() {
        let state = searching_b();
        assert_eq!(previous(&state, Cursor::at(at(3, 0))), in_b(2));
        assert_eq!(previous(&state, Cursor::at(at(2, 0))), in_b(1));
        assert_eq!(previous(&state, three()[2].cursor()), in_b(1));
        assert_eq!(previous(&state, three()[1].cursor()), in_b(0));
    }

    #[test]
    fn previous_wraps_to_the_last_match() {
        assert_eq!(previous(&searching_b(), three()[0].cursor()), in_b(2));
        assert_eq!(previous(&searching_b(), Cursor::at(at(0, 0))), in_b(2));
    }

    #[test]
    fn navigation_without_matches_goes_nowhere() {
        let state = FindState { document: Some(Document::A), ..Default::default() };
        assert_eq!(next(&state, Cursor::default()), None);
        assert_eq!(previous(&state, Cursor::default()), None);
        assert_eq!(state.first(at(0, 0)), None);
        assert_eq!(first_from(&[], at(0, 0)), None);
        assert_eq!(next(&FindState { matches: three(), ..Default::default() }, Cursor::default()), None, "no searched pane");
    }

    #[test]
    fn past_the_matches_of_a_pane_next_and_previous_go_into_the_panes_that_have_one() {
        let state = searching_all();
        assert_eq!(next(&state, three()[1].cursor()), in_b(2));
        assert_eq!(next(&state, three()[2].cursor()), Some((Document::D, Match { line: 0, start: 0, end: 2 })), "C has no match");
        assert_eq!(previous(&state, three()[1].cursor()), in_b(0));
        assert_eq!(previous(&state, three()[0].cursor()), Some((Document::A, Match { line: 0, start: 0, end: 2 })));
        let mut from_d = state.clone();
        from_d.stale = true;
        let text = "ab\nab\n";
        let ranges = ranges(text);
        from_d.refresh(Document::D, &[(Document::A, Lines { text: "ab\n", ranges: &ranges[..1] }), (Document::D, Lines { text, ranges: &ranges })]);
        assert_eq!(next(&from_d, Cursor::at(at(1, 2))), Some((Document::A, Match { line: 0, start: 0, end: 2 })), "after the last pane comes the first");
        assert_eq!(previous(&from_d, Cursor::at(at(0, 0))), Some((Document::A, Match { line: 0, start: 0, end: 2 })));
    }

    #[test]
    fn typing_stays_in_the_pane_while_it_has_a_match() {
        let mut state = searching_all();
        assert_eq!(state.first(at(2, 1)), in_b(0), "wraps before leaving the pane");
        state.matches.clear();
        assert_eq!(state.first(at(0, 0)), Some((Document::D, Match { line: 0, start: 0, end: 2 })));
    }

    #[test]
    fn the_summary_counts_the_matches_of_all_searched_panes_in_pane_order() {
        let mut state = searching_all();
        assert_eq!(state.summary(), "? of 6");
        state.current = Some(1);
        assert_eq!(state.summary(), "3 of 6");
        state.replace_open = true;
        assert!(!state.searches_all_panes());
    }

    #[test]
    fn replace_all_replaces_every_match_by_its_byte_range() {
        let text = "héllo ab\r\nAB ab\n";
        let ranges = ranges(text);
        let lines = Lines { text, ranges: &ranges };
        let replaced = replace_all(lines, "ab", "$0x", FindOptions::default()).unwrap();
        let expected: Vec<OutputReplacement> = [7..9, 11..13, 14..16].into_iter().map(|range| OutputReplacement { range, text: "$0x".to_string() }).collect();
        assert_eq!(replaced, expected);
        assert_eq!(replace_all(lines, "", "x", FindOptions::default()).unwrap(), []);
        assert!(replace_all(lines, "(", "x", FindOptions { regex: true, ..Default::default() }).is_err());
    }

    #[test]
    fn a_regular_expression_fills_its_groups_into_the_replacement() {
        let text = "let a = 1;\nlet bb = 22;\n";
        let ranges = ranges(text);
        let lines = Lines { text, ranges: &ranges };
        let options = FindOptions { regex: true, ..Default::default() };
        let replaced = replace_all(lines, r"(?<name>\w+) = (\d+)", "${2} = $name $$", options).unwrap();
        assert_eq!(replaced, [OutputReplacement { range: 4..9, text: "1 = a $".to_string() }, OutputReplacement { range: 15..22, text: "22 = bb $".to_string() }]);
        assert_eq!(replace_all(lines, r"\d*", "n", options).unwrap().len(), 2, "empty matches are skipped");
    }

    #[test]
    fn replace_one_replaces_only_a_match_that_is_still_there() {
        let text = "é ab cab\n";
        let ranges = ranges(text);
        let lines = Lines { text, ranges: &ranges };
        let word = FindOptions { whole_word: true, ..Default::default() };
        assert_eq!(replace_one(lines, Match { line: 0, start: 2, end: 4 }, "ab", "$1", word), Ok(Some("$1".to_string())));
        assert_eq!(replace_one(lines, Match { line: 0, start: 6, end: 8 }, "ab", "x", word), Ok(None), "inside a word");
        assert_eq!(replace_one(lines, Match { line: 0, start: 2, end: 3 }, "ab", "x", FindOptions::default()), Ok(None));
        assert_eq!(replace_one(lines, Match { line: 9, start: 0, end: 2 }, "ab", "x", FindOptions::default()), Ok(None));
        assert_eq!(replace_one(lines, Match { line: 0, start: 2, end: 4 }, "", "x", FindOptions::default()), Ok(None));
        let regex = FindOptions { regex: true, ..Default::default() };
        assert_eq!(replace_one(lines, Match { line: 0, start: 5, end: 8 }, "c(a)(b)", "$2$1", regex), Ok(Some("ba".to_string())));
    }

    #[test]
    fn the_selection_decides_the_current_match() {
        let mut state = FindState { matches: three(), ..Default::default() };
        state.follow(Some(three()[1].cursor()));
        assert_eq!(state.current, Some(1));
        state.follow(Some(Cursor::at(at(0, 5))));
        assert_eq!(state.current, None);
        state.follow(None);
        assert_eq!(state.current, None);
    }

    #[test]
    fn refresh_searches_again_only_when_something_changed() {
        let text = "ab ab\n";
        let ranges = ranges(text);
        let lines = Lines { text, ranges: &ranges };
        let mut state = FindState { query: "ab".to_string(), ..Default::default() };
        assert!(state.refresh(Document::A, &[(Document::A, lines)]));
        assert_eq!(state.matches.len(), 2);
        assert!(!state.refresh(Document::A, &[(Document::A, lines)]));
        assert!(state.refresh(Document::B, &[(Document::B, lines)]), "another pane is another search");
        state.query = "b".to_string();
        assert!(state.refresh(Document::B, &[(Document::B, lines)]));
        state.stale = true;
        assert!(state.refresh(Document::B, &[(Document::B, lines)]), "an edit makes the matches stale");
        let both = [(Document::A, lines), (Document::B, lines)];
        assert!(state.refresh(Document::B, &both), "another pane joined the search");
        assert_eq!(state.others.iter().map(|pane| (pane.document, pane.matches.len())).collect::<Vec<_>>(), [(Document::A, 2)]);
        assert!(!state.refresh(Document::B, &both));
        assert_eq!(state.others[0].matches.len(), 2, "kept matches stay");
        assert!(state.refresh(Document::B, &[(Document::B, lines)]), "the pane left the search");
        assert!(state.others.is_empty());
    }

    #[test]
    fn the_summary_tells_where_the_current_match_is() {
        let mut state = FindState { query: "x".to_string(), matches: three(), ..Default::default() };
        assert_eq!(state.summary(), "? of 3");
        state.current = Some(1);
        assert_eq!(state.summary(), "2 of 3");
        state.matches.clear();
        assert_eq!(state.summary(), "No results");
        state.query.clear();
        assert_eq!(state.summary(), "");
        state.error = Some(FindError::InvalidRegex("bad".to_string()));
        assert_eq!(state.summary(), "bad");
    }


    struct BarTest {
        state: FindState,
        events: Vec<FindEvent>,
    }

    fn bar() -> egui_kittest::Harness<'static, BarTest> {
        let test = BarTest {
            state: FindState { open: true, query: "ab".to_string(), matches: three(), ..Default::default() },
            events: Vec::new(),
        };
        let mut harness = egui_kittest::Harness::builder()
            .with_size(vec2(600.0, 200.0))
            .build_ui_state(|ui, test: &mut BarTest| {
                egui_extras::install_image_loaders(ui.ctx());
                test.events.extend(show_bar(ui.ctx(), 0, Pos2::new(580.0, 10.0), &mut test.state));
            }, test);
        harness.run_steps(3);
        harness
    }

    #[test]
    fn the_bar_buttons_step_and_close() {
        let mut harness = bar();
        harness.get_by_label("Next Match").click();
        harness.get_by_label("Previous Match").click();
        harness.get_by_label("Close").click();
        harness.run_steps(2);
        assert_eq!(harness.state().events, [FindEvent::Next, FindEvent::Previous, FindEvent::Close]);
    }

    #[test]
    fn the_bar_toggles_change_the_options() {
        let mut harness = bar();
        harness.get_by_label("Aa").click();
        harness.get_by_label("ab").click();
        harness.get_by_label(".*").click();
        harness.run_steps(2);
        assert_eq!(harness.state().state.options, FindOptions { match_case: true, whole_word: true, regex: true });
        assert_eq!(harness.state().events, [FindEvent::Changed; 3]);
    }

    #[test]
    fn the_all_panes_toggle_is_off_while_replacing() {
        let mut harness = bar();
        harness.get_by_label(ALL_PANES_LABEL).click();
        harness.run_steps(2);
        assert!(harness.state().state.all_panes && harness.state().state.searches_all_panes());
        assert_eq!(harness.state().events, [FindEvent::Changed]);
        assert!(harness.query_by_label(REPLACE_ALL_LABEL).is_none());
        harness.state_mut().state.replace_open = true;
        harness.run_steps(2);
        assert!(harness.get_by_label(ALL_PANES_LABEL).accesskit_node().is_disabled());
    }

    #[test]
    fn the_replace_row_replaces_one_or_all() {
        let mut harness = bar();
        harness.state_mut().state.replace_open = true;
        harness.run_steps(2);
        let one_row = harness.state().state.height;
        harness.get_by_label(REPLACE_LABEL).click();
        harness.get_by_label(REPLACE_ALL_LABEL).click();
        harness.run_steps(2);
        assert_eq!(harness.state().events, [FindEvent::Replace, FindEvent::ReplaceAll]);
        harness.state_mut().state.replace_open = false;
        harness.run_steps(2);
        assert!(harness.state().state.height > 0.0 && harness.state().state.height < one_row);
    }

    #[test]
    fn enter_in_the_replace_field_replaces_and_with_ctrl_alt_replaces_all() {
        let mut harness = bar();
        harness.state_mut().state.replace_open = true;
        harness.run_steps(2);
        harness.get_all_by_role(egui::Role::TextInput).nth(1).expect("the replace field").focus();
        harness.run_steps(2);
        harness.key_press(Key::Enter);
        harness.run_steps(2);
        harness.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::ALT, Key::Enter);
        harness.run_steps(2);
        harness.key_press(Key::Escape);
        harness.run_steps(2);
        assert_eq!(harness.state().events, [FindEvent::Replace, FindEvent::ReplaceAll, FindEvent::Close]);
    }

    #[test]
    fn tab_goes_between_the_find_and_the_replacement_text() {
        let mut harness = bar();
        harness.state_mut().state.replace_open = true;
        harness.state_mut().state.focus = true;
        harness.run_steps(3);
        let focused = |harness: &egui_kittest::Harness<'static, BarTest>| {
            harness
                .get_all_by_role(egui::Role::TextInput)
                .map(|field| field.is_focused())
                .collect::<Vec<_>>()
        };
        assert_eq!(focused(&harness), [true, false]);
        harness.key_press(Key::Tab);
        harness.run_steps(3);
        assert_eq!(focused(&harness), [false, true]);
        harness.key_press_modifiers(egui::Modifiers::SHIFT, Key::Tab);
        harness.run_steps(3);
        assert_eq!(focused(&harness), [true, false]);
    }

    #[test]
    fn the_bar_shows_the_result_count() {
        let harness = bar();
        harness.get_by_label("? of 3");
    }
}
