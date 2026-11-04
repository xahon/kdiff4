use std::collections::HashMap;
use std::path::{Path, PathBuf};

use egui::{Key, KeyboardShortcut, Modifiers, RichText};
use kdiff4_protocol::{Change, Client, Command, Document, Event, HostMessage, Navigation, RequestId, Response};

use crate::Startup;
use crate::char_diff::CharDiffLook;
use crate::config::{self, GuiConfig, ViewConfig};
use crate::editing::{self, Cursor, EditKey, Lines, Motion, Position};
use crate::fonts::{self, FontScan};
use crate::mirror::{DeltaOutcome, Mirror};
use crate::open_dialog::{self, FormAction, OpenForm};
use crate::panes::{self, PaneAction, PaneOptions, PaneState};
use crate::paths;
use crate::rows::{self, Rows};
use crate::scroll::ScrollMotion;
use crate::settings_dialog::{SettingsAction, SettingsDialog};
use crate::text_view::{self, Caret};
use crate::theme;
use crate::window_frame;


const OPEN_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const SAVE_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const SAVE_AS_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
const UNDO_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
const REDO_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
const REDO_ALT_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
const EXIT_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Q);
const SETTINGS_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma);
/// Toggle A, B or C in the region with the caret (kdiff3: Ctrl+1, Ctrl+2, Ctrl+3).
const CHOOSE_SHORTCUTS: [(Document, KeyboardShortcut); 3] = [
    (Document::A, KeyboardShortcut::new(Modifiers::COMMAND, Key::Num1)),
    (Document::B, KeyboardShortcut::new(Modifiers::COMMAND, Key::Num2)),
    (Document::C, KeyboardShortcut::new(Modifiers::COMMAND, Key::Num3)),
];
/// Ctrl+Up and Ctrl+Down go to the previous and next difference, Ctrl+PageUp and Ctrl+PageDown to the previous and next conflict.
const NAVIGATION_SHORTCUTS: [(Navigation, &str, KeyboardShortcut); 4] = [
    (Navigation::PreviousDifference, "Previous Difference", KeyboardShortcut::new(Modifiers::COMMAND, Key::ArrowUp)),
    (Navigation::NextDifference, "Next Difference", KeyboardShortcut::new(Modifiers::COMMAND, Key::ArrowDown)),
    (Navigation::PreviousConflict, "Previous Conflict", KeyboardShortcut::new(Modifiers::COMMAND, Key::PageUp)),
    (Navigation::NextConflict, "Next Conflict", KeyboardShortcut::new(Modifiers::COMMAND, Key::PageDown)),
];
const VOID_TOGGLE_LABEL: &str = "Void Space";
const VOID_TOGGLE_HINT: &str = "Toggle Void Space visibility in Output Pane";
const OVERVIEW_TOGGLE_LABEL: &str = "Overview Strip";
const OVERVIEW_TOGGLE_HINT: &str = "Toggle the Overview Strip: the current line of every document, with its character diff";
const TOOLBAR_ICON_SIZE: f32 = 18.0;
/// Space between a toolbar button's icon and its label.
const TOOLBAR_ICON_GAP: f32 = 2.0;
const UNSAVABLE_HINT: &str = "Resolve every conflict before saving";
pub const INITIAL_WINDOW_SIZE: [f32; 2] = [640.0, 190.0];
/// The startup window grows to this while the settings dialog is open, so the dialog fits.
const SETTINGS_WINDOW_SIZE: [f32; 2] = [860.0, 660.0];
const SESSION_WINDOW_SIZE: [f32; 2] = [1280.0, 800.0];


/// What a request in flight was for, so its response can be reported.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    Open,
    Edit,
    LoadSource(Document),
    SetOutputPath,
    Save,
    Resync,
    /// Undo or redo; its delta moves the caret to the changed text.
    UndoRedo,
    /// A choice for one region, which its delta keeps the caret in, or for every unresolved one.
    Choose(Option<usize>),
    /// A move to the next or previous difference or conflict; becomes `Navigated` when the current region moved.
    Navigate(Navigation),
    Navigated,
    /// A click on a gutter: makes its region current and scrolls there like a jump, keeping the caret where the click put it.

    SelectRegion,
    ComparisonOptions,
}


/// A message under the panes: the differences found on open and reload, or the outcome of the last save, reload or path change.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Status {
    text: String,
    is_error: bool,
}


pub struct KdiffApp {
    client: Client,
    config_path: Option<PathBuf>,
    config: GuiConfig,
    /// False when the config file failed to load, so the broken file is not overwritten.
    config_writable: bool,
    form: OpenForm,
    form_open: bool,
    /// The Open window's error.
    error: Option<String>,
    status: Option<Status>,
    engine_stopped: bool,
    mirror: Option<Mirror>,
    panes: PaneState,
    pending: HashMap<RequestId, Pending>,
    window_enlarged: bool,
    settings: Option<SettingsDialog>,
    font_scan: FontScan,
    /// The view settings currently applied to the egui context: the saved ones, or the settings dialog's draft.
    applied_view: Option<ViewConfig>,
    /// The corner radius last applied to the native window frame.
    frame_radius: Option<u8>,
    /// Whether the title bar theme was re-sent after the window first got focus.
    title_bar_refreshed: bool,
}


impl KdiffApp {
    pub fn new(ctx: &egui::Context, client: Client, startup: Startup) -> KdiffApp {
        let repaint_ctx = ctx.clone();
        client.set_waker(Box::new(move || repaint_ctx.request_repaint()));
        let config_path = config::default_config_path();
        let (config, config_writable) = match config_path.as_deref() {
            Some(path) => match config::load(path) {
                Ok(config) => (config, true),
                Err(error) => {
                    log::error!("{error}");
                    (GuiConfig::default(), false)
                }
            },
            None => {
                log::warn!("no per-user config directory; GUI settings won't be saved");
                (GuiConfig::default(), false)
            }
        };
        let mut form = OpenForm::default();
        for (document, path) in &startup.paths {
            form.set_path(document.clone(), path);
        }
        form.set_labels(startup.labels.clone());
        let mut pending = HashMap::new();
        if let Some(id) = startup.pending_open {
            pending.insert(id, Pending::Open);
        }
        let view = config.view.clone();
        let mut app = KdiffApp { client, config_path, config, config_writable, form, form_open: true, error: None, status: None, engine_stopped: false, mirror: None, panes: PaneState::default(), pending, window_enlarged: false, settings: None, font_scan: FontScan::default(), applied_view: None, frame_radius: None, title_bar_refreshed: false };
        app.apply_view(ctx, &view);
        app
    }


    /// Applies the theme, fonts and styles; reloads the font only when the family changed.
    fn apply_view(&mut self, ctx: &egui::Context, view: &ViewConfig) {
        if self.applied_view.as_ref().is_none_or(|applied| applied.theme != view.theme) {
            ctx.set_theme(theme::preference(view.theme.clone()));
            self.title_bar_refreshed = false;
        }
        let family_changed = self.applied_view.as_ref().is_none_or(|applied| applied.font_family != view.font_family);
        if family_changed {
            let data = view.font_family.as_deref().and_then(|name| self.find_font(name));
            ctx.set_fonts(fonts::font_definitions(data));
        }
        theme::apply_style(ctx, view);
        self.applied_view = Some(view.clone());
    }


