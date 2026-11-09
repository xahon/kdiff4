use std::ops::Range;
use std::path::Path;

use egui::{Align, Color32, CursorIcon, Key, Layout, RichText, Sense, Stroke, TextEdit, Ui, vec2};
use kdiff4_protocol::{AlignmentMark, Document, EolStyle};

use crate::alignment_gutter::PaneMarks;
use crate::config::WhitespaceConfig;
use crate::editing::{Cursor, Lines, Position};
use crate::eol;
use crate::find::FindState;
use crate::minimap;
use crate::mirror::Mirror;
use crate::overview;
use crate::rows::Rows;
use crate::scroll::{ScrollMotion, SmoothScroll};
use crate::text_view::{self, Caret, CharRuns, FindHits, MarginKind, MenuAction, PointerAction, ViewStyle, Viewport, Wheel};
use crate::theme::{self, Palette};

const SPLITTER_HEIGHT: f32 = 0.0;
const MIN_MERGE_FRACTION: f32 = 0.1;
const MAX_MERGE_FRACTION: f32 = 0.9;
pub const DEFAULT_MERGE_FRACTION: f32 = 0.5;
/// A pane that ended a frame this many pixels away from the offset it was given was scrolled by the user or revealed the caret.
const MOVED_BY_PANE: f32 = 0.5;
/// Longest frame a scroll animation steps by, so a stall doesn't skip it.
const MAX_SCROLL_STEP: f32 = 0.1;

/// What the user did in the panes; the app turns these into caret moves and commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneAction {
    /// The mouse button went down in a pane; `void_row` is the row when it went down on a void. `extend` (Shift) extends the selection there.
    Press {
        document: Document,
        position: Position,
        void_row: Option<usize>,
        extend: bool,
    },
    /// The button pressed in a pane is still held, over `position`.
    Drag {
        document: Document,
        position: Position,
        void_row: Option<usize>,
    },
    SelectWord(Document, Position),
    SelectLine(Document, usize),
    /// A triple-click in the overview strip: the text of the line without its line break, so the caret stays on the line.
    SelectLineText(Document, usize),
    /// A click on a gutter block of a region.
    SelectRegion(Document, usize),
    /// Copy the selection of the caret to the clipboard.
    Copy,
    /// Mark the selection in a source pane for alignment.
    MarkSelection(Document),
    /// Mark an anchor in a source pane before this line, which may be one past the last.
    MarkAnchor(Document, usize),
    RemoveAlignmentMark(Document, u32),
    /// Move the mark `number` of a document to these lines.
    SetAlignmentMarkLines(Document, u32, Range<usize>),
    /// Give the mark `number` of a document the number `to`.
    RenumberAlignmentMark(Document, u32, u32),
    /// Show the alignment `number` at the place with the index `slot` among the other alignments.
    PlaceAlignment(u32, usize),
    /// Enter in a header path field, with the field's text.
    CommitPath(Document, String),
    Browse(Document),
    /// The overview strip took a press; Up and Down now switch between its lines.
    FocusStrip,
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
    /// The caret was last placed in the overview strip, so Up and Down switch between its lines instead of moving by rows.
    pub strip_focused: bool,
    /// A region to scroll every pane to, with context rows above it, unless every pane already shows all of it. Kept until every pane has been laid out once.
    pub jump: Option<usize>,
    /// Scroll the caret into view on the next frame.
    pub reveal_caret: bool,
    /// Rows that fit in the pane with the caret, as of the last frame.
    pub page_rows: usize,
    /// Share of the height given to the merge pane.
    pub merge_fraction: f32,
    /// Texts of the header path fields, in `Document::ALL` order.
    pub path_fields: [String; 4],
    /// Vertical scrolling shared by the source panes, which scroll together row by row, and by D when it lines up with them.
    pub source_scroll: SmoothScroll,
    /// Vertical scrolling of D when it doesn't line up with the source panes.
    pub output_scroll: SmoothScroll,
    /// Vertical scrolling of each pane on its own while the panes scroll independently, in `Document::ALL` order.
    pub pane_scrolls: [SmoothScroll; 4],
    /// Every pane scrolls on its own, as of the last frame.
    pub independent_scroll: bool,
    /// The pane, among those that line up with the source panes, that the user last pressed in, scrolled with the wheel or dragged the scroll bar of. The others take its offset when scrolling is synced again.
    pub scroll_leader: Option<Document>,
    /// What each pane showed in the last frame, in `Document::ALL` order.
    pub viewports: [Option<Viewport>; 4],
    /// The find bar and its matches in the pane with the caret.
    pub find: FindState,
}

impl Default for PaneState {
    fn default() -> Self {
        PaneState {
            caret: None,
            void_row: None,
            caret_hidden: false,
            strip_focused: false,
            jump: None,
            reveal_caret: false,
            page_rows: 1,
            merge_fraction: DEFAULT_MERGE_FRACTION,
            path_fields: Default::default(),
            source_scroll: SmoothScroll::default(),
            output_scroll: SmoothScroll::default(),
            pane_scrolls: [SmoothScroll::default(); 4],
            independent_scroll: false,
            scroll_leader: None,
            viewports: [None; 4],
            find: FindState::default(),
        }
    }
}

impl PaneState {
    pub fn caret_in(&self, document: Document) -> Option<Cursor> {
        self.caret.filter(|(caret_document, _)| *caret_document == document).map(|(_, cursor)| cursor)
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
        self.strip_focused = false;
    }

