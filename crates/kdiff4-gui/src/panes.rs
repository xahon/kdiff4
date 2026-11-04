use std::path::Path;

use egui::{Align, Color32, CursorIcon, Key, Layout, RichText, Sense, Stroke, TextEdit, Ui, vec2};
use kdiff4_protocol::Document;

use crate::char_diff::CharDiffLook;
use crate::config::WhitespaceConfig;
use crate::editing::{Cursor, Lines, Position};
use crate::mirror::Mirror;
use crate::rows::Rows;
use crate::text_view::{self, Caret, CharRuns, MarginKind, PointerAction, ViewStyle, Viewport};
use crate::theme::{self, Palette};


const SPLITTER_HEIGHT: f32 = 6.0;
const MIN_MERGE_FRACTION: f32 = 0.1;
const MAX_MERGE_FRACTION: f32 = 0.9;
pub const DEFAULT_MERGE_FRACTION: f32 = 0.5;


/// What the user did in the panes; the app turns these into caret moves and commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneAction {
    /// The mouse button went down in a pane; `void_row` is the row when it went down on a void. `extend` (Shift) extends the selection there.
    Press { document: Document, position: Position, void_row: Option<usize>, extend: bool },
    /// The button pressed in a pane is still held, over `position`.
    Drag { document: Document, position: Position, void_row: Option<usize> },
    SelectWord(Document, Position),
    SelectLine(Document, usize),
    /// A click on a gutter block of a region.
    SelectRegion(Document, usize),
    /// Enter in a header path field, with the field's text.
    CommitPath(Document, String),
    Browse(Document),
}


/// View state of the panes, kept by the app between frames.
#[derive(Debug, Clone)]
pub struct PaneState {
    /// The one caret on screen, with its selection.
    pub caret: Option<(Document, Cursor)>,
    /// The row of the caret when it sits on a void; its position is then the start of the line after the void.
    pub void_row: Option<usize>,
    /// The caret is not drawn, as after a jump, until it is moved again.
    pub caret_hidden: bool,
    /// A region to scroll every pane to on the next frame, with context rows above it, unless every pane already shows all of it.
    pub jump: Option<usize>,
    /// A region D scrolls into view on the next frame if it is out of view, unless D scrolls with the source panes.
    pub reveal_region: Option<usize>,
    /// Scroll the caret into view on the next frame.
    pub reveal_caret: bool,
    /// Rows that fit in the pane with the caret, as of the last frame.
    pub page_rows: usize,
    /// Share of the height given to the merge pane.
    pub merge_fraction: f32,
    /// Texts of the header path fields, in `Document::ALL` order.
    pub path_fields: [String; 4],
    /// Vertical scroll offset shared by the source panes, which scroll together row by row.
    pub source_offset: f32,
    /// What each pane showed in the last frame, in `Document::ALL` order.
    pub viewports: [Option<Viewport>; 4],
}


impl Default for PaneState {
    fn default() -> Self {
        PaneState { caret: None, void_row: None, caret_hidden: false, jump: None, reveal_region: None, reveal_caret: false, page_rows: 1, merge_fraction: DEFAULT_MERGE_FRACTION, path_fields: Default::default(), source_offset: 0.0, viewports: [None; 4] }
    }
}


impl PaneState {
    pub fn caret_in(&self, document: Document) -> Option<Cursor> {
        self.caret
            .filter(|(caret_document, _)| *caret_document == document)
            .map(|(_, cursor)| cursor)
    }


    /// Moves the caret and shows it; `void_row` is the row when it sits on a void.
    pub fn set_caret(&mut self, document: Document, cursor: Cursor, void_row: Option<usize>) {
        self.caret = Some((document, cursor));
        self.void_row = void_row;
        self.caret_hidden = false;
    }


    /// Moves the caret to where a jump lands, without drawing it until it is moved again.
    pub fn set_hidden_caret(&mut self, document: Document, cursor: Cursor, void_row: Option<usize>) {
        self.set_caret(document, cursor, void_row);
        self.caret_hidden = true;
    }


    /// The caret to draw in `document`'s pane, if it is there and not hidden.
    fn visible_caret(&self, document: Document) -> Option<Caret> {
        self.caret_in(document)
            .filter(|_| !self.caret_hidden)
            .map(|cursor| Caret { cursor, void_row: self.void_row.clone() })
    }
}