    fn find_font(&mut self, name: &str) -> Option<egui::FontData> {
        let scanned = self.font_scan
            .families()
            .and_then(|families| families.iter().find(|family| family.name == name))
            .map(|family| family.data.clone());
        let found = scanned.or_else(|| fonts::load_family(name));
        if found.is_none() {
            log::warn!("font family {name:?} is not installed; using the built-in font");
        }
        found
    }


    fn open_settings(&mut self, ctx: &egui::Context) {
        if self.settings.is_none() {
            self.font_scan.start(ctx);
            let options = self.mirror.as_ref().map(Mirror::options);
            self.settings = Some(SettingsDialog::new(&self.config.view, options));
            if !self.window_enlarged {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(SETTINGS_WINDOW_SIZE.into()));
            }
        }
    }


    fn close_settings(&mut self, ctx: &egui::Context, action: SettingsAction) {
        let dialog = match self.settings.take() {
            Some(dialog) => dialog,
            None => return,
        };
        match action {
            SettingsAction::Ok => {
                self.config.view = dialog.draft().clone();
                self.apply_view(ctx, dialog.draft());
                self.save_config(true);
                if let Some(options) = dialog.changed_comparison() {
                    self.send(Command::SetComparisonOptions(options.clone()), Pending::ComparisonOptions);
                }
            }
            SettingsAction::Cancel | SettingsAction::OpenConfigFile => self.apply_view(ctx, dialog.original()),
        }
        if !self.window_enlarged {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(INITIAL_WINDOW_SIZE.into()));
        }
    }


    /// Writes the file first if it doesn't exist yet, so the editor has something to open.
    fn open_config_file(&mut self) {
        let path = match self.config_path.clone() {
            Some(path) => path,
            None => {
                self.report_error("Cannot open the settings file: no config directory".to_string());
                return;
            }
        };
        if !path.exists() {
            self.save_config(false);
        }
        if let Err(error) = config::open_command(&path).spawn() {
            self.report_error(format!("Cannot open {}: {error}", path.display()));
        }
    }


    /// `from_settings` is an explicit save by the user: a config file that failed to load is then backed up and replaced.
    fn save_config(&mut self, from_settings: bool) {
        let path = match self.config_path.as_deref() {
            Some(path) => path,
            None => return,
        };
        if !self.config_writable {
            if !from_settings {
                return;
            }
            if let Err(error) = config::back_up_broken(path) {
                log::error!("{error}");
                return;
            }
            self.config_writable = true;
        }
        if let Err(error) = config::save(path, &self.config) {
            log::error!("{error}");
        }
    }


    /// Sends a command and remembers what it was for. Returns `None` if the engine is gone.
    fn send(&mut self, command: Command, kind: Pending) -> Option<RequestId> {
        match self.client.send(command) {
            Ok(id) => {
                self.pending.insert(id.clone(), kind);
                Some(id)
            }
            Err(error) => {
                self.report_error(format!("Cannot reach the engine: {error}"));
                None
            }
        }
    }


    fn report_error(&mut self, text: String) {
        log::error!("{text}");
        if self.mirror.is_some() {
            self.status = Some(Status { text, is_error: true });
        } else {
            self.error = Some(text);
        }
    }


    fn is_opening(&self) -> bool {
        self.pending
            .values()
            .any(|kind| *kind == Pending::Open)
    }


    fn request_resync(&mut self) {
        if let Some(mirror) = self.mirror.as_mut() {
            mirror.resync_requested();
        }
        self.send(Command::Resync, Pending::Resync);
    }


    fn drain_messages(&mut self, ctx: &egui::Context) {
        while !self.engine_stopped {
            match self.client.try_recv() {
                Ok(Some(message)) => self.handle_message(ctx, message),
                Ok(None) => break,
                Err(error) => {
                    self.engine_stopped = true;
                    self.pending.clear();
                    self.report_error(format!("The engine stopped: {error}"));
                }
            }
        }
    }


    fn handle_message(&mut self, ctx: &egui::Context, message: HostMessage) {
        match message {
            HostMessage::Event(Event::Snapshot(snapshot)) => {
                if self.is_opening() {
                    self.panes = PaneState { merge_fraction: self.panes.merge_fraction.clone(), ..PaneState::default() };
                    self.status = None;
                }
                self.mirror = Some(Mirror::from_snapshot(snapshot, self.view().align_output));
                self.panes.void_row = None;
                self.form_open = false;
                self.error = None;
                if !self.window_enlarged {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(SESSION_WINDOW_SIZE.into()));
                    self.window_enlarged = true;
                }
            }
            HostMessage::Event(Event::Delta(delta)) => {
                let from_undo = delta.cause.as_ref().is_some_and(|cause| self.pending.get(cause) == Some(&Pending::UndoRedo));
                let chosen_region = match delta.cause.as_ref().and_then(|cause| self.pending.get(cause)) {
                    Some(Pending::Choose(region)) => region.clone(),
                    _ => None,
                };
                let changed_end = match &delta.change {
                    Change::OutputEdited { range, text, .. } if from_undo => Some(range.start + text.len()),
                    _ => None,
                };
                let outcome = match self.mirror.as_mut() {
                    Some(mirror) => mirror.apply_delta(delta),
                    None => DeltaOutcome::Ignored,
                };
                match outcome {
                    DeltaOutcome::NeedsResync => {
                        log::warn!("the session view fell out of step with the engine; resyncing");
                        self.request_resync();
                    }
                    DeltaOutcome::Applied => {
                        if let Some(offset) = changed_end
                            && let Some(lines) = self.mirror.as_ref().and_then(|mirror| lines_of(mirror, Document::D))
                        {
                            self.panes.set_caret(Document::D, Cursor::at(editing::position_of(lines, offset)), None);
                            self.panes.reveal_caret = true;
                        }
                        if let Some(region) = chosen_region {
                            self.keep_caret_in_region(region);
                        }
                    }
                    DeltaOutcome::Ignored => {}
                }
            }
            HostMessage::Event(Event::CurrentRegion(current)) => {
                let applied = self.mirror.as_mut().is_some_and(|mirror| mirror.apply_current_region(&current));
                let selected = current.cause.as_ref().is_some_and(|cause| self.pending.get(cause) == Some(&Pending::SelectRegion));
                if !selected
                    && let Some(cause) = current.cause.as_ref()
                    && let Some(kind) = self.pending.get_mut(cause)
                {
                    *kind = Pending::Navigated;
                }
                if applied && let Some(region) = current.region {
                    if selected {
                        self.panes.jump = Some(region);
                    } else {
                        self.jump_to(region);
                    }
                }
            }
            HostMessage::Response(Response { id, result }) => self.handle_response(id, result),
        }
    }


    /// Scrolls every pane to `region`, with context rows above it, unless every pane already shows all of it, and puts the caret, hidden, at its start in the pane that has it (A when none does).
    fn jump_to(&mut self, region: usize) {
        self.panes.jump = Some(region);
        let document = self.panes.caret.map_or(Document::A, |(document, _)| document);
        self.caret_to_region(document, region, true);
    }


    /// Puts the caret at the start of `region` in `document`; `hidden` keeps it from being drawn until it moves again.
    fn caret_to_region(&mut self, document: Document, region: usize, hidden: bool) {
        let target = self.mirror
            .as_ref()
            .and_then(|mirror| region_start_caret(mirror, document.clone(), region));
        if let Some((cursor, void_row)) = target {
            if hidden {
                self.panes.set_hidden_caret(document, cursor, void_row);
            } else {
                self.panes.set_caret(document, cursor, void_row);
            }
        }
    }


    /// A choice adds or removes the region's output lines, so the caret's line may now belong to a neighbor: puts it back at the region's start, keeping it hidden if it was.
    fn keep_caret_in_region(&mut self, region: usize) {
        let Some((document, _)) = self.panes.caret.clone() else { return };
        let hidden = self.panes.caret_hidden;
        self.caret_to_region(document, region, hidden);
    }


    fn handle_response(&mut self, id: RequestId, result: Result<(), kdiff4_protocol::ProtocolError>) {
        let kind = match self.pending.remove(&id) {
            Some(kind) => kind,
            None => return,
        };
        let error = match result {
            Ok(()) => {
                self.status = match kind {
                    Pending::Save => self.mirror
                        .as_ref()
                        .and_then(Mirror::output)
                        .and_then(|output| output.path.as_ref())
                        .map(|path| {
                            log::info!("saved {}", path.display());
                            Status { text: format!("Saved {}", path.display()), is_error: false }
                        }),
                    Pending::Edit | Pending::Resync | Pending::UndoRedo | Pending::Navigated | Pending::SelectRegion => self.status.take(),
                    Pending::Navigate(to) => Some(Status { text: format!("No {}", navigation_noun(to)), is_error: false }),
                    Pending::Open | Pending::LoadSource(_) | Pending::Choose(_) | Pending::ComparisonOptions => self.mirror.as_ref().map(|mirror| Status { text: mirror.difference_summary(), is_error: false }),
                    Pending::SetOutputPath => None,
                };
                return;
            }
            Err(error) => error,
        };
        match kind {
            Pending::Open => self.error = Some(error.message),
            Pending::Edit => {
                let rejected = self.mirror.as_ref().is_some_and(|mirror| mirror.edit_rejected(&id));
                self.report_error(format!("The engine rejected an edit: {}", error.message));
                if rejected {
                    self.request_resync();
                }
            }
            Pending::LoadSource(document) => self.report_error(format!("Cannot load {}: {}", document.letter(), error.message)),
            Pending::SetOutputPath => self.report_error(format!("Cannot change the output path: {}", error.message)),
            Pending::Save => self.report_error(format!("Cannot save: {}", error.message)),
            Pending::Choose(_) => self.report_error(format!("Cannot choose: {}", error.message)),
            Pending::Resync => self.report_error(format!("Cannot resync with the engine: {}", error.message)),
            Pending::UndoRedo => self.report_error(format!("Cannot undo or redo: {}", error.message)),
            Pending::Navigate(_) | Pending::Navigated => self.report_error(format!("Cannot navigate: {}", error.message)),
            Pending::SelectRegion => self.report_error(format!("Cannot select the region: {}", error.message)),
            Pending::ComparisonOptions => self.report_error(format!("Cannot apply the comparison options: {}", error.message)),
        }
    }


    fn handle_form_action(&mut self, action: FormAction) {
        match action {
            FormAction::Browse(document) => {
                let picked = open_dialog::pick_file(document, self.config.recent.last_dir.as_deref());
                if let Some(path) = picked {
                    self.form.set_path(document, &path);
                    self.remember_dir(&path);
                }
            }
            FormAction::Run => self.run(),
            FormAction::Cancel => self.form_open = false,
        }
    }


    fn run(&mut self) {
        let files = match self.form.files() {
            Some(files) => files,
            None => return,
        };
        if open_dialog::needs_overwrite_confirmation(&files)
            && let Some(output) = files.output.as_deref()
            && !open_dialog::confirm_overwrite(output)
        {
            return;
        }
        self.error = None;
        self.send(Command::Open(files), Pending::Open);
    }


    fn remember_dir(&mut self, picked: &Path) {
        let dir = open_dialog::remembered_dir(picked);
        if dir.is_none() || dir == self.config.recent.last_dir {
            return;
        }
        self.config.recent.last_dir = dir;
        self.save_config(false);
    }


    fn is_merge(&self) -> bool {
        self.mirror.as_ref().is_some_and(Mirror::is_merge)
    }


    /// Without an output path, Save asks for one like Save As.
    fn save(&mut self) {
        let has_path = self.mirror
            .as_ref()
            .and_then(Mirror::output)
            .is_some_and(|output| output.path.is_some());
        if has_path {
            self.send(Command::Save, Pending::Save);
        } else {
            self.save_as();
        }
    }


    /// The native Save dialog asks before overwriting an existing file.
    fn save_as(&mut self) {
        if !self.is_merge() {
            return;
        }
        if let Some(path) = open_dialog::pick_file(Document::D, self.config.recent.last_dir.as_deref()) {
            self.remember_dir(&path);
            self.send(Command::SaveAs(path), Pending::Save);
        }
    }


    fn handle_pane_action(&mut self, ctx: &egui::Context, action: PaneAction) {
        match action {
            PaneAction::Press { document, position, void_row, extend } => {
                let cursor = match self.panes.caret {
                    Some((current, cursor)) if extend && current == document => Cursor { anchor: cursor.anchor, head: position },
                    _ => Cursor::at(position),
                };
                self.panes.set_caret(document, cursor, void_row);
                self.panes.reveal_caret = false;
                if let Some(focused) = ctx.memory(|memory| memory.focused()) {
                    ctx.memory_mut(|memory| memory.surrender_focus(focused));
                }
            }
            PaneAction::SelectWord(document, position) => {
                if let Some(lines) = self.mirror.as_ref().and_then(|mirror| lines_of(mirror, document.clone())) {
                    self.panes.set_caret(document, editing::word_at(lines, position), None);
                }
            }
            PaneAction::SelectLine(document, line) => {
                if let Some(lines) = self.mirror.as_ref().and_then(|mirror| lines_of(mirror, document.clone())) {
                    self.panes.set_caret(document, editing::line_at(lines, line), None);
                }
            }
            PaneAction::Drag { document, position, void_row } => {
                if let Some((current, cursor)) = self.panes.caret
                    && current == document
                    && (cursor.head != position || self.panes.void_row != void_row)
                {
                    self.panes.set_caret(document, Cursor { anchor: cursor.anchor, head: position }, void_row);
                    self.panes.reveal_caret = true;
                }
            }
            PaneAction::SelectRegion(document, region) => {
                self.caret_to_region(document, region, false);
                self.panes.reveal_caret = false;
                if let Some(focused) = ctx.memory(|memory| memory.focused()) {
                    ctx.memory_mut(|memory| memory.surrender_focus(focused));
                }
                if let Some(base) = self.mirror.as_ref().and_then(Mirror::navigation_base) {
                    self.send(Command::SetCurrentRegion { base, region: Some(region) }, Pending::SelectRegion);
                }
            }
            PaneAction::CommitPath(document, text) => {
                let path = paths::clean_path_text(&text).map(PathBuf::from);
                self.change_path(document, path);
            }
            PaneAction::Browse(document) => {
                if let Some(path) = open_dialog::pick_file(document.clone(), self.config.recent.last_dir.as_deref()) {
                    self.remember_dir(&path);
                    self.change_path(document, Some(path));
                }
            }
        }
    }


    /// A/B/C reload from `path`; D only changes where it will be saved, and `None` makes it unnamed.
    fn change_path(&mut self, document: Document, path: Option<PathBuf>) {
        match (document, path) {
            (Document::D, path) => {
                self.send(Command::SetOutputPath(path), Pending::SetOutputPath);
            }
            (document, Some(path)) => {
                self.send(Command::LoadSource { document: document.clone(), path }, Pending::LoadSource(document));
            }
            (_, None) => {}
        }
    }


    /// Keys go to the pane with the caret, unless a text field or dialog has the keyboard.
    fn handle_pane_keys(&mut self, ctx: &egui::Context) {
        let document = match self.panes.caret {
            Some((document, _)) => document,
            None => return,
        };
        if self.mirror.is_none() || self.settings.is_some() || self.form_open || ctx.text_edit_focused() {
            return;
        }
        let events = ctx.input(|input| input.events.clone());
        for event in events {
            match event {
                egui::Event::Key { key: Key::A, pressed: true, modifiers, .. } if modifiers.command => self.select_all(),
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    if let Some(motion) = motion_for(key, modifiers, self.panes.page_rows) {
                        self.move_caret(motion, modifiers.shift);
                    } else if document == Document::D
                        && let Some(edit_key) = edit_key_for(key, modifiers)
                    {
                        self.edit_output(edit_key);
                    }
                }
                egui::Event::Text(text) if document == Document::D && !text.chars().any(char::is_control) => self.edit_output(EditKey::Insert(text)),
                egui::Event::Paste(text) if document == Document::D => self.edit_output(EditKey::Insert(text)),
                egui::Event::Copy => self.copy_selection(ctx),
                egui::Event::Cut => {
                    self.copy_selection(ctx);
                    if document == Document::D && self.panes.caret.is_some_and(|(_, cursor)| cursor.has_selection()) {
                        self.edit_output(EditKey::Delete);
                    }
                }
                _ => {}
            }
        }
    }


    /// Moves the caret; `extend` (Shift) grows or shrinks the selection instead of dropping it.
    fn move_caret(&mut self, motion: Motion, extend: bool) {
        let (document, cursor) = match self.panes.caret {
            Some(caret) => caret,
            None => return,
        };
        let mirror = match self.mirror.as_ref() {
            Some(mirror) => mirror,
            None => return,
        };
        let (lines, rows) = match (lines_of(mirror, document.clone()), rows_of(mirror, document.clone())) {
            (Some(lines), Some(rows)) => (lines, rows),
            _ => return,
        };
        let (cursor, void_row) = move_by_rows(lines, rows, Caret { cursor, void_row: self.panes.void_row.clone() }, motion, extend);
        self.panes.set_caret(document, cursor, void_row);
        self.panes.reveal_caret = true;
    }


    fn select_all(&mut self) {
        let document = match self.panes.caret {
            Some((document, _)) => document,
            None => return,
        };
        if let Some(lines) = self.mirror.as_ref().and_then(|mirror| lines_of(mirror, document.clone())) {
            self.panes.set_caret(document, editing::select_all(lines), None);
        }
    }


    fn copy_selection(&self, ctx: &egui::Context) {
        let (document, cursor) = match self.panes.caret {
            Some(caret) => caret,
            None => return,
        };
        if !cursor.has_selection() {
            return;
        }
        if let Some(lines) = self.mirror.as_ref().and_then(|mirror| lines_of(mirror, document)) {
            ctx.copy_text(editing::selected_text(lines, cursor));
        }
    }


    /// Applies a key to D locally at once and sends the edit to the engine.
    fn edit_output(&mut self, key: EditKey) {
        let cursor = match self.panes.caret {
            Some((Document::D, cursor)) => cursor,
            _ => return,
        };
        let mirror = match self.mirror.as_mut() {
            Some(mirror) => mirror,
            None => return,
        };
        let Some(output) = mirror.output() else { return };
        let target = match self.panes.void_row {
            Some(row) if !cursor.has_selection() && cursor.head.column == 0 => mirror.empty_region_at_row(row),
            _ => None,
        };
        let edited = match target {
            Some(_) => editing::edit_on_void(output.lines(), cursor.head, &key),
            None => editing::edit_at(output.lines(), cursor, &key),
        };
        let (edit, caret) = match edited {
            Some(edited) => edited,
            None => return,
        };
        let command = match mirror.apply_local_edit(&edit, target) {
            Some(command) => command,
            None => return,
        };
        self.panes.set_caret(Document::D, Cursor::at(caret), None);
        self.panes.reveal_caret = true;
        if let Some(id) = self.send(Command::EditOutput(command), Pending::Edit)
            && let Some(mirror) = self.mirror.as_mut()
        {
            mirror.edit_sent(id);
        }
    }


    /// The region of the row the caret is on, in any pane.
    fn caret_region(&self) -> Option<usize> {
        let (document, cursor) = self.panes.caret?;
        let rows = rows_of(self.mirror.as_ref()?, document)?;
        rows.region_of_row(text_view::caret_row(rows, Caret { cursor, void_row: self.panes.void_row.clone() }))
    }


    /// Asks the engine to move the current region from the caret's region.
    fn navigate(&mut self, to: Navigation) {
        let Some(base) = self.mirror.as_ref().and_then(Mirror::navigation_base) else { return };
        let from = self.caret_region();
        self.send(Command::Navigate { base, from, to }, Pending::Navigate(to));
    }


    /// Toggles `document` in the region of the row with the caret, in any pane.
    fn choose_current(&mut self, document: Document) {
        if self.panes.caret.is_none() {
            self.report_error("Put the caret in a region to choose for it".to_string());
            return;
        }
        let Some(base) = self.mirror.as_ref().and_then(Mirror::choice_base) else { return };
        let Some(region) = self.caret_region() else {
            self.report_error("The caret is not in a region".to_string());
            return;
        };
        let Some(mirror) = self.mirror.as_ref() else { return };
        let Some(choice) = mirror.toggled_choice(region, document) else { return };
        self.send(Command::SetChoice { base, region, choice }, Pending::Choose(Some(region)));
    }


    fn choose_for_unresolved(&mut self, document: Document) {
        let Some(base) = self.mirror.as_ref().and_then(Mirror::choice_base) else { return };
        self.send(Command::ChooseForUnresolved { base, document }, Pending::Choose(None));
    }


    /// A, B and, in a 3-way merge, C: the documents a choice can take hunks from.
    fn choosable(&self) -> Vec<Document> {
        if !self.is_merge() {
            return Vec::new();
        }
        let count = self.mirror.as_ref().map_or(0, |mirror| mirror.documents().len());
        Document::ALL
            .iter()
            .take(count.min(3))
            .cloned()
            .collect()
    }


    /// Undo and redo act on D, wherever the caret is. While a text field has the keyboard, they are left to the field.
    fn undo_redo(&mut self, ctx: &egui::Context, command: Command) {
        if self.is_merge() && !ctx.text_edit_focused() {
            self.send(command, Pending::UndoRedo);
        }
    }


    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if !ctx.text_edit_focused() {
            if ctx.input_mut(|input| input.consume_shortcut(&REDO_ALT_SHORTCUT) || input.consume_shortcut(&REDO_SHORTCUT)) {
                self.undo_redo(ctx, Command::Redo);
            }
            if ctx.input_mut(|input| input.consume_shortcut(&UNDO_SHORTCUT)) {
                self.undo_redo(ctx, Command::Undo);
            }
            for document in self.choosable() {
                let shortcut = CHOOSE_SHORTCUTS
                    .iter()
                    .find(|(choosable, _)| *choosable == document)
                    .map(|(_, shortcut)| shortcut.clone());
                if let Some(shortcut) = shortcut
                    && ctx.input_mut(|input| input.consume_shortcut(&shortcut))
                {
                    self.choose_current(document);
                }
            }
            if self.mirror.is_some() {
                for (to, _, shortcut) in NAVIGATION_SHORTCUTS.iter() {
                    if ctx.input_mut(|input| input.consume_shortcut(shortcut)) {
                        self.navigate(to.clone());
                    }
                }
            }
        }
        if ctx.input_mut(|input| input.consume_shortcut(&OPEN_SHORTCUT)) {
            self.form_open = true;
        }
        if ctx.input_mut(|input| input.consume_shortcut(&SAVE_AS_SHORTCUT)) {
            self.save_as();
        }
        if ctx.input_mut(|input| input.consume_shortcut(&SAVE_SHORTCUT)) && self.is_merge() {
            self.save();
        }
        if ctx.input_mut(|input| input.consume_shortcut(&EXIT_SHORTCUT)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if ctx.input_mut(|input| input.consume_shortcut(&SETTINGS_SHORTCUT)) {
            self.open_settings(ctx);
        }
    }


    fn show_settings(&mut self, ctx: &egui::Context) {
        let families = self.font_scan.families();
        let dialog = match self.settings.as_mut() {
            Some(dialog) => dialog,
            None => return,
        };
        let mut action = None;
        let modal = egui::Modal::new(egui::Id::unique("settings_dialog")).show(ctx, |ui| {
            action = dialog.show(ui, families);
        });
        if action.is_none() && modal.should_close() {
            action = Some(SettingsAction::Cancel);
        }
        let draft = dialog.draft().clone();
        match action {
            Some(SettingsAction::OpenConfigFile) => self.open_config_file(),
            Some(action) => self.close_settings(ctx, action),
            None if self.applied_view.as_ref() != Some(&draft) => self.apply_view(ctx, &draft),
            None => {}
        }
    }


    fn show_menu(&mut self, ui: &mut egui::Ui) {
        let is_merge = self.is_merge();
        let savable = is_merge && self.mirror.as_ref().is_some_and(|mirror| mirror.unresolved_count() == 0);
        let has_session = self.mirror.is_some();
        egui::Panel::top("menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    let open = egui::Button::new("Open...").shortcut_text(ui.ctx().format_shortcut(&OPEN_SHORTCUT));
                    if ui.add(open).clicked() {
                        self.form_open = true;
                    }
                    let save = egui::Button::new("Save").shortcut_text(ui.ctx().format_shortcut(&SAVE_SHORTCUT));
                    if ui.add_enabled(savable, save).on_disabled_hover_text(UNSAVABLE_HINT).clicked() {
                        self.save();
                    }
                    let save_as = egui::Button::new("Save As...").shortcut_text(ui.ctx().format_shortcut(&SAVE_AS_SHORTCUT));
                    if ui.add_enabled(savable, save_as).on_disabled_hover_text(UNSAVABLE_HINT).clicked() {
                        self.save_as();
                    }
                    ui.separator();
                    let exit = egui::Button::new("Exit").shortcut_text(ui.ctx().format_shortcut(&EXIT_SHORTCUT));
                    if ui.add(exit).clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Edit", |ui| {
                    let ctx = ui.ctx().clone();
                    let undo = egui::Button::new("Undo").shortcut_text(ctx.format_shortcut(&UNDO_SHORTCUT));
                    if ui.add_enabled(is_merge, undo).clicked() {
                        self.undo_redo(&ctx, Command::Undo);
                    }
                    let redo = egui::Button::new("Redo").shortcut_text(ctx.format_shortcut(&REDO_SHORTCUT));
                    if ui.add_enabled(is_merge, redo).clicked() {
                        self.undo_redo(&ctx, Command::Redo);
                    }
                });
                let choosable = self.choosable();
                ui.menu_button("Merge", |ui| {
                    for (document, shortcut) in CHOOSE_SHORTCUTS.iter() {
                        let button = egui::Button::new(format!("Choose {} Here", document.letter())).shortcut_text(ui.ctx().format_shortcut(shortcut));
                        if ui.add_enabled(choosable.contains(document), button).clicked() {
                            self.choose_current(document.clone());
                        }
                    }
                    ui.separator();
                    for document in CHOOSE_SHORTCUTS.iter().map(|(document, _)| document) {
                        let button = egui::Button::new(format!("Choose {} for All Unresolved", document.letter()));
                        if ui.add_enabled(choosable.contains(document), button).clicked() {
                            self.choose_for_unresolved(document.clone());
                        }
                    }
                    ui.separator();
                    for (to, label, shortcut) in NAVIGATION_SHORTCUTS.iter() {
                        let button = egui::Button::new(*label).shortcut_text(ui.ctx().format_shortcut(shortcut));
                        if ui.add_enabled(has_session, button).clicked() {
                            self.navigate(to.clone());
                        }
                    }
                    ui.separator();
                    let mut void_space = self.view().align_output;
                    if ui.add_enabled(is_merge, egui::Checkbox::new(&mut void_space, VOID_TOGGLE_LABEL)).on_hover_text(VOID_TOGGLE_HINT).clicked() {
                        self.toggle_void_space();
                    }
                });
                ui.menu_button("Settings", |ui| {
                    let configure = egui::Button::new("Configure Kdiff4...").shortcut_text(ui.ctx().format_shortcut(&SETTINGS_SHORTCUT));
                    if ui.add(configure).clicked() {
                        let ctx = ui.ctx().clone();
                        self.open_settings(&ctx);
                    }
                });
            });
        });
    }


    /// The strip under the menu bar: the void space toggle of the output pane and the overview strip toggle.
    fn show_toolbar(&mut self, ui: &mut egui::Ui) {
        let is_merge = self.is_merge();
        let on = self.view().align_output;
        let overview_on = self.view().show_overview_strip;
        let mut toggled = false;
        let mut overview_toggled = false;
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                let response = toolbar_toggle(ui, VOID_TOGGLE_LABEL, on, is_merge, paint_void_icon).on_hover_text(VOID_TOGGLE_HINT);
                if is_merge && response.clicked() {
                    toggled = true;
                }
                if toolbar_toggle(ui, OVERVIEW_TOGGLE_LABEL, overview_on, true, paint_overview_icon).on_hover_text(OVERVIEW_TOGGLE_HINT).clicked() {
                    overview_toggled = true;
                }
            });
        });
        if toggled {
            self.toggle_void_space();
        }
        if overview_toggled {
            self.toggle_overview_strip();
        }
    }


    /// Shows or hides void space in D, from the toolbar or the Merge menu. Saved at once, and followed by an open settings dialog.
    fn toggle_void_space(&mut self) {
        let on = !self.view().align_output;
        self.config.view.align_output = on;
        if let Some(applied) = self.applied_view.as_mut() {
            applied.align_output = on;
        }
        if let Some(dialog) = self.settings.as_mut() {
            dialog.set_align_output(on);
        }
        self.save_config(false);
    }


    /// Shows or hides the overview strip, from the toolbar. Saved at once, and followed by an open settings dialog.
    fn toggle_overview_strip(&mut self) {
        let on = !self.view().show_overview_strip;
        self.config.view.show_overview_strip = on;
        if let Some(applied) = self.applied_view.as_mut() {
            applied.show_overview_strip = on;
        }
        if let Some(dialog) = self.settings.as_mut() {
            dialog.set_show_overview_strip(on);
        }
        self.save_config(false);
    }


    /// The view settings in effect: the settings dialog's draft while it is open, otherwise the saved ones.
    fn view(&self) -> &ViewConfig {
        self.applied_view.as_ref().unwrap_or(&self.config.view)
    }


    fn pane_options(&self) -> PaneOptions {
        let view = self.view();
        let [r, g, b, a] = view.current_region_color;
        let rgb = |[r, g, b]: [u8; 3]| egui::Color32::from_rgb(r, g, b);
        let char_diff = CharDiffLook { red: rgb(view.char_diff_red), blue: rgb(view.char_diff_blue), green: rgb(view.char_diff_green), corner_radius: view.char_diff_corner_radius.clone() };
        PaneOptions { line_numbers: view.show_line_numbers, gutters: view.show_gutters, current_region_color: egui::Color32::from_rgba_unmultiplied(r, g, b, a), jump_context_rows: view.jump_context_lines as usize, scroll_step_rows: view.scroll_step_lines as usize, scroll_motion: ScrollMotion { smooth: view.smooth_scrolling, duration: view.smooth_scroll_ms as f32 / 1000.0 }, region_gap: view.region_gap as f32, region_corner_radius: view.region_corner_radius, char_diff, whitespace: view.whitespace.clone(), overview_strip: view.show_overview_strip }
    }


    /// Always shown and one line high, so the panes never move when a message comes or goes. Long messages are cut, with the full text in a tooltip.
    fn show_status(&self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            let (text, color) = match self.status.as_ref() {
                Some(status) if status.is_error => (status.text.as_str(), ui.visuals().error_fg_color),
                Some(status) => (status.text.as_str(), theme::palette_of(ui).weak_text),
                None => ("Ready", theme::palette_of(ui).weak_text),
            };
            ui.add(egui::Label::new(RichText::new(text).color(color)).truncate());
        });
    }


    /// egui sets the window theme on the first frame, before the window has focus, and Windows then keeps the old title bar colors until focus changes. Sending the theme again once the window has focus repaints the title bar.
    fn refresh_title_bar(&mut self, ctx: &egui::Context) {
        if self.title_bar_refreshed || !ctx.input(|input| input.viewport().focused.unwrap_or(false)) {
            return;
        }
        let theme = match ctx.options(|options| options.theme_preference) {
            egui::ThemePreference::Light => egui::SystemTheme::Light,
            egui::ThemePreference::Dark => egui::SystemTheme::Dark,
            egui::ThemePreference::System => egui::SystemTheme::SystemDefault,
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(theme));
        self.title_bar_refreshed = true;
    }


    /// Keeps the native frame's corners in step with the configured radius.
    fn update_window_frame(&mut self, frame: &eframe::Frame) {
        let radius = self.applied_view.as_ref().map(|view| view.corner_radius).unwrap_or(config::DEFAULT_CORNER_RADIUS);
        if self.frame_radius != Some(radius) {
            window_frame::apply(frame, window_frame::corner_preference(radius));
            self.frame_radius = Some(radius);
        }
    }
}