    /// The caret to draw in `document`'s pane, if it is there and not hidden.
    pub fn visible_caret(&self, document: Document) -> Option<Caret> {
        self.caret_in(document).filter(|_| !self.caret_hidden).map(|cursor| Caret {
            cursor,
            void_row: self.void_row.clone(),
        })
    }

    /// The pane the others follow when scrolling is synced again: the scroll leader if it lines up with the source panes, A otherwise.
    pub fn scroll_leader(&self, aligned: bool) -> Document {
        match self.scroll_leader {
            Some(Document::D) if !aligned => Document::A,
            Some(document) => document,
            None => Document::A,
        }
    }

    /// Switches between synced and independent scrolling. Going independent, every pane starts where it is; going synced, the panes that line up with the source panes take the offset of the scroll leader, and D on its own keeps its offset.
    pub fn set_independent_scroll(&mut self, independent: bool, aligned: bool) {
        if independent == self.independent_scroll {
            return;
        }
        if independent {
            for document in Document::ALL {
                let scroll = self.scroll_of(scroll_group(document.clone(), aligned, false)).clone();
                self.pane_scrolls[document_index(document)] = scroll;
            }
        } else {
            let leader = self.scroll_leader(aligned);
            self.source_scroll = self.pane_scrolls[document_index(leader)].clone();
            self.output_scroll = match aligned {
                true => self.source_scroll.clone(),
                false => self.pane_scrolls[document_index(Document::D)].clone(),
            };
        }
        self.independent_scroll = independent;
    }

    /// The scroll offset the minimap shows and moves, with the index of the pane whose viewport it stands for: the source panes', or the scroll leader's while scrolling is independent.
    pub fn map_scroll(&mut self, aligned: bool) -> (usize, &mut SmoothScroll) {
        let document = match self.independent_scroll {
            true => self.scroll_leader(aligned),
            false => Document::A,
        };
        let group = scroll_group(document.clone(), aligned, self.independent_scroll.clone());
        (document_index(document), self.scroll_of(group))
    }

    fn scroll_of(&mut self, group: ScrollGroup) -> &mut SmoothScroll {
        match group {
            ScrollGroup::Sources => &mut self.source_scroll,
            ScrollGroup::Output => &mut self.output_scroll,
            ScrollGroup::Pane(document) => &mut self.pane_scrolls[document_index(document)],
        }
    }
}

/// View settings the panes draw with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneOptions {
    pub line_numbers: bool,
    pub gutters: bool,
    pub current_region_color: Color32,
    pub jump_context_rows: usize,
    /// Rows one wheel notch scrolls.
    pub scroll_step_rows: usize,
    pub scroll_motion: ScrollMotion,
    pub region_corner_radius: u8,
    pub other_region_border_percent: u8,
    pub whitespace: WhitespaceConfig,
    pub overview_strip: bool,
    pub minimap: bool,
    /// Every pane scrolls on its own instead of with the others.
    pub independent_scroll: bool,
    pub alignment_color: Color32,
    /// Pixels between the lines of the arrows of moved hunks.
    pub inset_arrow_gap: u8,
}

/// Which scroll offset a pane follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScrollGroup {
    Sources,
    Output,
    /// The pane scrolls on its own, as every pane does while scrolling is independent.
    Pane(Document),
}

/// The scroll offset `document`'s pane follows: its own while scrolling is independent, otherwise the source panes', or D's own when D doesn't line up with them.
fn scroll_group(document: Document, aligned: bool, independent: bool) -> ScrollGroup {
    match (independent, document) {
        (true, document) => ScrollGroup::Pane(document),
        (false, Document::D) if !aligned => ScrollGroup::Output,
        (false, _) => ScrollGroup::Sources,
    }
}

/// How a pane's vertical scrolling went in a frame: the offset it was given, where it ended up, the wheel scrolling over it, its largest offset, and whether the mouse button went down in it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PaneScroll {
    document: Document,
    pressed: bool,
    group: ScrollGroup,
    given: f32,
    after: f32,
    wheel: Wheel,
    max: f32,
}

/// Shows the overview strip; whatever it returns also gives the strip the keyboard.
fn strip_actions(ui: &mut Ui, mirror: &Mirror, state: &PaneState, style: &ViewStyle<'_>, actions: &mut Vec<PaneAction>) {
    let shown = overview::show(ui, mirror, state, style);
    if !shown.is_empty() {
        actions.extend(shown);
        actions.push(PaneAction::FocusStrip);
    }
}

