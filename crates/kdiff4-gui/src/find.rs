use egui::text::{CCursor, CCursorRange};
use egui::{Align2, Area, Context, Frame, Id, Key, Order, Painter, Pos2, Response, Sense, Stroke, TextEdit, Ui, Vec2, Role, WidgetInfo, vec2};
use kdiff4_protocol::Document;
use regex::RegexBuilder;
use thiserror::Error;

use crate::editing::{Cursor, Lines, Position};

const FIELD_ID: &str = "find_field";
const FIELD_WIDTH: f32 = 220.0;
/// Half the size of the icons of the bar's buttons.
const ICON_ARM: f32 = 4.0;
/// Upper bound on the compiled size of a search expression, so a pathological one fails instead of eating memory.
const REGEX_SIZE_LIMIT: usize = 1 << 20;

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

/// Every non-empty occurrence of `query` in the lines of `lines`, in order. Matches never span lines. An empty query matches nothing.
pub fn find_all(lines: Lines<'_>, query: &str, options: FindOptions) -> Result<Vec<Match>, FindError> {
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let body = if options.regex { query.to_string() } else { regex::escape(query) };
    let pattern = if options.whole_word { format!(r"\b(?:{body})\b") } else { body };
    let regex = RegexBuilder::new(&pattern)
        .case_insensitive(!options.match_case)
        .size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|error| FindError::InvalidRegex(error.to_string().lines().last().unwrap_or_default().trim().to_string()))?;
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

/// The first match starting at or after `from`, wrapping to the first one.
pub fn first_from(matches: &[Match], from: Position) -> Option<usize> {
    let index = matches.partition_point(|found| (found.line, found.start) < (from.line, from.column));
    match index {
        _ if matches.is_empty() => None,
        index if index == matches.len() => Some(0),
        index => Some(index),
    }
}

/// The match after a selection (or a caret): the first starting at or after its end, wrapping to the first one.
pub fn next_after(matches: &[Match], cursor: Cursor) -> Option<usize> {
    first_from(matches, cursor.anchor.max(cursor.head))
}