impl eframe::App for KdiffApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_messages(ctx);
    }


    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_shortcuts(&ctx);
        self.handle_pane_keys(&ctx);
        self.show_menu(ui);
        if self.mirror.is_some() {
            self.show_toolbar(ui);
        }
        let aligned = self.view().align_output;
        if let Some(mirror) = self.mirror.as_mut() {
            mirror.set_aligned(aligned);
        }
        let pane_options = self.pane_options();

        let busy = self.is_opening();
        let mut form_action = None;
        let mut pane_actions = Vec::new();
        match self.mirror.as_ref() {
            None => {
                egui::CentralPanel::default().show(ui, |ui| {
                    form_action = self.form.show(ui, self.error.as_deref(), busy, false);
                });
            }
            Some(mirror) => {
                self.show_status(ui);
                pane_actions = panes::show_session(ui, mirror, &mut self.panes, &pane_options);
                self.panes.reveal_caret = false;
                if self.form_open {
                    let modal = egui::Modal::new(egui::Id::unique("open_dialog")).show(&ctx, |ui| {
                        ui.heading("Open");
                        form_action = self.form.show(ui, self.error.as_deref(), busy, !busy);
                    });
                    if !busy && modal.should_close() {
                        form_action = Some(FormAction::Cancel);
                    }
                }
            }
        }
        for action in pane_actions {
            self.handle_pane_action(&ctx, action);
        }
        if let Some(action) = form_action {
            self.handle_form_action(action);
        }
        self.show_settings(&ctx);
        self.update_window_frame(frame);
        self.refresh_title_bar(&ctx);
    }
}