/// Source panes side by side, then the overview strip when it is on; for a merge, the merge pane below them, behind a splitter. A lined-up D scrolls with the source panes.
pub fn show_session(ui: &mut Ui, mirror: &Mirror, state: &mut PaneState, options: &PaneOptions) -> Vec<PaneAction> {
    let palette = &theme::palette_of(ui);
    let style = |margin: MarginKind| ViewStyle {
        palette,
        line_numbers: options.line_numbers,
        margin,
        current_region: mirror.current_region(),
        current_region_color: options.current_region_color,
        scroll_step_rows: options.scroll_step_rows,
        region_corner_radius: options.region_corner_radius,
        other_region_border_opacity: f32::from(options.other_region_border_percent) / 100.0,
        whitespace: options.whitespace.clone(),
        alignment_color: None,
        inset_arrow_gap: f32::from(options.inset_arrow_gap),
    };
    let source_style = ViewStyle {
        alignment_color: Some(options.alignment_color),
        ..style(if options.gutters { MarginKind::Gutters } else { MarginKind::Nothing })
    };
    let output_style = style(MarginKind::Marks);
    let mut actions = Vec::new();
    state.set_independent_scroll(options.independent_scroll, mirror.is_aligned());
    if let Some(region) = state.jump
        && (region_in_view_everywhere(mirror, &state.viewports, region) || start_jump(mirror, state, region, options))
    {
        state.jump = None;
    }
    let dt = ui.input(|input| input.stable_dt).min(MAX_SCROLL_STEP);
    state.source_scroll.step(dt.clone(), options.scroll_motion);
    state.output_scroll.step(dt.clone(), options.scroll_motion);
    for scroll in state.pane_scrolls.iter_mut() {
        scroll.step(dt.clone(), options.scroll_motion);
    }
    if options.minimap {
        egui::Panel::right("minimap")
            .resizable(false)
            .exact_size(minimap::WIDTH)
            .show_separator_line(false)
            .frame(egui::Frame::NONE)
            .show(ui, |ui| minimap::show(ui, mirror, state, palette));
    }
    egui::CentralPanel::default().show(ui, |ui| {
        let total = ui.available_size();
        let mut scrolls = Vec::new();
        let strip_documents = mirror.documents().len() + usize::from(mirror.output().is_some());
        let strip_height = if options.overview_strip { overview::height(ui, strip_documents) } else { 0.0 };
        let strip_space = if options.overview_strip { strip_height + ui.spacing().item_spacing.y } else { 0.0 };
        if mirror.output().is_none() {
            ui.allocate_ui(vec2(total.x, (total.y - strip_space).max(0.0)), |ui| {
                source_columns(ui, &source_style, mirror, state, &mut actions, &mut scrolls)
            });
            if options.overview_strip {
                ui.allocate_ui(vec2(total.x, strip_height), |ui| strip_actions(ui, mirror, state, &source_style, &mut actions));
            }
        } else {
            let usable = (total.y - strip_space - SPLITTER_HEIGHT - 2.0 * ui.spacing().item_spacing.y).max(0.0);
            let merge_height = (usable * state.merge_fraction).round();
            ui.allocate_ui(vec2(total.x, usable - merge_height), |ui| {
                source_columns(ui, &source_style, mirror, state, &mut actions, &mut scrolls)
            });
            if options.overview_strip {
                splitter(ui, palette, usable, merge_height, &mut state.merge_fraction);
                ui.allocate_ui(vec2(total.x, strip_height), |ui| strip_actions(ui, mirror, state, &source_style, &mut actions));
            }
            splitter(ui, palette, usable, merge_height, &mut state.merge_fraction);
            ui.allocate_ui(vec2(total.x, merge_height), |ui| output_pane(ui, &output_style, mirror, state, &mut actions, &mut scrolls));
        }
        let mut groups: Vec<ScrollGroup> = Vec::new();
        for pane in &scrolls {
            if !groups.contains(&pane.group) {
                groups.push(pane.group.clone());
            }
        }
        for group in groups {
            follow_panes(state.scroll_of(group.clone()), group, &scrolls, options.scroll_motion);
        }
        if let Some(leader) = focused_pane(&scrolls, mirror.is_aligned()) {
            state.scroll_leader = Some(leader);
        }
        if mirror.is_aligned() && !state.independent_scroll {
            state.output_scroll = state.source_scroll.clone();
        }
    });
    let moving = |scroll: &SmoothScroll| scroll.offset != scroll.target;
    if moving(&state.source_scroll) || moving(&state.output_scroll) || state.pane_scrolls.iter().any(moving) || state.jump.is_some() {
        ui.ctx().request_repaint();
    }
    actions
}

/// Whether every pane of the session showed all rows of `region` in the last frame. A pane with no viewport yet does not.
fn region_in_view_everywhere(mirror: &Mirror, viewports: &[Option<Viewport>; 4], region: usize) -> bool {
    scroll_groups(mirror, false)
        .into_iter()
        .flat_map(|(_, panes)| panes)
        .all(|(document, rows)| region_in_view(Some(rows), viewports[document_index(document)], region))
}

/// The pane that lines up with the source panes and that the user pressed in, scrolled with the wheel or moved the scroll bar of this frame.
fn focused_pane(scrolls: &[PaneScroll], aligned: bool) -> Option<Document> {
    scrolls
        .iter()
        .filter(|pane| pane.document != Document::D || aligned)
        .find(|pane| pane.pressed || pane.wheel.stepped != 0.0 || pane.wheel.direct != 0.0 || (pane.after - pane.given).abs() > MOVED_BY_PANE)
        .map(|pane| pane.document.clone())
}