/// View settings the panes draw with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneOptions {
    pub line_numbers: bool,
    pub gutters: bool,
    pub current_region_color: Color32,
    pub jump_context_rows: usize,
    pub region_gap: f32,
    pub region_corner_radius: u8,
    pub char_diff: CharDiffLook,
    pub whitespace: WhitespaceConfig,
}


/// Source panes side by side; for a merge, the merge pane below them, behind a splitter. A lined-up D scrolls with the source panes.
pub fn show_session(ui: &mut Ui, mirror: &Mirror, state: &mut PaneState, options: &PaneOptions) -> Vec<PaneAction> {
    let palette = &theme::palette_of(ui);
    let style = |margin: MarginKind| ViewStyle { palette, line_numbers: options.line_numbers, margin, current_region: mirror.current_region(), current_region_color: options.current_region_color, jump_context_rows: options.jump_context_rows, region_gap: options.region_gap, region_corner_radius: options.region_corner_radius, char_diff: options.char_diff, whitespace: options.whitespace.clone() };
    let source_style = style(if options.gutters { MarginKind::Gutters } else { MarginKind::Nothing });
    let output_style = style(MarginKind::Marks);
    let mut actions = Vec::new();
    if state.jump.is_some_and(|region| region_in_view_everywhere(mirror, &state.viewports, region)) {
        state.jump = None;
    }
    egui::CentralPanel::default().show(ui, |ui| {
        let total = ui.available_size();
        let shared = state.source_offset.clone();
        let mut offsets = Vec::new();
        if mirror.output().is_none() {
            source_columns(ui, &source_style, mirror, state, &mut actions, &mut offsets);
        } else {
            let usable = (total.y - SPLITTER_HEIGHT - 2.0 * ui.spacing().item_spacing.y).max(0.0);
            let merge_height = (usable * state.merge_fraction).round();
            ui.allocate_ui(vec2(total.x, usable - merge_height), |ui| source_columns(ui, &source_style, mirror, state, &mut actions, &mut offsets));
            splitter(ui, palette, usable, merge_height, &mut state.merge_fraction);
            ui.allocate_ui(vec2(total.x, merge_height), |ui| output_pane(ui, &output_style, mirror, state, &mut actions, &mut offsets));
        }
        state.source_offset = next_shared_offset(shared.clone(), &offsets);
        if state.source_offset != shared {
            ui.ctx().request_repaint();
        }
    });
    state.jump = None;
    state.reveal_region = None;
    actions
}


/// Whether every pane of the session showed all rows of `region` in the last frame. A pane with no viewport yet does not.
fn region_in_view_everywhere(mirror: &Mirror, viewports: &[Option<Viewport>; 4], region: usize) -> bool {
    let sources = mirror
        .documents()
        .iter()
        .map(|text| (text.document.clone(), mirror.rows(text.document.clone()).map(Rows::Mapped)));
    let output = mirror
        .output()
        .map(|_| (Document::D, Some(Rows::Mapped(mirror.output_rows()))));
    sources
        .chain(output)
        .all(|(document, rows)| region_in_view(rows, viewports[document_index(document)], region))
}


/// Whether `viewport` shows every row of `region`. Without rows for it, there is nothing to scroll to.
fn region_in_view(rows: Option<Rows<'_>>, viewport: Option<Viewport>, region: usize) -> bool {
    match rows.and_then(|rows| rows.region_rows(region)) {
        Some(range) => viewport.is_some_and(|viewport| viewport.shows(range)),
        None => true,
    }
}


/// Moves the border between the source panes and the merge pane. The share is kept, so the split scales with the window.
fn splitter(ui: &mut Ui, palette: &Palette, usable: f32, merge_height: f32, fraction: &mut f32) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), SPLITTER_HEIGHT), Sense::drag());
    let response = response.on_hover_cursor(CursorIcon::ResizeVertical);
    if response.dragged() && usable > 0.0 {
        *fraction = ((merge_height - response.drag_delta().y) / usable).clamp(MIN_MERGE_FRACTION, MAX_MERGE_FRACTION);
    }
    if response.hovered() || response.dragged() {
        ui.painter().hline(rect.x_range(), rect.center().y, Stroke::new(2.0, palette.pane_border));
    }
}