/// The caret at the start of `region` in `document`, with its void row when the region shows no line there.
fn region_start_caret(mirror: &Mirror, document: Document, region: usize) -> Option<(Cursor, Option<usize>)> {
    let lines = lines_of(mirror, document.clone())?;
    let rows = rows_of(mirror, document)?;
    let (position, void_row) = rows.caret_at_row(rows.region_start(region)?, 0, text_end(lines));
    Some((Cursor::at(editing::clamp(lines, position)), void_row))
}


fn rows_of(mirror: &Mirror, document: Document) -> Option<Rows<'_>> {
    match document {
        Document::D => mirror.output().map(|_| Rows::Mapped(mirror.output_rows())),
        document => {
            let text = mirror.document(document.clone())?;
            Some(mirror.rows(document).map_or(Rows::PerLine(text.line_count()), Rows::Mapped))
        }
    }
}


fn text_end(lines: Lines<'_>) -> Position {
    editing::move_caret(lines, Position::default(), Motion::DocumentEnd)
}


/// Moves the caret. Up, Down, PageUp and PageDown go row by row, so the caret can stop on void rows; other motions work on lines and leave voids.
fn move_by_rows(lines: Lines<'_>, rows: Rows<'_>, caret: Caret, motion: Motion, extend: bool) -> (Cursor, Option<usize>) {
    let delta = match motion {
        Motion::Up => -1,
        Motion::Down => 1,
        Motion::PageUp(page) => -(page as isize),
        Motion::PageDown(page) => page as isize,
        _ => return (editing::move_cursor(lines, caret.cursor, motion, extend), None),
    };
    if rows.len() == 0 {
        return (editing::move_cursor(lines, caret.cursor, motion, extend), None);
    }
    let target = rows::step_row(text_view::caret_row(rows, caret), delta, rows.len());
    let (position, void_row) = rows.caret_at_row(target, caret.cursor.head.column, text_end(lines));
    let head = editing::clamp(lines, position);
    let cursor = if extend { Cursor { anchor: caret.cursor.anchor, head } } else { Cursor::at(head) };
    (cursor, void_row)
}