/// The panes of the session with their rows, by the scroll offset they follow: while scrolling is independent, every pane on its own; otherwise the source panes, with D when it lines up with them, and D on its own otherwise. Panes without row maps can't be jumped in and are left out.
fn scroll_groups(mirror: &Mirror, independent: bool) -> Vec<(ScrollGroup, Vec<(Document, Rows<'_>)>)> {
    let mut sources: Vec<(Document, Rows<'_>)> = mirror
        .documents()
        .iter()
        .filter_map(|text| mirror.rows(text.document.clone()).map(|rows| (text.document.clone(), Rows::Mapped(rows))))
        .collect();
    let output = mirror.output().map(|_| (Document::D, Rows::Mapped(mirror.output_rows())));
    if independent {
        return sources
            .into_iter()
            .chain(output)
            .map(|(document, rows)| (ScrollGroup::Pane(document.clone()), vec![(document, rows)]))
            .collect();
    }
    let mut groups = Vec::new();
    match output {
        Some(output) if mirror.is_aligned() => sources.push(output),
        Some(output) => groups.push((ScrollGroup::Output, vec![output])),
        None => {}
    }
    groups.insert(0, (ScrollGroup::Sources, sources));
    groups
}

/// Whether `viewport` shows every row of `region`. Without rows for it, there is nothing to scroll to.
fn region_in_view(rows: Option<Rows<'_>>, viewport: Option<Viewport>, region: usize) -> bool {
    match rows.and_then(|rows| rows.region_rows(region)) {
        Some(range) => viewport.is_some_and(|viewport| viewport.shows(range)),
        None => true,
    }
}

/// Where a jump scrolls one group of panes.
#[derive(Debug, Clone, Copy, PartialEq)]
enum JumpTarget {
    /// A pane of the group has not been laid out yet.
    NotReady,
    /// The group has no rows for the region.
    Nowhere,
    Offset {
        offset: f32,
        max: f32,
    },
}

/// Starts scrolling every group of panes to `region`. Returns false, scrolling nothing, while a pane has not been laid out yet.
fn start_jump(mirror: &Mirror, state: &mut PaneState, region: usize, options: &PaneOptions) -> bool {
    let targets: Vec<(ScrollGroup, JumpTarget)> = scroll_groups(mirror, state.independent_scroll.clone())
        .into_iter()
        .map(|(group, panes)| {
            let panes: Vec<(Rows<'_>, Option<Viewport>)> = panes.into_iter().map(|(document, rows)| (rows, state.viewports[document_index(document)])).collect();
            (group, jump_target(&panes, region, options.jump_context_rows))
        })
        .collect();
    if targets.iter().any(|(_, target)| *target == JumpTarget::NotReady) {
        return false;
    }
    for (group, target) in targets {
        if let JumpTarget::Offset { offset, max } = target {
            state.scroll_of(group).scroll_to(offset, max, options.scroll_motion);
        }
    }
    true
}

/// Where a group of panes that scroll together goes for a jump to `region`: as in its shortest pane, which every pane can follow.
fn jump_target(panes: &[(Rows<'_>, Option<Viewport>)], region: usize, context: usize) -> JumpTarget {
    if panes.iter().any(|(_, viewport)| viewport.is_none()) {
        return JumpTarget::NotReady;
    }
    let rows = panes.iter().find_map(|(rows, _)| rows.region_rows(region));
    let shortest = panes
        .iter()
        .filter_map(|(_, viewport)| *viewport)
        .min_by(|left, right| left.height.total_cmp(&right.height));
    match (rows, shortest) {
        (Some(rows), Some(viewport)) => {
            let max = panes
                .iter()
                .filter_map(|(_, viewport)| viewport.map(|viewport| viewport.max_offset()))
                .fold(f32::INFINITY, f32::min);
            JumpTarget::Offset {
                offset: jump_offset(rows, context, viewport),
                max,
            }
        }
        _ => JumpTarget::Nowhere,
    }
}

/// The scroll offset that shows `rows` with `context` rows above them. A pane too short for that keeps fewer, splitting the rows the region leaves free above and below it; a region taller than the pane starts at its top.
pub fn jump_offset(rows: Range<usize>, context: usize, viewport: Viewport) -> f32 {
    let length = rows.len().max(1);
    let context = context.min(viewport.page_rows().saturating_sub(length) / 2);
    rows.start.saturating_sub(context) as f32 * viewport.row_height
}

/// After a frame: a pane of `group` that ended up away from its given offset, because the user dragged its scroll bar or it revealed the caret, takes the group's offset with it; otherwise the wheel over the group's panes scrolls them.
fn follow_panes(scroll: &mut SmoothScroll, group: ScrollGroup, panes: &[PaneScroll], motion: ScrollMotion) {
    let panes: Vec<&PaneScroll> = panes.iter().filter(|pane| pane.group == group).collect();
    if let Some(moved) = panes.iter().find(|pane| (pane.after - pane.given).abs() > MOVED_BY_PANE) {
        scroll.set(moved.after.clone());
        return;
    }
    let stepped: f32 = panes.iter().map(|pane| pane.wheel.stepped).sum();
    let direct: f32 = panes.iter().map(|pane| pane.wheel.direct).sum();
    let max = panes.iter().map(|pane| pane.max).fold(f32::INFINITY, f32::min);
    if stepped != 0.0 {
        scroll.scroll_by(stepped, max.clone(), motion);
    }
    if direct != 0.0 {
        scroll.shift(direct, max);
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

/// The source panes all show the shared scroll offset, or their own while scrolling is independent. Each reports how its scrolling went to `scrolls`.
fn source_columns(ui: &mut Ui, style: &ViewStyle<'_>, mirror: &Mirror, state: &mut PaneState, actions: &mut Vec<PaneAction>, scrolls: &mut Vec<PaneScroll>) {
    let count = mirror.documents().len().max(1);
    ui.columns(count, |columns| {
        for (column, text) in columns.iter_mut().zip(mirror.documents()) {
            let lines = Lines {
                text: &text.text,
                ranges: &text.lines,
            };
            let rows = mirror.rows(text.document.clone()).map_or(Rows::PerLine(lines.count()), Rows::Mapped);
            let char_runs = mirror.char_diffs(text.document.clone()).map(|runs| CharRuns {
                document: text.document.clone(),
                lines: runs,
                sources: mirror.documents().len(),
            });
            let marks: Vec<AlignmentMark> = mirror.alignments().of(text.document.clone()).cloned().collect();
            let highest = mirror.alignments().marks.iter().map(|mark| mark.number).max().unwrap_or(0);
            let group = scroll_group(text.document.clone(), mirror.is_aligned(), state.independent_scroll.clone());
            let given = state.scroll_of(group.clone()).offset.clone();
            scrolls.push(pane(
                column,
                style,
                text.document.clone(),
                text.label.as_deref(),
                Some(&text.path),
                text.eol.clone(),
                false,
                lines,
                rows,
                char_runs,
                PaneMarks {
                    marks: &marks,
                    highest,
                    insets: &mirror.alignments().insets,
                },
                group,
                given,
                state,
                actions,
            ));
        }
    });
}

/// D scrolls on its own, unless it is lined up with the source panes and scrolling is synced: then it shares their offset.
fn output_pane(ui: &mut Ui, style: &ViewStyle<'_>, mirror: &Mirror, state: &mut PaneState, actions: &mut Vec<PaneAction>, scrolls: &mut Vec<PaneScroll>) {
    if let Some(output) = mirror.output() {
        let lines = output.lines();
        let group = scroll_group(Document::D, mirror.is_aligned(), state.independent_scroll.clone());
        let given = state.scroll_of(group.clone()).offset.clone();
        let rows = Rows::Mapped(mirror.output_rows());
        scrolls.push(pane(
            ui,
            style,
            Document::D,
            None,
            output.path.as_deref(),
            None,
            output.modified,
            lines,
            rows,
            None,
            PaneMarks {
                marks: &[],
                highest: 0,
                insets: &[],
            },
            group,
            given,
            state,
            actions,
        ));
    }
}

/// Shows one pane at the vertical scroll offset `given`, and reports how its scrolling went.
fn pane(
    ui: &mut Ui,
    style: &ViewStyle<'_>,
    document: Document,
    label: Option<&str>,
    path: Option<&Path>,
    eol: Option<EolStyle>,
    modified: bool,
    lines: Lines<'_>,
    rows: Rows<'_>,
    char_runs: Option<CharRuns<'_>>,
    marks: PaneMarks<'_>,
    group: ScrollGroup,
    given: f32,
    state: &mut PaneState,
    actions: &mut Vec<PaneAction>,
) -> PaneScroll {
    let palette = style.palette;
    let radius = theme::corner_radius_of(ui);
    theme::pane_frame(palette, radius)
        .show(ui, |ui| {
            header(ui, palette, radius, document.clone(), label, path, eol, modified, state, actions);
            theme::pane_body_frame()
                .show(ui, |ui| {
                    let caret = state.visible_caret(document.clone());
                    let find = Some(&state.find)
                        .filter(|find| find.open && find.document == Some(document))
                        .map(|find| FindHits { matches: &find.matches, current: find.current });
                    let response = text_view::show(
                        ui,
                        document.letter(),
                        lines,
                        rows,
                        char_runs,
                        caret,
                        state.reveal_caret && caret.is_some(),
                        Some(given.clone()),
                        style,
                        marks,
                        find,
                    );
                    if state.caret_in(document.clone()).is_some() {
                        state.page_rows = response.page_rows;
                    }
                    state.viewports[document_index(document.clone())] = Some(response.viewport);
                    match response.pointer {
                        Some(PointerAction::Press { position, void_row, extend }) => actions.push(PaneAction::Press {
                            document: document.clone(),
                            position,
                            void_row,
                            extend,
                        }),
                        Some(PointerAction::Drag { position, void_row }) => actions.push(PaneAction::Drag {
                            document: document.clone(),
                            position,
                            void_row,
                        }),
                        Some(PointerAction::SelectWord(position)) => actions.push(PaneAction::SelectWord(document.clone(), position)),
                        Some(PointerAction::SelectLine(line)) => actions.push(PaneAction::SelectLine(document.clone(), line)),
                        Some(PointerAction::SelectRegion(region)) => actions.push(PaneAction::SelectRegion(document.clone(), region)),
                        Some(PointerAction::SetMarkLines(number, start, end)) => actions.push(PaneAction::SetAlignmentMarkLines(document.clone(), number, start..end)),
                        None => {}
                    }
                    match response.menu {
                        Some(MenuAction::Copy) => actions.push(PaneAction::Copy),
                        Some(MenuAction::MarkSelection) => actions.push(PaneAction::MarkSelection(document.clone())),
                        Some(MenuAction::MarkAnchor(position)) => actions.push(PaneAction::MarkAnchor(document.clone(), position)),
                        Some(MenuAction::RemoveMark(number)) => actions.push(PaneAction::RemoveAlignmentMark(document.clone(), number)),
                        Some(MenuAction::RenumberMark(number, to)) => actions.push(PaneAction::RenumberAlignmentMark(document.clone(), number, to)),
                        Some(MenuAction::PlaceMark(number, slot)) => actions.push(PaneAction::PlaceAlignment(number, slot)),
                        None => {}
                    }
                    PaneScroll {
                        document: document.clone(),
                        pressed: matches!(response.pointer, Some(PointerAction::Press { .. })),
                        group,
                        given,
                        after: response.viewport.offset,
                        wheel: response.wheel,
                        max: response.viewport.max_offset(),
                    }
                })
                .inner
        })
        .inner
}

/// Letter, label, editable path and Browse button. The field shows the document's path unless the user is editing it.
fn header(
    ui: &mut Ui,
    palette: &Palette,
    radius: u8,
    document: Document,
    label: Option<&str>,
    path: Option<&Path>,
    eol: Option<EolStyle>,
    modified: bool,
    state: &mut PaneState,
    actions: &mut Vec<PaneAction>,
) {
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
                if let Some(eol) = eol {
                    ui.label(RichText::new(eol::name(eol)).color(palette.weak_text)).on_hover_text("Line ending style of this document");
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

    /// The header of A over a path, drawn in a window 500 wide.
    fn header_harness(eol: Option<EolStyle>) -> egui_kittest::Harness<'static> {
        let mut state = PaneState::default();
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(500.0, 100.0)).with_accessibility_check(false).build_ui(move |ui| {
            theme::apply_style(ui.ctx(), &crate::config::ViewConfig::default());
            let palette = theme::palette_of(ui);
            let mut actions = Vec::new();
            header(ui, &palette, 4, Document::A, None, Some(Path::new("a.txt")), eol.clone(), false, &mut state, &mut actions);
        });
        harness.run();
        harness
    }

    #[test]
    fn the_header_names_the_line_ending_style_beside_browse() {
        use egui_kittest::kittest::Queryable;
        for (style, name) in [(EolStyle::Lf, "LF"), (EolStyle::Cr, "CR"), (EolStyle::CrLf, "CRLF")] {
            let harness = header_harness(Some(style));
            let label = harness.get_by_label(name).rect();
            let browse = harness.get_by_label("Browse...").rect();
            assert!(label.right() <= browse.left(), "{name} sits left of Browse");
            assert!(label.left() > 250.0, "{name} sits at the right end, next to Browse, not at {}", label.left());
        }
    }

    #[test]
    fn a_document_without_a_line_ending_style_shows_no_label() {
        use egui_kittest::kittest::Queryable;
        let harness = header_harness(None);
        harness.get_by_label("Browse...");
        for name in ["LF", "CR", "CRLF"] {
            assert!(harness.query_by_label(name).is_none(), "no {name} label");
        }
    }

    #[test]
    fn caret_belongs_to_one_document() {
        let mut state = PaneState::default();
        assert_eq!(state.caret_in(Document::A), None);
        let cursor = Cursor {
            anchor: Position { line: 0, column: 0 },
            head: Position { line: 2, column: 1 },
        };
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
    fn jumps_take_the_keyboard_back_from_the_strip() {
        let mut state = PaneState::default();
        state.strip_focused = true;
        state.set_caret(Document::A, Cursor::at(Position { line: 1, column: 0 }), None);
        assert!(state.strip_focused, "moving the caret keeps the strip focused");
        state.set_hidden_caret(Document::A, Cursor::at(Position { line: 4, column: 0 }), None);
        assert!(!state.strip_focused);
    }

    #[test]
    fn region_counts_as_in_view_when_all_its_rows_are() {
        let comparison = Comparison {
            regions: vec![
                Region {
                    hunks: vec![0..10, 0..10],
                    status: RegionStatus::Diff(HunkChange::Unchanged),
                    rows: (0..10)
                        .map(|line| Row {
                            lines: vec![Some(line), Some(line)],
                        })
                        .collect(),
                },
                Region {
                    hunks: vec![10..12, 10..12],
                    status: RegionStatus::Diff(HunkChange::Modified),
                    rows: (10..12)
                        .map(|line| Row {
                            lines: vec![Some(line), Some(line)],
                        })
                        .collect(),
                },
            ],
        };
        let map = RowMap::for_source(&comparison, &[], 0, 12, None, None);
        let rows = Some(Rows::Mapped(&map));
        let viewport = |offset: f32| {
            Some(Viewport {
                offset,
                height: 50.0,
                row_height: 10.0,
                content_height: 120.0,
            })
        };
        assert!(region_in_view(rows, viewport(80.0), 1));
        assert!(region_in_view(rows, viewport(70.0), 1));
        assert!(!region_in_view(rows, viewport(69.0), 1));
        assert!(!region_in_view(rows, viewport(101.0), 1));
        assert!(!region_in_view(rows, viewport(0.0), 1));
        assert!(!region_in_view(rows, None, 1));
        assert!(region_in_view(rows, None, 5));
        assert!(region_in_view(None, None, 1));
    }

    const INSTANT: ScrollMotion = ScrollMotion { smooth: false, duration: 0.0 };

    fn scroll(group: ScrollGroup, given: f32, after: f32, wheel: f32) -> PaneScroll {
        PaneScroll {
            document: Document::A,
            pressed: false,
            group,
            given,
            after,
            wheel: Wheel { stepped: wheel, direct: 0.0 },
            max: 500.0,
        }
    }

    #[test]
    fn shared_offset_follows_the_pane_that_moved() {
        let mut shared = SmoothScroll { offset: 10.0, target: 10.0 };
        follow_panes(
            &mut shared,
            ScrollGroup::Sources,
            &[scroll(ScrollGroup::Sources, 10.0, 10.0, 0.0), scroll(ScrollGroup::Sources, 10.0, 42.0, 0.0)],
            INSTANT,
        );
        assert_eq!(shared, SmoothScroll { offset: 42.0, target: 42.0 });
        follow_panes(&mut shared, ScrollGroup::Sources, &[scroll(ScrollGroup::Output, 42.0, 7.0, 0.0)], INSTANT);
        assert_eq!(shared.offset, 42.0);
        follow_panes(&mut shared, ScrollGroup::Sources, &[], INSTANT);
        assert_eq!(shared.offset, 42.0);
    }

    #[test]
    fn wheel_over_any_pane_scrolls_its_group() {
        let mut shared = SmoothScroll::default();
        let panes = [
            scroll(ScrollGroup::Sources, 0.0, 0.0, 0.0),
            scroll(ScrollGroup::Sources, 0.0, 0.0, 45.0),
            scroll(ScrollGroup::Output, 0.0, 0.0, 900.0),
        ];
        follow_panes(&mut shared, ScrollGroup::Sources, &panes, INSTANT);
        assert_eq!(shared.offset, 45.0);
        let mut output = SmoothScroll::default();
        follow_panes(&mut output, ScrollGroup::Output, &panes, INSTANT);
        assert_eq!(output.offset, 500.0);
    }

    #[test]
    fn trackpad_scrolling_is_never_animated() {
        let smooth = ScrollMotion { smooth: true, duration: 0.2 };
        let mut shared = SmoothScroll::default();
        let pane = PaneScroll {
            document: Document::A,
            pressed: false,
            group: ScrollGroup::Sources,
            given: 0.0,
            after: 0.0,
            wheel: Wheel { stepped: 0.0, direct: 12.0 },
            max: 500.0,
        };
        follow_panes(&mut shared, ScrollGroup::Sources, &[pane], smooth);
        assert_eq!(shared, SmoothScroll { offset: 12.0, target: 12.0 });
    }

    #[test]
    fn panes_follow_their_own_offset_only_while_independent() {
        assert_eq!(scroll_group(Document::B, false, false), ScrollGroup::Sources);
        assert_eq!(scroll_group(Document::D, true, false), ScrollGroup::Sources);
        assert_eq!(scroll_group(Document::D, false, false), ScrollGroup::Output);
        assert_eq!(scroll_group(Document::B, true, true), ScrollGroup::Pane(Document::B));
        assert_eq!(scroll_group(Document::D, false, true), ScrollGroup::Pane(Document::D));
    }

    #[test]
    fn independent_panes_scroll_only_with_their_own_wheel() {
        let panes = [
            scroll(ScrollGroup::Pane(Document::A), 0.0, 0.0, 0.0),
            scroll(ScrollGroup::Pane(Document::B), 0.0, 0.0, 45.0),
        ];
        let mut a = SmoothScroll::default();
        let mut b = SmoothScroll::default();
        follow_panes(&mut a, ScrollGroup::Pane(Document::A), &panes, INSTANT);
        follow_panes(&mut b, ScrollGroup::Pane(Document::B), &panes, INSTANT);
        assert_eq!(a.offset, 0.0);
        assert_eq!(b.offset, 45.0);
    }

    fn at(offset: f32) -> SmoothScroll {
        SmoothScroll { offset, target: offset }
    }

    #[test]
    fn going_independent_starts_every_pane_where_it_was() {
        let mut state = PaneState {
            source_scroll: at(30.0),
            output_scroll: at(70.0),
            ..PaneState::default()
        };
        state.set_independent_scroll(true, false);
        assert!(state.independent_scroll);
        assert_eq!(state.pane_scrolls, [at(30.0), at(30.0), at(30.0), at(70.0)]);
        state.pane_scrolls = Default::default();
        state.set_independent_scroll(true, false);
        assert_eq!(state.pane_scrolls[0], at(0.0), "staying independent changes nothing");
        let mut aligned = PaneState {
            source_scroll: at(30.0),
            output_scroll: at(70.0),
            ..PaneState::default()
        };
        aligned.set_independent_scroll(true, true);
        assert_eq!(aligned.pane_scrolls[3], at(30.0), "a lined-up D starts with the source panes");
    }

    #[test]
    fn syncing_again_follows_the_last_focused_pane() {
        let mut state = PaneState {
            independent_scroll: true,
            pane_scrolls: [at(10.0), at(20.0), at(30.0), at(40.0)],
            scroll_leader: Some(Document::B),
            ..PaneState::default()
        };
        state.set_independent_scroll(false, false);
        assert!(!state.independent_scroll);
        assert_eq!(state.source_scroll, at(20.0));
        assert_eq!(state.output_scroll, at(40.0), "D on its own keeps its offset");
        let mut aligned = PaneState {
            independent_scroll: true,
            pane_scrolls: [at(10.0), at(20.0), at(30.0), at(40.0)],
            scroll_leader: Some(Document::D),
            ..PaneState::default()
        };
        aligned.set_independent_scroll(false, true);
        assert_eq!(aligned.source_scroll, at(40.0), "a lined-up D can lead");
        assert_eq!(aligned.output_scroll, at(40.0));
    }

    #[test]
    fn syncing_without_a_leader_that_lines_up_follows_a() {
        for leader in [None, Some(Document::D)] {
            let mut state = PaneState {
                independent_scroll: true,
                pane_scrolls: [at(10.0), at(20.0), at(30.0), at(40.0)],
                scroll_leader: leader,
                ..PaneState::default()
            };
            state.set_independent_scroll(false, false);
            assert_eq!(state.source_scroll, at(10.0));
        }
    }

    #[test]
    fn the_minimap_moves_the_leader_while_independent() {
        let mut state = PaneState {
            source_scroll: at(5.0),
            pane_scrolls: [at(10.0), at(20.0), at(30.0), at(40.0)],
            scroll_leader: Some(Document::C),
            ..PaneState::default()
        };
        let (index, scroll) = state.map_scroll(false);
        assert_eq!((index, scroll.clone()), (0, at(5.0)));
        state.independent_scroll = true;
        let (index, scroll) = state.map_scroll(false);
        assert_eq!(index, 2);
        scroll.set(99.0);
        assert_eq!(state.pane_scrolls[2], at(99.0));
    }

    #[test]
    fn a_pane_counts_as_focused_when_pressed_wheeled_or_dragged() {
        let pane = |document: Document, pressed: bool, after: f32, wheel: f32| PaneScroll {
            document,
            pressed,
            ..scroll(ScrollGroup::Pane(document), 10.0, after, wheel)
        };
        assert_eq!(focused_pane(&[pane(Document::A, false, 10.0, 0.0), pane(Document::B, false, 10.0, 0.0)], true), None);
        assert_eq!(focused_pane(&[pane(Document::A, false, 10.0, 0.0), pane(Document::B, true, 10.0, 0.0)], true), Some(Document::B));
        assert_eq!(focused_pane(&[pane(Document::C, false, 10.0, 30.0)], true), Some(Document::C));
        assert_eq!(focused_pane(&[pane(Document::B, false, 50.0, 0.0)], true), Some(Document::B));
        assert_eq!(focused_pane(&[pane(Document::D, true, 10.0, 0.0)], true), Some(Document::D));
        assert_eq!(focused_pane(&[pane(Document::D, true, 10.0, 0.0)], false), None, "D on its own can't lead the source panes");
    }

    #[test]
    fn jumps_keep_context_rows_above_when_the_pane_has_room() {
        let tall = Viewport {
            offset: 0.0,
            height: 400.0,
            row_height: 10.0,
            content_height: 2000.0,
        };
        assert_eq!(jump_offset(30..32, 12, tall), 180.0);
        assert_eq!(jump_offset(5..6, 12, tall), 0.0);
        assert_eq!(jump_offset(30..32, 0, tall), 300.0);
    }

    // Regression: with 12 context rows a region landed below the bottom of a short merge pane, so the jump missed it.
    #[test]
    fn jumps_in_short_panes_keep_the_region_in_view() {
        let short = Viewport {
            offset: 0.0,
            height: 80.0,
            row_height: 10.0,
            content_height: 2000.0,
        };
        let offset = jump_offset(30..32, 12, short);
        assert_eq!(offset, 270.0);
        assert!(Viewport { offset, ..short }.shows(30..32));
        let one_row = jump_offset(10..11, 12, short);
        assert!(Viewport { offset: one_row, ..short }.shows(10..11));
        assert_eq!(jump_offset(30..50, 12, short), 300.0);
        assert_eq!(jump_offset(30..30, 12, short), 270.0);
    }

    fn two_region_map() -> RowMap {
        let comparison = Comparison {
            regions: vec![
                Region {
                    hunks: vec![0..30, 0..30],
                    status: RegionStatus::Diff(HunkChange::Unchanged),
                    rows: (0..30)
                        .map(|line| Row {
                            lines: vec![Some(line), Some(line)],
                        })
                        .collect(),
                },
                Region {
                    hunks: vec![30..32, 30..32],
                    status: RegionStatus::Diff(HunkChange::Modified),
                    rows: (30..32)
                        .map(|line| Row {
                            lines: vec![Some(line), Some(line)],
                        })
                        .collect(),
                },
            ],
        };
        RowMap::for_source(&comparison, &[], 0, 32, None, None)
    }

    #[test]
    fn group_jumps_follow_their_shortest_pane() {
        let map = two_region_map();
        let tall = Viewport {
            offset: 0.0,
            height: 400.0,
            row_height: 10.0,
            content_height: 320.0,
        };
        let short = Viewport {
            height: 80.0,
            content_height: 320.0,
            ..tall
        };
        assert_eq!(
            jump_target(&[(Rows::Mapped(&map), Some(tall)), (Rows::Mapped(&map), Some(short))], 1, 12),
            JumpTarget::Offset { offset: 270.0, max: 0.0 }
        );
        assert_eq!(jump_target(&[(Rows::Mapped(&map), Some(short)), (Rows::Mapped(&map), None)], 1, 12), JumpTarget::NotReady);
        assert_eq!(jump_target(&[(Rows::Mapped(&map), Some(short))], 9, 12), JumpTarget::Nowhere);
        assert_eq!(jump_target(&[(Rows::PerLine(32), Some(short))], 1, 12), JumpTarget::Nowhere);
    }

    #[test]
    fn merge_pane_starts_at_half_height() {
        assert_eq!(PaneState::default().merge_fraction, 0.5);
    }
}