/// The source panes are forced to one scroll offset; the pane the user scrolled this frame, or the one that revealed the caret, sets it for the next. Each pushes its offset after this frame to `offsets`.
fn source_columns(ui: &mut Ui, style: &ViewStyle<'_>, mirror: &Mirror, state: &mut PaneState, actions: &mut Vec<PaneAction>, offsets: &mut Vec<f32>) {
    let count = mirror.documents().len().max(1);
    let shared = state.source_offset.clone();
    ui.columns(count, |columns| {
        for (column, text) in columns.iter_mut().zip(mirror.documents()) {
            let lines = Lines { text: &text.text, ranges: &text.lines };
            let rows = mirror.rows(text.document.clone()).map_or(Rows::PerLine(lines.count()), Rows::Mapped);
            let char_runs = mirror.char_diffs(text.document.clone()).map(|runs| CharRuns { document: text.document.clone(), lines: runs, sources: mirror.documents().len() });
            offsets.push(pane(column, style, text.document.clone(), text.label.as_deref(), Some(&text.path), false, lines, rows, char_runs, Some(shared.clone()), None, state, actions));
        }
    });
}


fn next_shared_offset(shared: f32, offsets: &[f32]) -> f32 {
    offsets
        .iter()
        .find(|offset| **offset != shared)
        .cloned()
        .unwrap_or(shared)
}


/// D scrolls on its own, unless it is lined up with the source panes: then it shares their offset.
fn output_pane(ui: &mut Ui, style: &ViewStyle<'_>, mirror: &Mirror, state: &mut PaneState, actions: &mut Vec<PaneAction>, offsets: &mut Vec<f32>) {
    if let Some(output) = mirror.output() {
        let lines = output.lines();
        let aligned = mirror.is_aligned();
        let offset = aligned.then(|| state.source_offset.clone());
        let rows = Rows::Mapped(mirror.output_rows());
        let reveal_row = state.reveal_region.filter(|_| !aligned).and_then(|region| rows.region_start(region));
        let after = pane(ui, style, Document::D, None, output.path.as_deref(), output.modified, lines, rows, None, offset, reveal_row, state, actions);
        if aligned {
            offsets.push(after);
        }
    }
}


/// `reveal_row` is scrolled into view if it is out of view. Returns the pane's vertical scroll offset after this frame.
fn pane(ui: &mut Ui, style: &ViewStyle<'_>, document: Document, label: Option<&str>, path: Option<&Path>, modified: bool, lines: Lines<'_>, rows: Rows<'_>, char_runs: Option<CharRuns<'_>>, offset: Option<f32>, reveal_row: Option<usize>, state: &mut PaneState, actions: &mut Vec<PaneAction>) -> f32 {
    let palette = style.palette;
    let radius = theme::corner_radius_of(ui);
    theme::pane_frame(palette, radius)
        .show(ui, |ui| {
            header(ui, palette, radius, document.clone(), label, path, modified, state, actions);
            theme::pane_body_frame()
                .show(ui, |ui| {
                    let caret = state.visible_caret(document.clone());
                    let jump_row = state.jump.and_then(|region| rows.region_start(region));
                    let response = text_view::show(ui, document.letter(), lines, rows, char_runs, caret, state.reveal_caret && caret.is_some(), jump_row, reveal_row, offset, style);
                    if state.caret_in(document.clone()).is_some() {
                        state.page_rows = response.page_rows;
                    }
                    state.viewports[document_index(document.clone())] = Some(response.viewport);
                    match response.pointer {
                        Some(PointerAction::Press { position, void_row, extend }) => actions.push(PaneAction::Press { document: document.clone(), position, void_row, extend }),
                        Some(PointerAction::Drag { position, void_row }) => actions.push(PaneAction::Drag { document: document.clone(), position, void_row }),
                        Some(PointerAction::SelectWord(position)) => actions.push(PaneAction::SelectWord(document.clone(), position)),
                        Some(PointerAction::SelectLine(line)) => actions.push(PaneAction::SelectLine(document.clone(), line)),
                        Some(PointerAction::SelectRegion(region)) => actions.push(PaneAction::SelectRegion(document.clone(), region)),
                        None => {}
                    }
                    response.viewport.offset
                })
                .inner
        })
        .inner
}