fn navigation_noun(to: Navigation) -> &'static str {
    match to {
        Navigation::PreviousDifference => "previous difference",
        Navigation::NextDifference => "next difference",
        Navigation::PreviousConflict => "previous conflict",
        Navigation::NextConflict => "next conflict",
    }
}


/// The size of a toolbar button: room for the icon above the label, plus the button padding.
fn toolbar_button_size(label_size: egui::Vec2, padding: egui::Vec2) -> egui::Vec2 {
    egui::vec2(label_size.x.max(TOOLBAR_ICON_SIZE), TOOLBAR_ICON_SIZE + TOOLBAR_ICON_GAP + label_size.y) + padding * 2.0
}


/// A bordered toolbar button with its icon centered above the label; filled while `on`. `paint_icon` draws the icon into its square in the given color.
fn toolbar_toggle(ui: &mut egui::Ui, label: &str, on: bool, enabled: bool, paint_icon: impl FnOnce(&egui::Ui, egui::Rect, egui::Color32)) -> egui::Response {
    let palette = theme::palette_of(ui);
    let color = if enabled { palette.text } else { palette.weak_text.gamma_multiply(0.5) };
    let galley = ui.painter().layout_no_wrap(label.to_string(), egui::TextStyle::Button.resolve(ui.style()), color);
    let padding = ui.spacing().button_padding;
    let (rect, response) = ui.allocate_exact_size(toolbar_button_size(galley.size(), padding), egui::Sense::click());
    let visuals = ui.visuals();
    let fill = if on {
        visuals.selection.bg_fill
    } else if enabled && response.hovered() {
        visuals.widgets.hovered.weak_bg_fill
    } else {
        visuals.widgets.inactive.weak_bg_fill
    };
    ui.painter().rect(rect, theme::corner_radius_of(ui), fill, egui::Stroke::new(1.0, palette.pane_border), egui::StrokeKind::Inside);
    let icon = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + padding.y + TOOLBAR_ICON_SIZE / 2.0), egui::vec2(TOOLBAR_ICON_SIZE, TOOLBAR_ICON_SIZE));
    paint_icon(ui, icon, color);
    ui.painter().galley(egui::pos2(rect.center().x - galley.size().x / 2.0, icon.bottom() + TOOLBAR_ICON_GAP), galley, color);
    response
}


