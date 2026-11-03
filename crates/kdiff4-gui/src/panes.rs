use std::path::Path;

use egui::{Align, CursorIcon, Key, Layout, RichText, Sense, Stroke, TextEdit, Ui, vec2};
use kdiff4_protocol::Document;

use crate::editing::{Cursor, Lines, Position};
use crate::mirror::Mirror;
use crate::rows::Rows;
use crate::text_view::{self, PointerAction};
use crate::theme::{self, Palette};


const SPLITTER_HEIGHT: f32 = 6.0;
const MIN_MERGE_FRACTION: f32 = 0.1;
const MAX_MERGE_FRACTION: f32 = 0.9;
pub const DEFAULT_MERGE_FRACTION: f32 = 0.5;


/// What the user did in the panes; the app turns these into caret moves and commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneAction {
    /// The mouse button went down in a pane; `extend` (Shift) extends the selection there.
    Press { document: Document, position: Position, extend: bool },
    /// The button pressed in a pane is still held, over `Position`.
    Drag(Document, Position),
    SelectWord(Document, Position),
    SelectLine(Document, usize),
    /// Enter in a header path field, with the field's text.
    CommitPath(Document, String),
    Browse(Document),
}


/// View state of the panes, kept by the app between frames.
#[derive(Debug, Clone)]
pub struct PaneState {
    /// The one caret on screen, with its selection.
    pub caret: Option<(Document, Cursor)>,
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
}


impl Default for PaneState {
    fn default() -> Self {
        PaneState { caret: None, reveal_caret: false, page_rows: 1, merge_fraction: DEFAULT_MERGE_FRACTION, path_fields: Default::default(), source_offset: 0.0 }
    }
}


impl PaneState {
    pub fn caret_in(&self, document: Document) -> Option<Cursor> {
        self.caret
            .filter(|(caret_document, _)| *caret_document == document)
            .map(|(_, cursor)| cursor)
    }
}


/// Source panes side by side; for a merge, the merge pane below them, behind a splitter.
pub fn show_session(ui: &mut Ui, mirror: &Mirror, state: &mut PaneState) -> Vec<PaneAction> {
    let palette = theme::palette_of(ui);
    let mut actions = Vec::new();
    egui::CentralPanel::default().show(ui, |ui| {
        let total = ui.available_size();
        if mirror.output().is_none() {
            source_columns(ui, palette, mirror, state, &mut actions);
            return;
        }
        let usable = (total.y - SPLITTER_HEIGHT - 2.0 * ui.spacing().item_spacing.y).max(0.0);
        let merge_height = (usable * state.merge_fraction).round();
        ui.allocate_ui(vec2(total.x, usable - merge_height), |ui| source_columns(ui, palette, mirror, state, &mut actions));
        splitter(ui, palette, usable, merge_height, &mut state.merge_fraction);
        ui.allocate_ui(vec2(total.x, merge_height), |ui| output_pane(ui, palette, mirror, state, &mut actions));
    });
    actions
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


/// The source panes are forced to one scroll offset; the pane the user scrolled this frame, or the one that revealed the caret, sets it for the next.
fn source_columns(ui: &mut Ui, palette: &Palette, mirror: &Mirror, state: &mut PaneState, actions: &mut Vec<PaneAction>) {
    let count = mirror.documents().len().max(1);
    let shared = state.source_offset.clone();
    let mut offsets = Vec::with_capacity(count);
    ui.columns(count, |columns| {
        for (column, text) in columns.iter_mut().zip(mirror.documents()) {
            let lines = Lines { text: &text.text, ranges: &text.lines };
            let rows = mirror.rows(text.document.clone()).map_or(Rows::PerLine(lines.count()), Rows::Mapped);
            offsets.push(pane(column, palette, text.document.clone(), text.label.as_deref(), Some(&text.path), false, lines, rows, Some(shared.clone()), state, actions));
        }
    });
    state.source_offset = next_shared_offset(shared, &offsets);
    if state.source_offset != shared {
        ui.ctx().request_repaint();
    }
}


fn next_shared_offset(shared: f32, offsets: &[f32]) -> f32 {
    offsets
        .iter()
        .find(|offset| **offset != shared)
        .cloned()
        .unwrap_or(shared)
}


fn output_pane(ui: &mut Ui, palette: &Palette, mirror: &Mirror, state: &mut PaneState, actions: &mut Vec<PaneAction>) {
    if let Some(output) = mirror.output() {
        let lines = output.lines();
        pane(ui, palette, Document::D, None, output.path.as_deref(), output.modified, lines, Rows::PerLine(lines.count()), None, state, actions);
    }
}


/// Returns the pane's vertical scroll offset after this frame.
fn pane(ui: &mut Ui, palette: &Palette, document: Document, label: Option<&str>, path: Option<&Path>, modified: bool, lines: Lines<'_>, rows: Rows<'_>, offset: Option<f32>, state: &mut PaneState, actions: &mut Vec<PaneAction>) -> f32 {
    let radius = theme::corner_radius_of(ui);
    theme::pane_frame(palette, radius)
        .show(ui, |ui| {
            header(ui, palette, radius, document.clone(), label, path, modified, state, actions);
            theme::pane_body_frame()
                .show(ui, |ui| {
                    let caret = state.caret_in(document.clone());
                    let response = text_view::show(ui, document.letter(), lines, rows, caret, state.reveal_caret && caret.is_some(), offset, palette);
                    if caret.is_some() {
                        state.page_rows = response.page_rows;
                    }
                    match response.pointer {
                        Some(PointerAction::Press { position, extend }) => actions.push(PaneAction::Press { document: document.clone(), position, extend }),
                        Some(PointerAction::Drag(position)) => actions.push(PaneAction::Drag(document.clone(), position)),
                        Some(PointerAction::SelectWord(position)) => actions.push(PaneAction::SelectWord(document.clone(), position)),
                        Some(PointerAction::SelectLine(line)) => actions.push(PaneAction::SelectLine(document.clone(), line)),
                        None => {}
                    }
                    response.offset
                })
                .inner
        })
        .inner
}


/// Letter, label, editable path and Browse button. The field shows the document's path unless the user is editing it.
fn header(ui: &mut Ui, palette: &Palette, radius: u8, document: Document, label: Option<&str>, path: Option<&Path>, modified: bool, state: &mut PaneState, actions: &mut Vec<PaneAction>) {
    theme::pane_header_frame(palette, radius).show(ui, |ui| {
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
    use super::*;


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