/// Letter, label, editable path and Browse button. The field shows the document's path unless the user is editing it.
fn header(ui: &mut Ui, palette: &Palette, radius: u8, document: Document, label: Option<&str>, path: Option<&Path>, modified: bool, state: &mut PaneState, actions: &mut Vec<PaneAction>) {
    theme::pane_header_frame(palette, document.clone(), radius).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let letter = if modified { format!("{}*", document.letter()) } else { document.letter().to_string() };
            let letter_response = ui.label(RichText::new(letter).strong().color(palette.text));
            if modified {
                letter_response.on_hover_text("Modified since it was loaded or saved");
            }
            if let Some(label) = label {
                ui.label(RichText::new(label).color(palette.weak_text));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("Browse...").clicked() {
                    actions.push(PaneAction::Browse(document.clone()));
                }
                let field_id = ui.make_persistent_id(("path_field", document.letter()));
                let field = &mut state.path_fields[document_index(document.clone())];
                if !ui.memory(|memory| memory.has_focus(field_id)) {
                    *field = path.map(|path| path.display().to_string()).unwrap_or_default();
                }
                let hint = if document == Document::D { "unnamed, kept in memory until saved" } else { "" };
                let response = ui.add(TextEdit::singleline(field).id(field_id).desired_width(ui.available_width()).hint_text(hint));
                if response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
                    actions.push(PaneAction::CommitPath(document.clone(), field.clone()));
                }
            });
        });
    });
    let separator_y = ui.cursor().top();
    ui.painter().hline(ui.max_rect().x_range(), separator_y, Stroke::new(1.0, palette.pane_border));
}


fn document_index(document: Document) -> usize {
    match document {
        Document::A => 0,
        Document::B => 1,
        Document::C => 2,
        Document::D => 3,
    }
}


#[cfg(test)]
mod tests {
    use kdiff4_protocol::engine::{Comparison, HunkChange, Region, RegionStatus, Row};

    use super::*;
    use crate::rows::RowMap;


    #[test]
    fn caret_belongs_to_one_document() {
        let mut state = PaneState::default();
        assert_eq!(state.caret_in(Document::A), None);
        let cursor = Cursor { anchor: Position { line: 0, column: 0 }, head: Position { line: 2, column: 1 } };
        state.caret = Some((Document::B, cursor));
        assert_eq!(state.caret_in(Document::B), Some(cursor));
        assert_eq!(state.caret_in(Document::A), None);
        assert_eq!(state.caret_in(Document::D), None);
    }


    #[test]
    fn jumps_hide_the_caret_until_it_moves() {
        let mut state = PaneState::default();
        let cursor = Cursor::at(Position { line: 4, column: 0 });
        state.set_hidden_caret(Document::A, cursor, Some(7));
        assert_eq!(state.caret_in(Document::A), Some(cursor));
        assert_eq!(state.visible_caret(Document::A), None);
        state.set_caret(Document::A, cursor, None);
        assert_eq!(state.visible_caret(Document::A), Some(Caret { cursor, void_row: None }));
    }


    #[test]
    fn region_counts_as_in_view_when_all_its_rows_are() {
        let comparison = Comparison {
            regions: vec![
                Region { hunks: vec![0..10, 0..10], status: RegionStatus::Diff(HunkChange::Unchanged), rows: (0..10).map(|line| Row { lines: vec![Some(line), Some(line)] }).collect() },
                Region { hunks: vec![10..12, 10..12], status: RegionStatus::Diff(HunkChange::Modified), rows: (10..12).map(|line| Row { lines: vec![Some(line), Some(line)] }).collect() },
            ],
        };
        let map = RowMap::for_source(&comparison, 0, 12, None);
        let rows = Some(Rows::Mapped(&map));
        let viewport = |offset: f32| Some(Viewport { offset, height: 50.0, row_height: 10.0 });
        assert!(region_in_view(rows, viewport(80.0), 1));
        assert!(region_in_view(rows, viewport(70.0), 1));
        assert!(!region_in_view(rows, viewport(69.0), 1));
        assert!(!region_in_view(rows, viewport(101.0), 1));
        assert!(!region_in_view(rows, viewport(0.0), 1));
        assert!(!region_in_view(rows, None, 1));
        assert!(region_in_view(rows, None, 5));
        assert!(region_in_view(None, None, 1));
    }


    #[test]
    fn shared_offset_follows_the_pane_that_moved() {
        assert_eq!(next_shared_offset(10.0, &[10.0, 10.0, 10.0]), 10.0);
        assert_eq!(next_shared_offset(10.0, &[10.0, 42.0, 10.0]), 42.0);
        assert_eq!(next_shared_offset(10.0, &[3.0, 3.0]), 3.0);
        assert_eq!(next_shared_offset(10.0, &[]), 10.0);
    }


    #[test]
    fn merge_pane_starts_at_half_height() {
        assert_eq!(PaneState::default().merge_fraction, 0.5);
    }
}