/// A small square, hatched like void space.
fn paint_void_icon(ui: &egui::Ui, rect: egui::Rect, stroke_color: egui::Color32) {
    let radius = theme::corner_radius_of(ui).max(3);
    let square = egui::Rect::from_center_size(rect.center(), rect.size() * 0.8);
    let clip = ui.painter().with_clip_rect(square);
    let spacing = 3.0;
    let mut sum = square.left() + square.top() - square.height();
    while sum < square.right() + square.bottom() {
        clip.line_segment([egui::pos2(sum - square.bottom(), square.bottom()), egui::pos2(sum - square.top(), square.top())], egui::Stroke::new(1.0, stroke_color));
        sum += spacing;
    }
    ui.painter().rect_stroke(square, radius.min(4), egui::Stroke::new(1.0, stroke_color), egui::StrokeKind::Inside);
}


/// Four lines of text of different lengths in a box.
fn paint_overview_icon(ui: &egui::Ui, rect: egui::Rect, stroke_color: egui::Color32) {
    let radius = theme::corner_radius_of(ui).max(3);
    let square = egui::Rect::from_center_size(rect.center(), rect.size() * 0.8);
    let stroke = egui::Stroke::new(1.0, stroke_color);
    let left = square.left() + 3.0;
    for (index, share) in [0.8, 0.55, 0.7, 0.45].iter().enumerate() {
        let y = square.top() + square.height() * (index as f32 + 1.0) / 5.0;
        ui.painter().hline(left..=left + (square.width() - 6.0) * share, y, stroke);
    }
    ui.painter().rect_stroke(square, radius.min(4), stroke, egui::StrokeKind::Inside);
}


