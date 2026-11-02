use std::collections::HashMap;
use std::path::{Path, PathBuf};

use egui::{Key, KeyboardShortcut, Modifiers, RichText};
use kdiff4_protocol::{Change, Client, Command, Document, Event, HostMessage, RequestId, Response};

use crate::Startup;
use crate::config::{self, GuiConfig, ViewConfig};
use crate::editing::{self, Cursor, EditKey, Lines, Motion};
use crate::fonts::{self, FontScan};
use crate::mirror::{DeltaOutcome, Mirror};
use crate::open_dialog::{self, FormAction, OpenForm};
use crate::panes::{self, PaneAction, PaneState};
use crate::settings_dialog::{SettingsAction, SettingsDialog};
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
pub const INITIAL_WINDOW_SIZE: [f32; 2] = [640.0, 190.0];
/// The startup window grows to this while the settings dialog is open, so the dialog fits.
const SETTINGS_WINDOW_SIZE: [f32; 2] = [640.0, 400.0];
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
}


/// A message under the panes: the outcome of the last save, reload or path change.
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
        ctx.set_theme(egui::ThemePreference::Light);
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


    /// Applies fonts and styles; reloads the font only when the family changed.
    fn apply_view(&mut self, ctx: &egui::Context, view: &ViewConfig) {
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
            self.settings = Some(SettingsDialog::new(&self.config.view));
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
            }
            SettingsAction::Cancel => self.apply_view(ctx, dialog.original()),
        }
        if !self.window_enlarged {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(INITIAL_WINDOW_SIZE.into()));
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
                self.mirror = Some(Mirror::from_snapshot(snapshot));
                self.form_open = false;
                self.error = None;
                if !self.window_enlarged {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(SESSION_WINDOW_SIZE.into()));
                    self.window_enlarged = true;
                }
            }
            HostMessage::Event(Event::Delta(delta)) => {
                let from_undo = delta.cause.as_ref().is_some_and(|cause| self.pending.get(cause) == Some(&Pending::UndoRedo));
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
                            self.panes.caret = Some((Document::D, Cursor::at(editing::position_of(lines, offset))));
                            self.panes.reveal_caret = true;
                        }
                    }
                    DeltaOutcome::Ignored => {}
                }
            }
            HostMessage::Response(Response { id, result }) => self.handle_response(id, result),
        }
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
                        .map(|path| Status { text: format!("Saved {}", path.display()), is_error: false }),
                    Pending::Edit | Pending::Resync | Pending::UndoRedo => self.status.take(),
                    Pending::Open | Pending::LoadSource(_) | Pending::SetOutputPath => None,
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
            Pending::Resync => self.report_error(format!("Cannot resync with the engine: {}", error.message)),
            Pending::UndoRedo => self.report_error(format!("Cannot undo or redo: {}", error.message)),
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
            PaneAction::Press { document, position, extend } => {
                let cursor = match self.panes.caret {
                    Some((current, cursor)) if extend && current == document => Cursor { anchor: cursor.anchor, head: position },
                    _ => Cursor::at(position),
                };
                self.panes.caret = Some((document, cursor));
                self.panes.reveal_caret = false;
                if let Some(focused) = ctx.memory(|memory| memory.focused()) {
                    ctx.memory_mut(|memory| memory.surrender_focus(focused));
                }
            }
            PaneAction::SelectWord(document, position) => {
                if let Some(lines) = self.mirror.as_ref().and_then(|mirror| lines_of(mirror, document.clone())) {
                    self.panes.caret = Some((document, editing::word_at(lines, position)));
                }
            }
            PaneAction::SelectLine(document, line) => {
                if let Some(lines) = self.mirror.as_ref().and_then(|mirror| lines_of(mirror, document.clone())) {
                    self.panes.caret = Some((document, editing::line_at(lines, line)));
                }
            }
            PaneAction::Drag(document, position) => {
                if let Some((current, cursor)) = self.panes.caret
                    && current == document
                    && cursor.head != position
                {
                    self.panes.caret = Some((document, Cursor { anchor: cursor.anchor, head: position }));
                    self.panes.reveal_caret = true;
                }
            }
            PaneAction::CommitPath(document, text) => {
                let text = text.trim();
                let path = if text.is_empty() { None } else { Some(PathBuf::from(text)) };
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
        let lines = match self.mirror.as_ref().and_then(|mirror| lines_of(mirror, document.clone())) {
            Some(lines) => lines,
            None => return,
        };
        self.panes.caret = Some((document, editing::move_cursor(lines, cursor, motion, extend)));
        self.panes.reveal_caret = true;
    }


    fn select_all(&mut self) {
        let document = match self.panes.caret {
            Some((document, _)) => document,
            None => return,
        };
        if let Some(lines) = self.mirror.as_ref().and_then(|mirror| lines_of(mirror, document.clone())) {
            self.panes.caret = Some((document, editing::select_all(lines)));
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
        let edited = mirror.output().and_then(|output| editing::edit_at(output.lines(), cursor, &key));
        let (edit, caret) = match edited {
            Some(edited) => edited,
            None => return,
        };
        let command = match mirror.apply_local_edit(&edit) {
            Some(command) => command,
            None => return,
        };
        self.panes.caret = Some((Document::D, Cursor::at(caret)));
        self.panes.reveal_caret = true;
        if let Some(id) = self.send(Command::EditOutput(command), Pending::Edit)
            && let Some(mirror) = self.mirror.as_mut()
        {
            mirror.edit_sent(id);
        }
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
            Some(action) => self.close_settings(ctx, action),
            None if self.applied_view.as_ref() != Some(&draft) => self.apply_view(ctx, &draft),
            None => {}
        }
    }


    fn show_menu(&mut self, ui: &mut egui::Ui) {
        let is_merge = self.is_merge();
        egui::Panel::top("menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    let open = egui::Button::new("Open...").shortcut_text(ui.ctx().format_shortcut(&OPEN_SHORTCUT));
                    if ui.add(open).clicked() {
                        self.form_open = true;
                    }
                    let save = egui::Button::new("Save").shortcut_text(ui.ctx().format_shortcut(&SAVE_SHORTCUT));
                    if ui.add_enabled(is_merge, save).clicked() {
                        self.save();
                    }
                    let save_as = egui::Button::new("Save As...").shortcut_text(ui.ctx().format_shortcut(&SAVE_AS_SHORTCUT));
                    if ui.add_enabled(is_merge, save_as).clicked() {
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


    fn show_status(&self, ui: &mut egui::Ui) {
        let status = match self.status.as_ref() {
            Some(status) => status,
            None => return,
        };
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            let color = if status.is_error { ui.visuals().error_fg_color } else { theme::palette_of(ui).weak_text };
            ui.label(RichText::new(&status.text).color(color));
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
                pane_actions = panes::show_session(ui, mirror, &mut self.panes);
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
}