/// The match before a selection (or a caret): the last starting before its start, wrapping to the last one.
pub fn previous_before(matches: &[Match], cursor: Cursor) -> Option<usize> {
    let from = cursor.anchor.min(cursor.head);
    let index = matches.partition_point(|found| (found.line, found.start) < (from.line, from.column));
    match index {
        _ if matches.is_empty() => None,
        0 => Some(matches.len() - 1),
        index => Some(index - 1),
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

/// The find bar and the search it runs: in the pane that has the caret.
#[derive(Debug, Clone, Default)]
pub struct FindState {
    pub open: bool,
    pub query: String,
    pub options: FindOptions,
    /// Give the field the keyboard on the next frame, with its text selected.
    pub focus: bool,
    /// The text of the searched document changed.
    pub stale: bool,
    pub document: Option<Document>,
    pub matches: Vec<Match>,
    pub error: Option<FindError>,
    pub current: Option<usize>,
    searched: Option<Searched>,
}

impl FindState {
    /// Finds the matches again when the searched document, the query, the options or the text changed. Returns whether they were found again.
    pub fn refresh(&mut self, document: Document, lines: Lines<'_>) -> bool {
        let searched = Searched {
            document: document.clone(),
            query: self.query.clone(),
            options: self.options,
            text_address: lines.text.as_ptr() as usize,
            text_len: lines.text.len(),
            line_count: lines.count(),
        };
        if !self.stale && self.searched.as_ref() == Some(&searched) {
            return false;
        }
        self.stale = false;
        self.document = Some(document);
        (self.matches, self.error) = match find_all(lines, &self.query, self.options) {
            Ok(matches) => (matches, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        self.searched = Some(searched);
        true
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
        self.current = None;
        self.error = None;
        self.searched = None;
    }

    pub fn summary(&self) -> String {
        match (&self.error, self.matches.len(), self.current) {
            (Some(error), _, _) => error.to_string(),
            (None, _, _) if self.query.is_empty() => String::new(),
            (None, 0, _) => "No results".to_string(),
            (None, total, Some(current)) => format!("{} of {total}", current + 1),
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
    Close,
}

/// The find bar, floating at the top right of `top_right`: a field, the option toggles, the result count, previous and next, and close.
pub fn show_bar(ctx: &Context, top_right: Pos2, state: &mut FindState) -> Vec<FindEvent> {
    let mut events = Vec::new();
    let field_id = Id::unique(FIELD_ID);
    Area::new(Id::unique("find_bar"))
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
                    let (pressed_enter, pressed_escape, shift) = ctx.input(|input| (input.key_pressed(Key::Enter), input.key_pressed(Key::Escape), input.modifiers.shift));
                    if field.lost_focus() && pressed_enter {
                        events.push(if shift { FindEvent::Previous } else { FindEvent::Next });
                        field.request_focus();
                    }
                    if (field.has_focus() || field.lost_focus()) && pressed_escape {
                        events.push(FindEvent::Close);
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
                    let summary = state.summary();
                    let color = if state.error.is_some() { ui.visuals().error_fg_color } else { ui.visuals().text_color() };
                    ui.add_sized(vec2(90.0, ui.spacing().interact_size.y), egui::Label::new(egui::RichText::new(summary).color(color)).truncate());
                    if icon_button(ui, "Previous Match", "Previous Match (Shift+F3)", |painter, center, stroke| chevron(painter, center, stroke, -1.0)).clicked() {
                        events.push(FindEvent::Previous);
                    }
                    if icon_button(ui, "Next Match", "Next Match (F3)", |painter, center, stroke| chevron(painter, center, stroke, 1.0)).clicked() {
                        events.push(FindEvent::Next);
                    }
                    if icon_button(ui, "Close", "Close (Escape)", cross).clicked() {
                        events.push(FindEvent::Close);
                    }
                });
            });
        });
    events
}


/// A square button with an icon painted by `paint` around the center, named `label` for screen readers.
fn icon_button(ui: &mut Ui, label: &str, hint: &str, paint: impl FnOnce(&Painter, Pos2, Stroke)) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(ui.spacing().interact_size.y), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), label));
    if response.hovered() {
        ui.painter().rect_filled(rect, ui.visuals().widgets.hovered.corner_radius, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    paint(ui.painter(), rect.center(), Stroke::new(1.5, ui.visuals().text_color()));
    response.on_hover_text(hint)
}


/// A chevron pointing down for a `direction` of 1 and up for -1.
fn chevron(painter: &Painter, center: Pos2, stroke: Stroke, direction: f32) {
    let half = ICON_ARM / 2.0 * direction;
    painter.line(vec![center + vec2(-ICON_ARM, -half), center + vec2(0.0, half), center + vec2(ICON_ARM, -half)], stroke);
}


fn cross(painter: &Painter, center: Pos2, stroke: Stroke) {
    painter.line_segment([center + vec2(-ICON_ARM, -ICON_ARM), center + vec2(ICON_ARM, ICON_ARM)], stroke);
    painter.line_segment([center + vec2(-ICON_ARM, ICON_ARM), center + vec2(ICON_ARM, -ICON_ARM)], stroke);
}


#[cfg(test)]
mod tests {
    use std::ops::Range;

    use egui::Pos2;
    use egui_kittest::kittest::Queryable;

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

    #[test]
    fn next_goes_to_the_first_match_after_the_caret_or_selection() {
        let matches = three();
        assert_eq!(next_after(&matches, Cursor::at(at(0, 0))), Some(0));
        assert_eq!(next_after(&matches, Cursor::at(at(0, 1))), Some(0));
        assert_eq!(next_after(&matches, Cursor::at(at(0, 2))), Some(1));
        assert_eq!(next_after(&matches, matches[0].cursor()), Some(1));
        assert_eq!(next_after(&matches, matches[1].cursor()), Some(2));
    }

    #[test]
    fn next_wraps_to_the_first_match() {
        assert_eq!(next_after(&three(), three()[2].cursor()), Some(0));
        assert_eq!(next_after(&three(), Cursor::at(at(5, 0))), Some(0));
    }

    #[test]
    fn previous_goes_to_the_last_match_before_the_caret_or_selection() {
        let matches = three();
        assert_eq!(previous_before(&matches, Cursor::at(at(3, 0))), Some(2));
        assert_eq!(previous_before(&matches, Cursor::at(at(2, 0))), Some(1));
        assert_eq!(previous_before(&matches, matches[2].cursor()), Some(1));
        assert_eq!(previous_before(&matches, matches[1].cursor()), Some(0));
    }

    #[test]
    fn previous_wraps_to_the_last_match() {
        assert_eq!(previous_before(&three(), three()[0].cursor()), Some(2));
        assert_eq!(previous_before(&three(), Cursor::at(at(0, 0))), Some(2));
    }

    #[test]
    fn navigation_without_matches_goes_nowhere() {
        assert_eq!(next_after(&[], Cursor::default()), None);
        assert_eq!(previous_before(&[], Cursor::default()), None);
        assert_eq!(first_from(&[], at(0, 0)), None);
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
        assert!(state.refresh(Document::A, lines));
        assert_eq!(state.matches.len(), 2);
        assert!(!state.refresh(Document::A, lines));
        assert!(state.refresh(Document::B, lines), "another pane is another search");
        state.query = "b".to_string();
        assert!(state.refresh(Document::B, lines));
        state.stale = true;
        assert!(state.refresh(Document::B, lines), "an edit makes the matches stale");
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
            .build_ui_state(|ui, test: &mut BarTest| test.events.extend(show_bar(ui.ctx(), Pos2::new(580.0, 10.0), &mut test.state)), test);
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
    fn the_bar_shows_the_result_count() {
        let harness = bar();
        harness.get_by_label("? of 3");
    }
}