fn lines_of(mirror: &Mirror, document: Document) -> Option<Lines<'_>> {
    match document {
        Document::D => mirror.output().map(|output| output.lines()),
        document => mirror.document(document).map(|text| Lines { text: &text.text, ranges: &text.lines }),
    }
}


/// Caret keys; with Ctrl, Left and Right jump by words, and Home and End go to the start and end of the document.
fn motion_for(key: Key, modifiers: Modifiers, page_rows: usize) -> Option<Motion> {
    let motion = match key {
        Key::ArrowLeft if modifiers.command => Motion::WordLeft,
        Key::ArrowRight if modifiers.command => Motion::WordRight,
        Key::ArrowLeft => Motion::Left,
        Key::ArrowRight => Motion::Right,
        Key::ArrowUp => Motion::Up,
        Key::ArrowDown => Motion::Down,
        Key::Home if modifiers.command => Motion::DocumentStart,
        Key::End if modifiers.command => Motion::DocumentEnd,
        Key::Home => Motion::LineStart,
        Key::End => Motion::LineEnd,
        Key::PageUp => Motion::PageUp(page_rows),
        Key::PageDown => Motion::PageDown(page_rows),
        _ => return None,
    };
    Some(motion)
}


/// Enter works with Ctrl held too, so a fast Ctrl+V, Enter sequence loses no line breaks. Ctrl+Backspace and Ctrl+Delete delete by words.
fn edit_key_for(key: Key, modifiers: Modifiers) -> Option<EditKey> {
    if modifiers.alt {
        return None;
    }
    match key {
        Key::Enter => Some(EditKey::Newline),
        Key::Backspace if modifiers.command => Some(EditKey::DeleteWordLeft),
        Key::Delete if modifiers.command => Some(EditKey::DeleteWordRight),
        Key::Backspace => Some(EditKey::Backspace),
        Key::Delete => Some(EditKey::Delete),
        _ => None,
    }
}


#[cfg(test)]
mod tests {
    use super::*;


    /// A 2-way merge of two lines: a modified region resolved to B, then an unchanged one; D is "b\nx\n".
    fn resolved_mirror() -> Mirror {
        use std::path::PathBuf;
        use kdiff4_protocol::engine::{Comparison, ComparisonOptions, EngineCapabilities, HunkChange, Region, RegionStatus, Row};
        use kdiff4_protocol::{Choice, DocumentText, EolStyle, OutputDocument, RegionMerge, Revision, Snapshot};
        let source = |document: Document, text: &str| DocumentText { document: document.clone(), path: PathBuf::from(format!("{}.txt", document.letter())), label: None, eol: EolStyle::Lf, text: text.to_string(), lines: vec![0..1, 2..3] };
        let region = |index: usize, status: RegionStatus| Region { hunks: vec![index..index + 1, index..index + 1], status, rows: vec![Row { lines: vec![Some(index), Some(index)] }] };
        let comparison = Comparison { regions: vec![region(0, RegionStatus::Diff(HunkChange::Modified)), region(1, RegionStatus::Diff(HunkChange::Unchanged))] };
        let merge = vec![RegionMerge { choice: Choice::Hunks(vec![Document::B]), edited: false, output_lines: 1 }, RegionMerge { choice: Choice::Hunks(vec![Document::A]), edited: false, output_lines: 1 }];
        let output = OutputDocument { path: None, text: "b\nx\n".to_string(), modified: false };
        Mirror::from_snapshot(Snapshot { revision: Revision(1), documents: vec![source(Document::A, "a\nx\n"), source(Document::B, "b\nx\n")], output: Some(output), comparison, engine: EngineCapabilities::default(), merge, options: ComparisonOptions::default(), current_region: None, char_diffs: Vec::new() }, false)
    }


    #[test]
    fn caret_stays_in_a_region_that_loses_its_lines() {
        use kdiff4_protocol::engine::{HunkChange, Region, RegionStatus, Row};
        use kdiff4_protocol::{Choice, Delta, MergeUpdate, RegionMerge, Revision};
        let mut mirror = resolved_mirror();
        let (cursor, void_row) = region_start_caret(&mirror, Document::D, 0).unwrap();
        assert_eq!(void_row, None);
        let region = Region { hunks: vec![0..1, 0..1], status: RegionStatus::Diff(HunkChange::Modified), rows: vec![Row { lines: vec![Some(0), Some(0)] }] };
        let update = MergeUpdate { at: 0, removed: 1, regions: vec![region], merge: vec![RegionMerge { choice: Choice::Unresolved, edited: false, output_lines: 0 }] };
        assert_eq!(mirror.apply_delta(Delta { revision: Revision(2), cause: None, change: Change::OutputEdited { range: 0..2, text: String::new(), modified: true, merge: update } }), DeltaOutcome::Applied);
        let rows = rows_of(&mirror, Document::D).unwrap();
        assert_eq!(rows.region_of_row(text_view::caret_row(rows, Caret { cursor, void_row })), Some(1), "the old caret now points into the region below");
        let (cursor, void_row) = region_start_caret(&mirror, Document::D, 0).unwrap();
        assert!(void_row.is_some());
        assert_eq!(rows.region_of_row(text_view::caret_row(rows, Caret { cursor, void_row })), Some(0));
    }


    #[test]
    fn caret_keys_map_to_motions() {
        assert_eq!(motion_for(Key::ArrowLeft, Modifiers::NONE, 10), Some(Motion::Left));
        assert_eq!(motion_for(Key::Home, Modifiers::NONE, 10), Some(Motion::LineStart));
        assert_eq!(motion_for(Key::Home, Modifiers::COMMAND, 10), Some(Motion::DocumentStart));
        assert_eq!(motion_for(Key::End, Modifiers::COMMAND, 10), Some(Motion::DocumentEnd));
        assert_eq!(motion_for(Key::PageDown, Modifiers::NONE, 7), Some(Motion::PageDown(7)));
        assert_eq!(motion_for(Key::A, Modifiers::NONE, 7), None);
        assert_eq!(motion_for(Key::ArrowLeft, Modifiers::COMMAND, 7), Some(Motion::WordLeft));
        assert_eq!(motion_for(Key::ArrowRight, Modifiers::COMMAND.plus(Modifiers::SHIFT), 7), Some(Motion::WordRight));
    }


    #[test]
    fn editing_keys_work_with_ctrl() {
        assert_eq!(edit_key_for(Key::Enter, Modifiers::NONE), Some(EditKey::Newline));
        assert_eq!(edit_key_for(Key::Backspace, Modifiers::SHIFT), Some(EditKey::Backspace));
        assert_eq!(edit_key_for(Key::Delete, Modifiers::NONE), Some(EditKey::Delete));
        assert_eq!(edit_key_for(Key::Enter, Modifiers::COMMAND), Some(EditKey::Newline));
        assert_eq!(edit_key_for(Key::Backspace, Modifiers::COMMAND), Some(EditKey::DeleteWordLeft));
        assert_eq!(edit_key_for(Key::Delete, Modifiers::COMMAND), Some(EditKey::DeleteWordRight));
        assert_eq!(edit_key_for(Key::Enter, Modifiers::ALT), None);
        assert_eq!(edit_key_for(Key::S, Modifiers::NONE), None);
    }


    /// Lines "ab" and "c" of A, with a void row between them.
    fn rows_with_a_void() -> rows::RowMap {
        use kdiff4_protocol::engine::{Comparison, HunkChange, Region, RegionStatus, Row};
        let region = |hunk: std::ops::Range<usize>, line: Option<usize>| Region { hunks: vec![hunk.clone(), hunk], status: RegionStatus::Diff(HunkChange::Unchanged), rows: vec![Row { lines: vec![line, line] }] };
        let comparison = Comparison { regions: vec![region(0..1, Some(0)), region(1..1, None), region(1..2, Some(1))] };
        rows::RowMap::for_source(&comparison, &[], 0, 2, None)
    }


    #[test]
    fn vertical_moves_stop_on_void_rows() {
        let map = rows_with_a_void();
        let rows = Rows::Mapped(&map);
        let ranges = [0..2, 3..4];
        let lines = Lines { text: "ab\nc", ranges: &ranges };
        let at = |line, column| Position { line, column };
        let start = Caret { cursor: Cursor::at(at(0, 2)), void_row: None };
        let (cursor, void_row) = move_by_rows(lines, rows, start, Motion::Down, false);
        assert_eq!((cursor, void_row), (Cursor::at(at(1, 0)), Some(1)));
        let (cursor, void_row) = move_by_rows(lines, rows, Caret { cursor, void_row }, Motion::Down, false);
        assert_eq!((cursor, void_row), (Cursor::at(at(1, 0)), None));
        let (cursor, void_row) = move_by_rows(lines, rows, Caret { cursor, void_row }, Motion::PageUp(5), true);
        assert_eq!((cursor, void_row), (Cursor { anchor: at(1, 0), head: at(0, 0) }, None));
        let (cursor, void_row) = move_by_rows(lines, rows, Caret { cursor: Cursor::at(at(0, 1)), void_row: Some(1) }, Motion::Right, false);
        assert_eq!((cursor, void_row), (Cursor::at(at(0, 2)), None));
    }


    #[test]
    fn toolbar_buttons_fit_the_icon_above_the_label() {
        let padding = egui::vec2(4.0, 1.0);
        let wide = toolbar_button_size(egui::vec2(60.0, 14.0), padding);
        assert_eq!(wide, egui::vec2(60.0 + 8.0, TOOLBAR_ICON_SIZE + TOOLBAR_ICON_GAP + 14.0 + 2.0));
        let narrow = toolbar_button_size(egui::vec2(5.0, 14.0), padding);
        assert_eq!(narrow.x, TOOLBAR_ICON_SIZE + 8.0);
    }


    #[test]
    fn missing_navigation_targets_are_named() {
        assert_eq!(navigation_noun(Navigation::NextConflict), "next conflict");
        assert_eq!(navigation_noun(Navigation::PreviousDifference), "previous difference");
    }
}
