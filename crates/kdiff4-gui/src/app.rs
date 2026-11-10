use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui_wgpu::wgpu::rwh::HasWindowHandle;
use egui::{Key, KeyboardShortcut, Modifiers, RichText};
use egui_commonmark::CommonMarkCache;
use semver::Version;
use kdiff4_protocol::{Alignments, Change, Client, Command, Document, EolChoice, EolStyle, ErrorKind, Event, HeuristicOptions, HostMessage, InsetChoice, MarkStatus, Navigation, RequestId, Response};

use crate::Startup;
use crate::about_dialog;
use crate::config::{self, GuiConfig, ViewConfig};
use crate::dialog_window::{self, DialogSpec, DialogWindows};
use crate::editing::{self, Cursor, EditKey, Lines, Motion, Position};
use crate::eol::{self, EolAnswer};
use crate::find::{self, FindEvent};
use crate::folding::{self, Folds};
use crate::fonts::{self, FontScan};
use crate::language::{self, Language, LanguageState};
use crate::mirror::{DeltaOutcome, Mirror};
use crate::open_dialog::{self, FormAction, OpenForm};
use crate::overview;
use crate::panes::{self, PaneAction, PaneOptions, PaneState};
use crate::paths;
use crate::rows::{self, Rows};
use crate::scroll::ScrollMotion;
use crate::settings_dialog::{self, SettingsAction, SettingsDialog};
use crate::start_menu::{ShortcutError, ShortcutInstall};
use crate::text_view::{self, Caret};
use crate::self_update::{self, UpdateInstall};
use crate::theme;
use crate::update_check::{self, CheckOutcome, ReleaseInfo, UpdateCheck, UpdateError};
use crate::heuristics_dialog::{self, HeuristicsAction, HeuristicsDialog};
use crate::inset_dialog::{self, InsetAnswer};
use crate::update_dialog::{self, AvailableAction, InstallOffer};
use crate::window_frame;

const OPEN_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const RELOAD_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F5);
const SAVE_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const SAVE_AS_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
const UNDO_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
const REDO_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
const MARK_LINES_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Space);
const MARK_FUNCTION_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Space);
const APPLY_ALIGNMENTS_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
const CLEAR_ALIGNMENTS_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Y);
const HEURISTICS_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::H);
const HEURISTICS_INSIDE_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::ALT), Key::H);
const INSIDE_ALIGNMENT_HINT: &str = "Put the caret inside an applied alignment to align inside it";
const EXIT_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Q);
const SETTINGS_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma);
const FIND_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::F);
const FIND_NEXT_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F3);
const FIND_PREVIOUS_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::SHIFT, Key::F3);
const MINIMAP_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::G);
const IGNORE_WHITESPACE_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::W);
const INDEPENDENT_SCROLL_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::U);
/// egui can't tell the numeric keypad's minus and plus from the main ones, so both fold and unfold.
const FOLD_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Minus);
const UNFOLD_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Plus);
const FOLD_ALL_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Minus);
const UNFOLD_ALL_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Plus);
/// Toggle A, B or C in the region with the caret (kdiff3: Ctrl+1, Ctrl+2, Ctrl+3).
const CHOOSE_SHORTCUTS: [(Document, KeyboardShortcut); 3] = [
    (Document::A, KeyboardShortcut::new(Modifiers::COMMAND, Key::Num1)),
    (Document::B, KeyboardShortcut::new(Modifiers::COMMAND, Key::Num2)),
    (Document::C, KeyboardShortcut::new(Modifiers::COMMAND, Key::Num3)),
];
/// Ctrl+Up and Ctrl+Down go to the previous and next difference, Ctrl+PageUp and Ctrl+PageDown to the previous and next conflict.
const NAVIGATION_SHORTCUTS: [(Navigation, &str, KeyboardShortcut); 4] = [
    (
        Navigation::PreviousDifference,
        "Previous Difference",
        KeyboardShortcut::new(Modifiers::COMMAND, Key::ArrowUp),
    ),
    (Navigation::NextDifference, "Next Difference", KeyboardShortcut::new(Modifiers::COMMAND, Key::ArrowDown)),
    (Navigation::PreviousConflict, "Previous Conflict", KeyboardShortcut::new(Modifiers::COMMAND, Key::PageUp)),
    (Navigation::NextConflict, "Next Conflict", KeyboardShortcut::new(Modifiers::COMMAND, Key::PageDown)),
];
/// Distance of the find bar from the top right corner of the panes.
const FIND_BAR_MARGIN: egui::Vec2 = egui::vec2(24.0, 6.0);
const VOID_TOGGLE_LABEL: &str = "Void Space";
const VOID_TOGGLE_HINT: &str = "Toggle Void Space visibility in Output Pane";
const MINIMAP_TOGGLE_LABEL: &str = "Minimap";
const MINIMAP_TOGGLE_HINT: &str = "Toggle the Minimap: the whole document as a column right of the panes, to scroll by clicking or dragging";
const OVERVIEW_TOGGLE_LABEL: &str = "Overview Strip";
const OVERVIEW_TOGGLE_HINT: &str = "Toggle the Overview Strip: the current line of every document, with its character diff";
const IGNORE_WHITESPACE_TOGGLE_LABEL: &str = "Ignore Whitespace";
const IGNORE_WHITESPACE_TOGGLE_HINT: &str = "Toggle ignoring whitespace and line breaks in the character diff, so reflowed or reindented text shows only its real changes";
const INDEPENDENT_SCROLL_TOGGLE_LABEL: &str = "Independent Scroll";
const INDEPENDENT_SCROLL_TOGGLE_HINT: &str = "Toggle independent scrolling: every pane scrolls on its own. Turned off, the panes line up again with the one last clicked or scrolled";
const TOOLBAR_ICON_SIZE: f32 = 18.0;
/// Space between a toolbar button's icon and its label.
const TOOLBAR_ICON_GAP: f32 = 2.0;
const UNSAVABLE_HINT: &str = "Resolve every conflict before saving";
const ADD_TO_START_MENU_LABEL: &str = "Add to Start Menu";
const ADD_TO_START_MENU_HINT: &str = "Create a Kdiff4 shortcut in your Start menu that starts this executable";
const ALIGNMENTS_CHANGED_HINT: &str = "Alignments changed: Apply Alignments (Ctrl+Y) compares the documents again";
pub const WINDOW_TITLE: &str = "Kdiff4";
pub const INITIAL_WINDOW_SIZE: [f32; 2] = [640.0, 190.0];
const UPDATE_DIALOG_ID: &str = "update_dialog";

/// A save held back until the user chooses the output's line ending style.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingSave {
    Save,
    SaveAs(PathBuf),
}

/// What a request in flight was for, so its response can be reported.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    Open,
    Edit,
    LoadSource(Document),
    Reload,
    SetOutputPath,
    SetOutputEol,
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
    /// A mark, anchor, renumbering, removal or clearing of alignment marks.
    AlignmentEdit,
    ApplyAlignments,
    /// A request for the matches of the Heuristics Alignment window.
    HeuristicPropose,
    /// The heuristic alignments sent, and the alignment they were limited to, to send again once the user confirms dropping D's work.
    HeuristicApply(Vec<Vec<std::ops::Range<usize>>>, Option<u32>),
    /// A request for the fold ranges of the documents.
    FoldRanges,
}

/// A message under the panes: the differences found on open and reload, or the outcome of the last save, reload or path change.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Status {
    text: String,
    is_error: bool,
}

/// The About or update window in front of everything: build information, the offer, the download with its progress, or a notice.
enum UpdateWindow {
    About,
    Available(ReleaseInfo),
    Installing { release: ReleaseInfo, install: UpdateInstall },
    Message { title: String, text: String },
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
    language: LanguageState,
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
    /// The title last sent to the native window.
    window_title: String,
    current_version: Version,
    update_check: Option<UpdateCheck>,
    /// The Start menu shortcut being written, while PowerShell runs.
    shortcut_install: Option<ShortcutInstall>,
    update_window: Option<UpdateWindow>,
    update_cache: CommonMarkCache,
    dialogs: DialogWindows,
    /// The alignment a mark just completed that the documents disagree on, waiting for the user to say where to show it.
    inset_prompt: Option<InsetChoice>,
    /// The save waiting for the user to choose the output's line ending style.
    eol_prompt: Option<PendingSave>,
    /// The Heuristics Alignment window, while it is open.
    heuristics: Option<HeuristicsDialog>,
    /// The request for fold ranges in flight; one at a time.
    fold_request: Option<RequestId>,
    /// The language the fold ranges of the session were asked for; `None` asks again for every document.
    folds_language: Option<String>,
    /// D changed since its fold ranges were asked for.
    output_folds_stale: bool,
}

impl KdiffApp {
    pub fn new(ctx: &egui::Context, client: Client, startup: Startup) -> KdiffApp {
        let repaint_ctx = ctx.clone();
        client.set_waker(Box::new(move || repaint_ctx.request_repaint()));
        egui_extras::install_image_loaders(ctx);
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
        let mut app = KdiffApp {
            client,
            config_path,
            config,
            config_writable,
            form,
            form_open: true,
            error: None,
            status: None,
            engine_stopped: false,
            mirror: None,
            language: LanguageState::default(),
            panes: PaneState::default(),
            pending,
            window_enlarged: false,
            settings: None,
            font_scan: FontScan::default(),
            applied_view: None,
            frame_radius: None,
            title_bar_refreshed: false,
            window_title: WINDOW_TITLE.to_string(),
            current_version: update_check::current_version(),
            update_check: None,
            shortcut_install: None,
            update_window: None,
            update_cache: CommonMarkCache::default(),
            dialogs: DialogWindows::default(),
            inset_prompt: None,
            eol_prompt: None,
            heuristics: None,
            fold_request: None,
            folds_language: None,
            output_folds_stale: false,
        };
        ctx.options_mut(|options| options.zoom_with_keyboard = false);
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
        let scanned = self
            .font_scan
            .families()
            .and_then(|families| families.iter().find(|family| family.name == name))
            .map(|family| family.data.clone());
        let found = scanned.or_else(|| fonts::load_family(name));
        if found.is_none() {
            log::warn!("font family {name:?} is not installed; using the built-in font");
        }
        found
    }

    /// Removes what the last update left behind and, when due, looks for a newer release. Not part of `new`, so tests never touch the network.
    pub fn begin_startup_update_tasks(&mut self, ctx: &egui::Context) {
        self_update::clean_up_previous_update();
        let updates = &self.config.updates;
        if updates.check_automatically && update_check::is_due(updates.last_check_unix, update_check::now_unix()) {
            self.begin_update_check(ctx, false);
        }
    }

    fn begin_update_check(&mut self, ctx: &egui::Context, manual: bool) {
        if self.update_check.is_some() || matches!(self.update_window, Some(UpdateWindow::Installing { .. })) {
            return;
        }
        let repaint_ctx = ctx.clone();
        self.update_check = Some(UpdateCheck::start(update_check::release_url(), manual, move || repaint_ctx.request_repaint()));
    }

    fn poll_update_check(&mut self) {
        let Some(check) = self.update_check.as_ref() else {
            return;
        };
        let Some(result) = check.poll() else {
            return;
        };
        let manual = check.manual;
        self.update_check = None;
        match result {
            Ok(release) => {
                self.config.updates.last_check_unix = update_check::now_unix();
                self.save_config(false);
                let outcome = update_check::evaluate(&self.current_version, release, self.config.updates.skipped_version.as_deref(), manual);
                match outcome {
                    CheckOutcome::Available(release) => self.update_window = Some(UpdateWindow::Available(release)),
                    CheckOutcome::UpToDate if manual => {
                        self.update_window = Some(UpdateWindow::Message {
                            title: "No Update Available".to_string(),
                            text: format!("Kdiff4 {} is the newest version.", self.current_version),
                        });
                    }
                    CheckOutcome::UpToDate | CheckOutcome::Skipped => {}
                }
            }
            Err(error) => {
                log::warn!("{error}");
                if manual {
                    self.update_window = Some(UpdateWindow::Message {
                        title: "Update Check Failed".to_string(),
                        text: error.to_string(),
                    });
                }
            }
        }
    }

    fn begin_shortcut_install(&mut self, ctx: &egui::Context) {
        if self.shortcut_install.is_some() {
            return;
        }
        let repaint_ctx = ctx.clone();
        self.shortcut_install = Some(ShortcutInstall::start(move || repaint_ctx.request_repaint()));
    }

    fn poll_shortcut_install(&mut self) {
        let Some(install) = self.shortcut_install.as_ref() else {
            return;
        };
        let Some(result) = install.poll() else {
            return;
        };
        self.shortcut_install = None;
        self.finish_shortcut_install(result);
    }

    fn finish_shortcut_install(&mut self, result: Result<PathBuf, ShortcutError>) {
        let text = match result {
            Ok(link) => format!("Kdiff4 is now in the Start menu.

Shortcut: {}", link.display()),
            Err(error) => {
                log::warn!("{error}");
                format!("Cannot add Kdiff4 to the Start menu: {error}")
            }
        };
        self.update_window = Some(UpdateWindow::Message { title: "Start Menu Shortcut".to_string(), text });
    }

    fn update_message(&mut self, title: &str, error: &UpdateError) {
        self.update_window = Some(UpdateWindow::Message {
            title: title.to_string(),
            text: error.to_string(),
        });
    }

    fn start_install(&mut self, ctx: &egui::Context, release: ReleaseInfo) {
        let Some(asset) = release.asset_for_this_platform().cloned() else {
            self.open_release_page(&release);
            return;
        };
        let overridden = std::env::var_os(update_check::URL_OVERRIDE_ENV).is_some();
        if !overridden && !asset.url.starts_with("https://") {
            self.update_message("Update Failed", &UpdateError::Download(format!("refusing to download over an insecure address: {}", asset.url)));
            return;
        }
        let repaint_ctx = ctx.clone();
        match UpdateInstall::start(asset.url, asset.size, move || repaint_ctx.request_repaint()) {
            Ok(install) => self.update_window = Some(UpdateWindow::Installing { release, install }),
            Err(error) => self.update_message("Update Failed", &error),
        }
    }

    fn open_release_page(&mut self, release: &ReleaseInfo) {
        if !release.page_url.starts_with("https://") && !release.page_url.starts_with("http://") {
            self.report_error(format!("Not opening the release page: {:?} is not a web address", release.page_url));
            return;
        }
        if let Err(error) = config::open_command(Path::new(&release.page_url)).spawn() {
            self.report_error(format!("Cannot open {}: {error}", release.page_url));
        }
    }

    fn finish_install(&mut self, ctx: &egui::Context, exe: &Path, result: Result<(), UpdateError>) {
        match result {
            Ok(()) => match self_update::relaunch(exe, std::env::args_os().skip(1).collect()) {
                Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Err(error) => {
                    self.update_window = Some(UpdateWindow::Message {
                        title: "Update Installed".to_string(),
                        text: format!("The new version is installed, but it did not start: {error}. Start Kdiff4 again by hand."),
                    });
                }
            },
            Err(UpdateError::Canceled) => {}
            Err(error) => self.update_message("Update Failed", &error),
        }
    }

    fn show_update_window(&mut self, ctx: &egui::Context) {
        self.poll_update_check();
        self.poll_shortcut_install();
        let has_unsaved_changes = self.mirror.as_ref().and_then(Mirror::output).is_some_and(|output| output.modified);
        let mut choice = None;
        let mut finished = None;
        let mut dismissed = false;
        match self.update_window.as_ref() {
            None => return,
            Some(UpdateWindow::About) => {
                let spec = DialogSpec { id: "about_dialog", title: "About Kdiff4", width: about_dialog::DIALOG_WIDTH, closable: true };
                let mut check_for_updates = false;
                let closed = self.dialogs.show(ctx, &spec, |ui| check_for_updates |= about_dialog::show(ui, self.update_check.is_some()));
                if check_for_updates {
                    self.update_window = None;
                    self.begin_update_check(ctx, true);
                } else {
                    dismissed = closed;
                }
            }
            Some(UpdateWindow::Available(release)) => {
                let offer = InstallOffer {
                    can_install: release.asset_for_this_platform().is_some(),
                    has_unsaved_changes,
                };
                let spec = DialogSpec { id: UPDATE_DIALOG_ID, title: "Update Available", width: update_dialog::DIALOG_WIDTH, closable: true };
                let closed = self.dialogs.show(ctx, &spec, |ui| {
                    choice = update_dialog::show_available(ui, release, &self.current_version, offer.clone(), &mut self.update_cache);
                });
                if choice.is_none() && closed {
                    choice = Some(AvailableAction::RemindLater);
                }
            }
            Some(UpdateWindow::Installing { release, install }) => match install.poll() {
                Some(result) => finished = Some(result),
                None => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                    let downloaded = install.downloaded.load(std::sync::atomic::Ordering::Relaxed);
                    let total = install.total.load(std::sync::atomic::Ordering::Relaxed);
                    let spec = DialogSpec { id: UPDATE_DIALOG_ID, title: "Updating Kdiff4", width: update_dialog::DIALOG_WIDTH, closable: false };
                    self.dialogs.show(ctx, &spec, |ui| {
                        if update_dialog::show_progress(ui, &release.version, install.fraction(), downloaded, total) {
                            install.cancel();
                        }
                    });
                }
            },
            Some(UpdateWindow::Message { title, text }) => {
                let spec = DialogSpec { id: UPDATE_DIALOG_ID, title, width: update_dialog::DIALOG_WIDTH, closable: true };
                let closed = self.dialogs.show(ctx, &spec, |ui| dismissed = update_dialog::show_message(ui, title, text));
                dismissed |= closed;
            }
        }
        if dismissed {
            self.update_window = None;
        }
        if let Some(result) = finished {
            if let Some(UpdateWindow::Installing { install, .. }) = self.update_window.take() {
                self.finish_install(ctx, &install.exe, result);
            }
        }
        if let Some(choice) = choice {
            if let Some(UpdateWindow::Available(release)) = self.update_window.take() {
                self.handle_update_choice(ctx, choice, release);
            }
        }
    }

    fn handle_update_choice(&mut self, ctx: &egui::Context, choice: AvailableAction, release: ReleaseInfo) {
        match choice {
            AvailableAction::Install => self.start_install(ctx, release),
            AvailableAction::OpenPage => self.open_release_page(&release),
            AvailableAction::RemindLater => {}
            AvailableAction::SkipVersion => {
                self.config.updates.skipped_version = Some(release.version.to_string());
                self.save_config(false);
            }
        }
    }

    fn open_settings(&mut self, ctx: &egui::Context) {
        if self.settings.is_none() {
            self.font_scan.start(ctx);
            let options = self.mirror.as_ref().map(Mirror::options);
            self.settings = Some(SettingsDialog::new(&self.config.view, options));
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
        self.pending.values().any(|kind| *kind == Pending::Open)
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
        self.panes.find.stale = true;
        match message {
            HostMessage::Event(Event::Snapshot(snapshot)) => {
                let opening = self.is_opening();
                let initial_region = snapshot.current_region.clone();
                if opening {
                    self.panes = PaneState {
                        merge_fraction: self.panes.merge_fraction.clone(),
                        ..PaneState::default()
                    };
                    self.status = None;
                }
                if let Some(dialog) = self.heuristics.as_mut() {
                    dialog.forget_proposal();
                }
                let mut mirror = Mirror::from_snapshot(snapshot, self.view().align_output);
                if let Some(previous) = self.mirror.as_ref() {
                    mirror.keep_folds_from(previous);
                }
                self.language.follow(&mirror);
                self.mirror = Some(mirror);
                self.folds_language = None;
                self.panes.void_row = None;
                if opening && let Some(region) = initial_region {
                    self.jump_to(region);
                }
                self.form_open = false;
                self.error = None;
                if !self.window_enlarged {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
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
                let asked = match &delta.change {
                    Change::AlignmentsChanged(alignments) => alignments.ask,
                    _ => None,
                };
                let edits_output = matches!(delta.change, Change::OutputEdited { .. });
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
                        self.output_folds_stale |= edits_output;
                        if let Some(offset) = changed_end
                            && let Some(lines) = self.mirror.as_ref().and_then(|mirror| lines_of(mirror, Document::D))
                        {
                            self.panes.set_caret(Document::D, Cursor::at(editing::position_of(lines, offset)), None);
                            self.panes.reveal_caret = true;
                        }
                        if let Some(region) = chosen_region {
                            self.keep_caret_in_region(region);
                        }
                        if let Some(number) = asked {
                            self.inset_prompt = self.mirror.as_ref().and_then(|mirror| mirror.alignments().insets.iter().find(|choice| choice.number == number).cloned());
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
            HostMessage::Event(Event::HeuristicProposal(proposal)) => {
                let asked = proposal.cause.as_ref().is_some_and(|cause| self.pending.get(cause) == Some(&Pending::HeuristicPropose));
                if asked && let Some(dialog) = self.heuristics.as_mut() {
                    dialog.set_proposal(proposal);
                }
            }
            HostMessage::Event(Event::FoldRanges(found)) => {
                if found.cause.is_some() && found.cause == self.fold_request {
                    self.fold_request = None;
                }
                if let Some(mirror) = self.mirror.as_mut() {
                    self.output_folds_stale |= mirror.set_fold_ranges(found);
                }
            }
            HostMessage::Response(Response { id, result }) => self.handle_response(id, result),
        }
    }


    /// Asks for the fold ranges of every document when the session or its language is new, and for those of D after it changed. One request is in flight at a time.
    fn request_fold_ranges(&mut self) {
        if self.mirror.is_none() || self.fold_request.is_some() || self.engine_stopped {
            return;
        }
        let language = self.language.current().name().to_string();
        let documents = if self.folds_language.as_deref() != Some(language.as_str()) {
            Document::ALL.to_vec()
        } else if self.output_folds_stale {
            vec![Document::D]
        } else {
            return;
        };
        self.folds_language = Some(language.clone());
        self.output_folds_stale = false;
        self.fold_request = self.send(Command::FindFoldRanges { language, documents }, Pending::FoldRanges);
    }


    /// The pane folding acts on: the one with the caret, or else the scroll leader.
    fn fold_pane(&self) -> Document {
        match self.panes.caret {
            Some((document, _)) => document,
            None => self.panes.scroll_leader(self.mirror.as_ref().is_some_and(|mirror| mirror.is_aligned())),
        }
    }


    /// Folds the innermost range around the caret that is not folded yet, and puts the caret at the end of its header line, before the Fold Ellipsis.
    fn fold_at_caret(&mut self) {
        let document = self.fold_pane();
        let Some(mirror) = self.mirror.as_mut() else { return };
        let line = self.panes.caret_in(document.clone()).map_or(0, |cursor| cursor.head.line);
        let Some(range) = mirror.folds(document.clone()).innermost_unfolded(line) else {
            return;
        };
        if mirror.change_folds(document.clone(), |folds| folds.fold(range.clone())) && self.panes.caret_in(document.clone()).is_some() {
            self.caret_to_line_end(document, range.start);
        }
    }


    /// Unfolds the fold whose header has the caret, or else the innermost fold around it.
    fn unfold_at_caret(&mut self) {
        let document = self.fold_pane();
        let line = self.panes.caret_in(document.clone()).map_or(0, |cursor| cursor.head.line);
        if let Some(mirror) = self.mirror.as_mut() {
            mirror.change_folds(document, |folds| folds.unfold_at(line));
        }
    }


    fn fold_all(&mut self, fold: bool) {
        let document = self.fold_pane();
        if let Some(mirror) = self.mirror.as_mut() {
            mirror.change_folds(document.clone(), |folds| if fold { folds.fold_all() } else { folds.unfold_all() });
        }
        self.park_caret_out_of_folds(document);
    }


    /// Folds or unfolds the range whose header is `line`, as its button in the fold column does.
    fn toggle_fold(&mut self, document: Document, line: usize) {
        let Some(mirror) = self.mirror.as_mut() else { return };
        let folds = mirror.folds(document.clone());
        if folds.is_folded(line) {
            mirror.change_folds(document, |folds| folds.unfold_at(line));
            return;
        }
        if let Some(range) = folds.range_at(line) {
            mirror.change_folds(document.clone(), |folds| folds.fold(range));
            self.park_caret_out_of_folds(document);
        }
    }


    /// A fold that now hides the caret in `document` would unfold again at once, so the caret goes to the end of the header of the outermost fold that hides it.
    fn park_caret_out_of_folds(&mut self, document: Document) {
        let Some(cursor) = self.panes.caret_in(document.clone()) else { return };
        let header = self.mirror.as_ref().and_then(|mirror| mirror.folds(document.clone()).outermost_hiding(cursor.head.line));
        if let Some(header) = header {
            self.caret_to_line_end(document, header);
        }
    }


    fn caret_to_line_end(&mut self, document: Document, line: usize) {
        let Some(lines) = self.mirror.as_ref().and_then(|mirror| lines_of(mirror, document.clone())) else { return };
        let position = editing::Position {
            line: line.clone(),
            column: lines.line_chars(line),
        };
        self.panes.set_caret(document, Cursor::at(position), None);
        self.panes.reveal_caret = true;
    }


    /// The caret entered a fold, by keys, a find or a jump: the folds that hide it unfold. On the closing token shown after a Fold Ellipsis it stays folded.
    fn unfold_around_caret(&mut self) {
        let Some((document, cursor)) = self.panes.caret else { return };
        let Some(mirror) = self.mirror.as_mut() else { return };
        let line = cursor.head.line;
        let text = lines_of(mirror, document.clone()).map_or("", |lines| lines.line(line));
        if folding::on_shown_closing(mirror.folds(document.clone()), line, text, cursor.head.column) {
            return;
        }
        if mirror.change_folds(document, |folds| folds.reveal(line..line)) {
            self.panes.void_row = None;
        }
    }

    /// Scrolls every pane to `region`, with context rows above it, unless every pane already shows all of it, and puts the caret, hidden, at its start in the pane that has it (A when none does).
    fn jump_to(&mut self, region: usize) {
        if let Some(mirror) = self.mirror.as_mut() {
            mirror.reveal_region(region);
        }
        self.panes.jump = Some(region);
        let document = self.panes.caret.map_or(Document::A, |(document, _)| document);
        self.caret_to_region(document, region, true);
    }

    /// Puts the caret at the start of `region` in `document`; `hidden` keeps it from being drawn until it moves again.
    fn caret_to_region(&mut self, document: Document, region: usize, hidden: bool) {
        let target = self.mirror.as_ref().and_then(|mirror| region_start_caret(mirror, document.clone(), region));
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
                    Pending::Save => self.mirror.as_ref().and_then(Mirror::output).and_then(|output| output.path.as_ref()).map(|path| {
                        log::info!("saved {}", path.display());
                        Status {
                            text: format!("Saved {}", path.display()),
                            is_error: false,
                        }
                    }),
                    Pending::Edit | Pending::Resync | Pending::UndoRedo | Pending::Navigated | Pending::SelectRegion | Pending::SetOutputEol => self.status.take(),
                    Pending::Navigate(to) => Some(Status {
                        text: format!("No {}", navigation_noun(to)),
                        is_error: false,
                    }),
                    Pending::Open | Pending::LoadSource(_) | Pending::Choose(_) | Pending::ComparisonOptions => self.mirror.as_ref().map(|mirror| Status {
                        text: mirror.difference_summary(),
                        is_error: false,
                    }),
                    Pending::SetOutputPath => None,
                    Pending::AlignmentEdit => self.mirror.as_ref().filter(|mirror| mirror.alignments().changed).map(|_| Status {
                        text: ALIGNMENTS_CHANGED_HINT.to_string(),
                        is_error: false,
                    }),
                    Pending::Reload => self.mirror.as_ref().map(|mirror| Status {
                        text: format!("Reloaded. {}", mirror.difference_summary()),
                        is_error: false,
                    }),
                    Pending::ApplyAlignments | Pending::HeuristicApply(..) => self.mirror.as_ref().map(|mirror| Status {
                        text: format!("Alignments applied. {}", mirror.difference_summary()),
                        is_error: false,
                    }),
                    Pending::HeuristicPropose => self.status.take(),
                    Pending::FoldRanges => self.status.take(),
                };
                return;
            }
            Err(error) => error,
        };
        match kind {
            Pending::Open => self.error = Some(error.message),
            Pending::FoldRanges => {
                self.fold_request = None;
                log::warn!("cannot find the fold ranges: {}", error.message);
            }
            Pending::Edit => {
                let rejected = self.mirror.as_ref().is_some_and(|mirror| mirror.edit_rejected(&id));
                self.report_error(format!("The engine rejected an edit: {}", error.message));
                if rejected {
                    self.request_resync();
                }
            }
            Pending::LoadSource(document) => self.report_error(format!("Cannot load {}: {}", document.letter(), error.message)),
            Pending::Reload if error.kind == ErrorKind::NeedsConfirmation => {
                if open_dialog::confirm_reload() {
                    self.send(Command::Reload { confirmed: true }, Pending::Reload);
                }
            }
            Pending::Reload => self.report_error(format!("Cannot reload: {}", error.message)),
            Pending::SetOutputPath => self.report_error(format!("Cannot change the output path: {}", error.message)),
            Pending::SetOutputEol => self.report_error(format!("Cannot change the line ending style: {}", error.message)),
            Pending::Save => self.report_error(format!("Cannot save: {}", error.message)),
            Pending::Choose(_) => self.report_error(format!("Cannot choose: {}", error.message)),
            Pending::Resync => self.report_error(format!("Cannot resync with the engine: {}", error.message)),
            Pending::UndoRedo => self.report_error(format!("Cannot undo or redo: {}", error.message)),
            Pending::Navigate(_) | Pending::Navigated => self.report_error(format!("Cannot navigate: {}", error.message)),
            Pending::SelectRegion => self.report_error(format!("Cannot select the region: {}", error.message)),
            Pending::ComparisonOptions => self.report_error(format!("Cannot apply the comparison options: {}", error.message)),
            Pending::AlignmentEdit => self.report_error(format!("Cannot change the alignments: {}", error.message)),
            Pending::ApplyAlignments if error.kind == ErrorKind::NeedsConfirmation => {
                if open_dialog::confirm_apply_alignments() {
                    self.send(Command::ApplyAlignments { confirmed: true }, Pending::ApplyAlignments);
                }
            }
            Pending::ApplyAlignments => self.report_error(format!("Cannot apply the alignments: {}", error.message)),
            Pending::HeuristicPropose => match self.heuristics.as_mut() {
                Some(dialog) => dialog.set_error(error.message),
                None => self.report_error(format!("Cannot find heuristic alignments: {}", error.message)),
            },
            Pending::HeuristicApply(alignments, within) if error.kind == ErrorKind::NeedsConfirmation => {
                if open_dialog::confirm_apply_alignments() {
                    self.send(Command::ApplyHeuristicAlignments { alignments: alignments.clone(), confirmed: true, within }, Pending::HeuristicApply(alignments, within));
                }
            }
            Pending::HeuristicApply(..) => self.report_error(format!("Cannot apply the heuristic alignments: {}", error.message)),
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

    /// The pane the find bar searches: the one with the caret, else the first document.
    fn find_target(&self) -> Option<Document> {
        self.panes
            .caret
            .map(|(document, _)| document)
            .or_else(|| self.mirror.as_ref().and_then(|mirror| mirror.documents().first()).map(|text| text.document))
    }

    /// Shows the find bar with the keyboard, searching for the selected text if it is on one line.
    fn open_find(&mut self) {
        let selected = self.panes.caret.and_then(|(document, cursor)| {
            let (start, end) = cursor.range();
            let lines = lines_of(self.mirror.as_ref()?, document)?;
            let text: String = lines.line(start.line).chars().skip(start.column).take(end.column.saturating_sub(start.column)).collect();
            (start.line == end.line && !text.is_empty()).then_some(text)
        });
        if let Some(text) = selected {
            self.panes.find.query = text;
        }
        self.panes.find.open = true;
        self.panes.find.focus = true;
    }

    /// Finds the matches in the searched pane again if needed, and follows the selection.
    fn refresh_find(&mut self) {
        if !self.panes.find.open {
            return;
        }
        let Some(document) = self.find_target() else { return };
        let Some(lines) = self.mirror.as_ref().and_then(|mirror| lines_of(mirror, document)) else { return };
        self.panes.find.refresh(document, lines);
        let selection = self.panes.caret_in(document);
        self.panes.find.follow(selection);
    }

    fn select_match(&mut self, ctx: &egui::Context, document: Document, index: usize) {
        let Some(found) = self.panes.find.matches.get(index).copied() else { return };
        self.panes.set_caret(document, found.cursor(), None);
        self.panes.strip_focused = false;
        self.panes.reveal_caret = true;
        self.panes.find.current = Some(index);
        ctx.request_repaint();
    }

    /// Goes to the next or previous match in the pane with the caret. With nothing to search for, opens the find bar.
    fn find_step(&mut self, ctx: &egui::Context, forward: bool) {
        if self.panes.find.query.is_empty() {
            self.open_find();
            return;
        }
        self.panes.find.open = true;
        self.refresh_find();
        let Some(document) = self.panes.find.document else { return };
        let cursor = self.panes.caret_in(document).unwrap_or_default();
        let matches = &self.panes.find.matches;
        let index = if forward { find::next_after(matches, cursor) } else { find::previous_before(matches, cursor) };
        if let Some(index) = index {
            self.select_match(ctx, document, index);
        }
    }

    fn handle_find_event(&mut self, ctx: &egui::Context, event: FindEvent) {
        match event {
            FindEvent::Changed => {
                self.refresh_find();
                let Some(document) = self.panes.find.document else { return };
                let from = self.panes.caret_in(document).map_or_else(Position::default, |cursor| cursor.range().0);
                if let Some(index) = find::first_from(&self.panes.find.matches, from) {
                    self.select_match(ctx, document, index);
                }
            }
            FindEvent::Next => self.find_step(ctx, true),
            FindEvent::Previous => self.find_step(ctx, false),
            FindEvent::Close => {
                self.panes.find.open = false;
                self.panes.find.clear();
            }
        }
    }

    fn is_merge(&self) -> bool {
        self.mirror.as_ref().is_some_and(Mirror::is_merge)
    }

    /// The sources disagree on the line ending style and nobody has chosen one for the output.
    fn eol_conflicts(&self) -> bool {
        self.mirror.as_ref().and_then(Mirror::output).is_some_and(|output| output.eol == EolChoice::Conflict)
    }

    /// Without an output path, Save asks for one like Save As.
    fn save(&mut self) {
        let has_path = self.mirror.as_ref().and_then(Mirror::output).is_some_and(|output| output.path.is_some());
        if !has_path {
            self.save_as();
        } else if self.eol_conflicts() {
            self.eol_prompt = Some(PendingSave::Save);
        } else {
            self.send(Command::Save, Pending::Save);
        }
    }

    /// The native Save dialog asks before overwriting an existing file.
    fn save_as(&mut self) {
        if !self.is_merge() {
            return;
        }
        if let Some(path) = open_dialog::pick_file(Document::D, self.config.recent.last_dir.as_deref()) {
            self.remember_dir(&path);
            if self.eol_conflicts() {
                self.eol_prompt = Some(PendingSave::SaveAs(path));
            } else {
                self.send(Command::SaveAs(path), Pending::Save);
            }
        }
    }

    fn handle_pane_action(&mut self, ctx: &egui::Context, action: PaneAction) {
        match action {
            PaneAction::FocusStrip => self.panes.strip_focused = true,
            PaneAction::Press {
                document,
                position,
                void_row,
                extend,
            } => {
                self.panes.strip_focused = false;
                let cursor = match self.panes.caret {
                    Some((current, cursor)) if extend && current == document => Cursor {
                        anchor: cursor.anchor,
                        head: position,
                    },
                    _ => Cursor::at(position),
                };
                self.panes.set_caret(document, cursor, void_row);
                self.panes.reveal_caret = false;
                if let Some(focused) = ctx.memory(|memory| memory.focused()) {
                    ctx.memory_mut(|memory| memory.surrender_focus(focused));
                }
            }
            PaneAction::SelectWord(document, position) => {
                self.panes.strip_focused = false;
                if let Some(lines) = self.mirror.as_ref().and_then(|mirror| lines_of(mirror, document.clone())) {
                    self.panes.set_caret(document, editing::word_at(lines, position), None);
                }
            }
            PaneAction::SelectLineText(document, line) => {
                self.panes.strip_focused = false;
                if let Some(lines) = self.mirror.as_ref().and_then(|mirror| lines_of(mirror, document.clone())) {
                    self.panes.set_caret(document, editing::line_text_at(lines, line), None);
                }
            }
            PaneAction::SelectLine(document, line) => {
                self.panes.strip_focused = false;
                if let Some(lines) = self.mirror.as_ref().and_then(|mirror| lines_of(mirror, document.clone())) {
                    self.panes.set_caret(document, editing::line_at(lines, line), None);
                }
            }
            PaneAction::Drag { document, position, void_row } => {
                if let Some((current, cursor)) = self.panes.caret
                    && current == document
                    && (cursor.head != position || self.panes.void_row != void_row)
                {
                    self.panes.set_caret(
                        document,
                        Cursor {
                            anchor: cursor.anchor,
                            head: position,
                        },
                        void_row,
                    );
                    self.panes.reveal_caret = true;
                }
            }
            PaneAction::SelectRegion(document, region) => {
                self.panes.strip_focused = false;
                self.caret_to_region(document, region, false);
                self.panes.reveal_caret = false;
                if let Some(focused) = ctx.memory(|memory| memory.focused()) {
                    ctx.memory_mut(|memory| memory.surrender_focus(focused));
                }
                if let Some(base) = self.mirror.as_ref().and_then(Mirror::navigation_base) {
                    self.send(Command::SetCurrentRegion { base, region: Some(region) }, Pending::SelectRegion);
                }
            }
            PaneAction::Copy => self.copy_selection(ctx),
            PaneAction::ToggleFold(document, line) => self.toggle_fold(document, line),
            PaneAction::MarkSelection(_) => self.mark_selected_lines(),
            PaneAction::MarkAnchor(document, position) => self.mark_anchor(document, position),
            PaneAction::RemoveAlignmentMark(document, number) => {
                self.send(Command::RemoveAlignmentMark { document, number }, Pending::AlignmentEdit);
            }
            PaneAction::SetAlignmentMarkLines(document, number, lines) => {
                self.send(Command::SetAlignmentMarkLines { document, number, lines }, Pending::AlignmentEdit);
            }
            PaneAction::RenumberAlignmentMark(document, number, to) => {
                self.send(Command::RenumberAlignmentMark { document, number, to }, Pending::AlignmentEdit);
            }
            PaneAction::PlaceAlignment(number, slot) => {
                self.send(Command::PlaceAlignment { number, slot }, Pending::AlignmentEdit);
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

    fn reload(&mut self) {
        self.send(Command::Reload { confirmed: false }, Pending::Reload);
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
        if self.mirror.is_none() || self.settings.is_some() || self.update_window.is_some() || self.eol_prompt.is_some() || self.form_open || ctx.text_edit_focused() {
            return;
        }
        let events = ctx.input(|input| input.events.clone());
        for event in events {
            match event {
                egui::Event::Key {
                    key: Key::A,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.command => self.select_all(),
                egui::Event::Key {
                    key, pressed: true, modifiers, ..
                } => {
                    if self.panes.strip_focused && modifiers.is_none() && matches!(key, Key::ArrowUp | Key::ArrowDown) {
                        self.move_strip_line(if key == Key::ArrowUp { -1 } else { 1 });
                    } else if let Some(motion) = motion_for(key, modifiers, self.panes.page_rows) {
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

    /// Moves the caret to the next line of the overview strip above (`direction` -1) or below (1), keeping its column.
    fn move_strip_line(&mut self, direction: isize) {
        let Some(mirror) = self.mirror.as_ref() else { return };
        let Some((document, position, void_row)) = overview::neighbor_press(mirror, &self.panes, direction) else {
            return;
        };
        let Some(lines) = lines_of(mirror, document.clone()) else { return };
        let position = editing::clamp(lines, position);
        self.panes.set_caret(document, Cursor::at(position), void_row);
        self.panes.reveal_caret = true;
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
        let caret = Caret {
            cursor,
            void_row: self.panes.void_row.clone(),
        };
        let mut moved = move_by_rows(lines, rows, caret.clone(), motion.clone(), extend);
        if self.panes.strip_focused && !overview::stays_on_rows(mirror, &self.panes, document.clone(), moved.0, moved.1.clone()) {
            let fallback = match motion {
                Motion::Right | Motion::WordRight => Motion::LineEnd,
                Motion::Left | Motion::WordLeft => Motion::LineStart,
                _ => return,
            };
            moved = move_by_rows(lines, rows, caret, fallback, extend);
            if !overview::stays_on_rows(mirror, &self.panes, document.clone(), moved.0, moved.1.clone()) {
                return;
            }
        }
        self.panes.set_caret(document, moved.0, moved.1);
        self.panes.reveal_caret = true;
    }

    fn select_all(&mut self) {
        let document = match self.panes.caret {
            Some((document, _)) => document,
            None => return,
        };
        if let Some(lines) = self.mirror.as_ref().and_then(|mirror| lines_of(mirror, document.clone())) {
            let selection = match self.panes.caret {
                Some((_, cursor)) if self.panes.strip_focused => editing::line_text_at(lines, cursor.head.line),
                _ => editing::select_all(lines),
            };
            self.panes.set_caret(document, selection, None);
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
        self.panes.find.stale = true;
        let cursor = match self.panes.caret {
            Some((Document::D, cursor)) => cursor,
            _ => return,
        };
        let mirror = match self.mirror.as_mut() {
            Some(mirror) => mirror,
            None => return,
        };
        let Some(output) = mirror.output() else { return };
        if self.panes.strip_focused && changes_line_structure(output.lines(), cursor, self.panes.void_row.is_some(), &key) {
            return;
        }
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
        rows.region_of_row(text_view::caret_row(
            rows,
            Caret {
                cursor,
                void_row: self.panes.void_row.clone(),
            },
        ))
    }

    /// Asks the engine to move the current region from the caret's region.
    fn navigate(&mut self, to: Navigation) {
        let Some(base) = self.mirror.as_ref().and_then(Mirror::navigation_base) else { return };
        let from = self.caret_region();
        let include_alignments = self.view().alignment_stops;
        self.send(
            Command::Navigate {
                base,
                from,
                to,
                include_alignments,
            },
            Pending::Navigate(to),
        );
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

    /// Marks the lines the selection touches in A, B or C.
    fn mark_selected_lines(&mut self) {
        let Some((document, cursor)) = self.panes.caret else {
            self.report_error("Select lines in A, B or C to mark them for alignment".to_string());
            return;
        };
        let count = self.mirror.as_ref().and_then(|mirror| lines_of(mirror, document.clone())).map_or(0, |lines| lines.count());
        let lines = if document == Document::D || self.panes.void_row.is_some() {
            None
        } else {
            editing::selected_lines(cursor, count)
        };
        let lines = match (lines, self.mirror.as_ref()) {
            (Some(lines), Some(mirror)) => Some(through_folds(mirror.folds(document.clone()), lines)),
            (lines, _) => lines,
        };
        match lines {
            Some(lines) => {
                self.send(Command::MarkAlignment { document, lines }, Pending::AlignmentEdit);
            }
            None => self.report_error("Select lines in A, B or C to mark them for alignment".to_string()),
        }
    }

    /// An anchor before the line with the caret in A, B or C; with the caret past the last line, at the end of the text.
    fn mark_anchor_at_caret(&mut self) {
        let Some((document, cursor)) = self.panes.caret else {
            self.report_error("Put the caret in A, B or C to mark an anchor".to_string());
            return;
        };
        if document == Document::D {
            self.report_error("Put the caret in A, B or C to mark an anchor".to_string());
            return;
        }
        let count = self.mirror.as_ref().and_then(|mirror| lines_of(mirror, document.clone())).map_or(0, |lines| lines.count());
        self.mark_anchor(document, cursor.head.line.min(count));
    }

    /// The function or method around the caret's line in A, B or C, from its signature to its closing line.
    fn mark_function_at_caret(&mut self) {
        let Some((document, cursor)) = self.panes.caret.clone() else {
            self.report_error("Put the caret in a function in A, B or C to mark it".to_string());
            return;
        };
        if document == Document::D {
            self.report_error("Put the caret in a function in A, B or C to mark it".to_string());
            return;
        }
        let language = self.language.current().name().to_string();
        self.send(Command::MarkFunctionAlignment { document, line: cursor.head.line.clone(), language }, Pending::AlignmentEdit);
    }

    /// An anchor before line `position`: a position between two lines, which may be the end of the text.
    fn mark_anchor(&mut self, document: Document, position: usize) {
        self.send(
            Command::MarkAlignment {
                document,
                lines: position..position,
            },
            Pending::AlignmentEdit,
        );
    }

    fn apply_alignments(&mut self) {
        if self.mirror.as_ref().is_some_and(|mirror| mirror.alignments().changed) {
            self.send(Command::ApplyAlignments { confirmed: false }, Pending::ApplyAlignments);
        } else {
            self.status = Some(Status {
                text: "The marks give the alignments already applied".to_string(),
                is_error: false,
            });
        }
    }

    /// Opens the Heuristics Alignment window with the options it was last closed with, and asks the engine for its matches at once.
    fn open_heuristics(&mut self) {
        self.open_heuristics_window(None);
    }


    /// Opens the Heuristics Alignment window for what lies inside the innermost applied alignment around the caret.
    fn open_heuristics_inside(&mut self) {
        let number = match (self.panes.caret, self.mirror.as_ref()) {
            (Some((document, cursor)), Some(mirror)) if document != Document::D && self.panes.void_row.is_none() => {
                innermost_applied_alignment(mirror.alignments(), document, cursor.head.line, mirror.documents().len().min(3))
            }
            _ => None,
        };
        match number {
            Some(number) => self.open_heuristics_window(Some(number)),
            None => self.report_error(INSIDE_ALIGNMENT_HINT.to_string()),
        }
    }


    fn open_heuristics_window(&mut self, within: Option<u32>) {
        let Some(mirror) = self.mirror.as_ref() else { return };
        let options = HeuristicOptions {
            extractor: self.view().heuristic_extractor.clone(),
            ..self.config.heuristics.clone()
        };
        let documents = mirror.documents().len().min(3);
        let dialog = HeuristicsDialog::new(options, self.language.current().name(), documents);
        self.heuristics = Some(match within {
            Some(number) => dialog.inside(number),
            None => dialog,
        });
        self.find_heuristic_matches();
    }


    fn find_heuristic_matches(&mut self) {
        let Some(dialog) = self.heuristics.as_mut() else { return };
        dialog.start_search();
        let command = Command::ProposeHeuristicAlignments {
            language: dialog.language().to_string(),
            options: dialog.options.clone(),
            within: dialog.within(),
        };
        self.send(command, Pending::HeuristicPropose);
    }


    fn show_heuristics(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.heuristics.as_mut() else { return };
        let mut action = None;
        let spec = DialogSpec { id: "heuristics_dialog", title: "Heuristics Alignment", width: heuristics_dialog::DIALOG_WIDTH, closable: true };
        let closed = self.dialogs.show(ctx, &spec, |ui| action = dialog.show(ui));
        if action.is_none() && closed {
            action = Some(HeuristicsAction::Cancel);
        }
        let Some(action) = action else { return };
        self.config.heuristics = dialog.options.clone();
        let within = dialog.within();
        self.save_config(false);
        match action {
            HeuristicsAction::Find => self.find_heuristic_matches(),
            HeuristicsAction::Align(alignments) => {
                self.heuristics = None;
                self.apply_heuristics(alignments, within);
            }
            HeuristicsAction::Cancel => self.heuristics = None,
        }
    }


    fn apply_heuristics(&mut self, alignments: Vec<Vec<std::ops::Range<usize>>>, within: Option<u32>) {
        self.send(Command::ApplyHeuristicAlignments { alignments: alignments.clone(), confirmed: false, within }, Pending::HeuristicApply(alignments, within));
    }

    fn clear_alignments(&mut self) {
        if self.mirror.as_ref().is_some_and(|mirror| !mirror.alignments().marks.is_empty()) {
            self.send(Command::ClearAlignmentMarks, Pending::AlignmentEdit);
        } else {
            self.status = Some(Status {
                text: "There are no alignment marks".to_string(),
                is_error: false,
            });
        }
    }

    /// A, B and, in a 3-way merge, C: the documents a choice can take hunks from.
    fn choosable(&self) -> Vec<Document> {
        if !self.is_merge() {
            return Vec::new();
        }
        let count = self.mirror.as_ref().map_or(0, |mirror| mirror.documents().len());
        Document::ALL.iter().take(count.min(3)).cloned().collect()
    }

    /// Undo and redo act on D, wherever the caret is. While a text field has the keyboard, they are left to the field.
    fn undo_redo(&mut self, ctx: &egui::Context, command: Command) {
        if self.is_merge() && !ctx.text_edit_focused() {
            self.send(command, Pending::UndoRedo);
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if !ctx.text_edit_focused() {
            if ctx.input_mut(|input| input.consume_shortcut(&REDO_SHORTCUT)) {
                self.undo_redo(ctx, Command::Redo);
            }
            if self.mirror.is_some() {
                if ctx.input_mut(|input| input.consume_shortcut(&MARK_FUNCTION_SHORTCUT)) {
                    self.mark_function_at_caret();
                }
                if ctx.input_mut(|input| input.consume_shortcut(&MARK_LINES_SHORTCUT)) {
                    self.mark_selected_lines();
                }
                if ctx.input_mut(|input| input.consume_shortcut(&CLEAR_ALIGNMENTS_SHORTCUT)) {
                    self.clear_alignments();
                }
                if ctx.input_mut(|input| input.consume_shortcut(&APPLY_ALIGNMENTS_SHORTCUT)) {
                    self.apply_alignments();
                }
                if ctx.input_mut(|input| input.consume_shortcut(&HEURISTICS_INSIDE_SHORTCUT)) {
                    self.open_heuristics_inside();
                }
                if ctx.input_mut(|input| input.consume_shortcut(&HEURISTICS_SHORTCUT)) {
                    self.open_heuristics();
                }
            }
            if ctx.input_mut(|input| input.consume_shortcut(&UNDO_SHORTCUT)) {
                self.undo_redo(ctx, Command::Undo);
            }
            for document in self.choosable() {
                let shortcut = CHOOSE_SHORTCUTS.iter().find(|(choosable, _)| *choosable == document).map(|(_, shortcut)| shortcut.clone());
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
        if self.mirror.is_some() {
            if ctx.input_mut(|input| input.consume_shortcut(&FIND_PREVIOUS_SHORTCUT)) {
                self.find_step(ctx, false);
            }
            if ctx.input_mut(|input| input.consume_shortcut(&FIND_NEXT_SHORTCUT)) {
                self.find_step(ctx, true);
            }
            if ctx.input_mut(|input| input.consume_shortcut(&FIND_SHORTCUT)) {
                self.open_find();
            }
            if ctx.input_mut(|input| input.consume_shortcut(&RELOAD_SHORTCUT)) {
                self.reload();
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
        if ctx.input_mut(|input| input.consume_shortcut(&MINIMAP_SHORTCUT)) {
            self.toggle_minimap();
        }
        if ctx.input_mut(|input| input.consume_shortcut(&INDEPENDENT_SCROLL_SHORTCUT)) {
            self.toggle_independent_scroll();
        }
        if ctx.input_mut(|input| input.consume_shortcut(&IGNORE_WHITESPACE_SHORTCUT)) {
            self.toggle_ignore_whitespace();
        }
        if self.mirror.is_some() && !ctx.text_edit_focused() {
            if ctx.input_mut(|input| input.consume_shortcut(&FOLD_ALL_SHORTCUT)) {
                self.fold_all(true);
            }
            if ctx.input_mut(|input| input.consume_shortcut(&UNFOLD_ALL_SHORTCUT)) {
                self.fold_all(false);
            }
            if ctx.input_mut(|input| input.consume_shortcut(&FOLD_SHORTCUT)) {
                self.fold_at_caret();
            }
            if ctx.input_mut(|input| input.consume_shortcut(&UNFOLD_SHORTCUT)) {
                self.unfold_at_caret();
            }
        }
    }

    /// Asks where to show the alignment the last mark completed, when the documents disagree on its order. The answer is a `PlaceAlignment` unless the user keeps the suggestion, which is in effect already.
    fn show_inset_prompt(&mut self, ctx: &egui::Context) {
        let Some(choice) = self.inset_prompt.as_ref() else { return };
        let mut answer = None;
        let spec = DialogSpec { id: "inset_dialog", title: "Alignment Placement", width: inset_dialog::DIALOG_WIDTH, closable: true };
        let closed = self.dialogs.show(ctx, &spec, |ui| answer = inset_dialog::show(ui, choice));
        if answer.is_none() && closed {
            answer = Some(InsetAnswer::Keep);
        }
        if let Some(answer) = answer {
            self.answer_inset_prompt(answer);
        }
    }

    fn answer_inset_prompt(&mut self, answer: InsetAnswer) {
        let Some(choice) = self.inset_prompt.take() else { return };
        if let InsetAnswer::Place(slot) = answer {
            self.send(Command::PlaceAlignment { number: choice.number, slot }, Pending::AlignmentEdit);
        }
    }

    /// Asks for the output's line ending style when the user saved while the sources disagree. A style sets it and then saves; closing the window saves nothing.
    fn show_eol_prompt(&mut self, ctx: &egui::Context) {
        if self.eol_prompt.is_none() {
            return;
        }
        let mut answer = None;
        let spec = DialogSpec { id: "eol_dialog", title: "Line Endings", width: eol::DIALOG_WIDTH, closable: true };
        let closed = self.dialogs.show(ctx, &spec, |ui| answer = eol::show_dialog(ui));
        if answer.is_none() && closed {
            answer = Some(EolAnswer::Cancel);
        }
        if let Some(answer) = answer {
            self.answer_eol_prompt(answer);
        }
    }

    fn answer_eol_prompt(&mut self, answer: EolAnswer) {
        let Some(save) = self.eol_prompt.take() else { return };
        let EolAnswer::Use(style) = answer else { return };
        self.send(Command::SetOutputEol(style), Pending::SetOutputEol);
        match save {
            PendingSave::Save => self.send(Command::Save, Pending::Save),
            PendingSave::SaveAs(path) => self.send(Command::SaveAs(path), Pending::Save),
        };
    }

    fn show_settings(&mut self, ctx: &egui::Context) {
        let families = self.font_scan.families();
        let dialog = match self.settings.as_mut() {
            Some(dialog) => dialog,
            None => return,
        };
        let mut action = None;
        let spec = DialogSpec { id: "settings_dialog", title: "Settings", width: settings_dialog::DIALOG_WIDTH, closable: true };
        let closed = self.dialogs.show(ctx, &spec, |ui| {
            action = dialog.show(ui, families);
        });
        if action.is_none() && closed {
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
                    let reload = egui::Button::new("Reload").shortcut_text(ui.ctx().format_shortcut(&RELOAD_SHORTCUT));
                    if ui.add_enabled(has_session, reload).clicked() {
                        self.reload();
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
                    ui.separator();
                    let find = egui::Button::new("Find...").shortcut_text(ctx.format_shortcut(&FIND_SHORTCUT));
                    if ui.add_enabled(has_session, find).clicked() {
                        self.open_find();
                    }
                    let find_next = egui::Button::new("Find Next").shortcut_text(ctx.format_shortcut(&FIND_NEXT_SHORTCUT));
                    if ui.add_enabled(has_session, find_next).clicked() {
                        self.find_step(&ctx, true);
                    }
                    let find_previous = egui::Button::new("Find Previous").shortcut_text(ctx.format_shortcut(&FIND_PREVIOUS_SHORTCUT));
                    if ui.add_enabled(has_session, find_previous).clicked() {
                        self.find_step(&ctx, false);
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
                    let (marks, changed) = self
                        .mirror
                        .as_ref()
                        .map_or((0, false), |mirror| (mirror.alignments().marks.len(), mirror.alignments().changed));
                    let mark_lines = egui::Button::new("Mark Selected Lines for Alignment").shortcut_text(ui.ctx().format_shortcut(&MARK_LINES_SHORTCUT));
                    if ui.add_enabled(has_session, mark_lines).clicked() {
                        self.mark_selected_lines();
                    }
                    let mark_function = egui::Button::new("Mark Function for Alignment").shortcut_text(ui.ctx().format_shortcut(&MARK_FUNCTION_SHORTCUT));
                    if ui.add_enabled(has_session, mark_function).clicked() {
                        self.mark_function_at_caret();
                    }
                    if ui.add_enabled(has_session, egui::Button::new("Mark Alignment Anchor Before Line")).clicked() {
                        self.mark_anchor_at_caret();
                    }
                    let apply = egui::Button::new("Apply Alignments").shortcut_text(ui.ctx().format_shortcut(&APPLY_ALIGNMENTS_SHORTCUT));
                    if ui
                        .add_enabled(changed, apply)
                        .on_disabled_hover_text("The marks give the alignments already applied")
                        .clicked()
                    {
                        self.apply_alignments();
                    }
                    let heuristics = egui::Button::new("Heuristics Alignment…").shortcut_text(ui.ctx().format_shortcut(&HEURISTICS_SHORTCUT));
                    if ui
                        .add_enabled(has_session, heuristics)
                        .on_hover_text("Find functions, methods and types in the documents and align them, also where a document moved them")
                        .clicked()
                    {
                        self.open_heuristics();
                    }
                    let inside = egui::Button::new("Heuristics Alignment Inside…").shortcut_text(ui.ctx().format_shortcut(&HEURISTICS_INSIDE_SHORTCUT));
                    if ui
                        .add_enabled(has_session, inside)
                        .on_hover_text("Find and align the entities inside the applied alignment around the caret")
                        .clicked()
                    {
                        self.open_heuristics_inside();
                    }
                    let clear = egui::Button::new("Clear All Alignments").shortcut_text(ui.ctx().format_shortcut(&CLEAR_ALIGNMENTS_SHORTCUT));
                    if ui.add_enabled(marks > 0, clear).clicked() {
                        self.clear_alignments();
                    }
                    ui.separator();
                    let mut void_space = self.view().align_output;
                    if ui
                        .add_enabled(is_merge, egui::Checkbox::new(&mut void_space, VOID_TOGGLE_LABEL))
                        .on_hover_text(VOID_TOGGLE_HINT)
                        .clicked()
                    {
                        self.toggle_void_space();
                    }
                });
                ui.menu_button("View", |ui| {
                    let mut minimap = self.view().show_minimap;
                    let item = egui::Checkbox::new(&mut minimap, MINIMAP_TOGGLE_LABEL);
                    let shortcut = ui.ctx().format_shortcut(&MINIMAP_SHORTCUT);
                    if ui.add(item).on_hover_text(format!("{MINIMAP_TOGGLE_HINT} ({shortcut})")).clicked() {
                        self.toggle_minimap();
                    }
                    let mut independent = self.view().independent_scroll;
                    let item = egui::Checkbox::new(&mut independent, INDEPENDENT_SCROLL_TOGGLE_LABEL);
                    let shortcut = ui.ctx().format_shortcut(&INDEPENDENT_SCROLL_SHORTCUT);
                    if ui.add(item).on_hover_text(format!("{INDEPENDENT_SCROLL_TOGGLE_HINT} ({shortcut})")).clicked() {
                        self.toggle_independent_scroll();
                    }
                    ui.separator();
                    let folds = [
                        ("Fold", &FOLD_SHORTCUT, "Fold the innermost function, type or block around the caret"),
                        ("Unfold", &UNFOLD_SHORTCUT, "Unfold the fold at the caret"),
                        ("Fold All", &FOLD_ALL_SHORTCUT, "Fold the outermost functions, types and blocks of the pane with the caret"),
                        ("Unfold All", &UNFOLD_ALL_SHORTCUT, "Unfold everything in the pane with the caret"),
                    ];
                    for (index, (label, shortcut, hint)) in folds.into_iter().enumerate() {
                        let button = egui::Button::new(label).shortcut_text(ui.ctx().format_shortcut(shortcut));
                        if ui.add_enabled(has_session, button).on_hover_text(hint).clicked() {
                            match index {
                                0 => self.fold_at_caret(),
                                1 => self.unfold_at_caret(),
                                2 => self.fold_all(true),
                                _ => self.fold_all(false),
                            }
                        }
                    }
                    ui.separator();
                    let mut ignore_whitespace = self.ignores_whitespace();
                    let item = egui::Checkbox::new(&mut ignore_whitespace, IGNORE_WHITESPACE_TOGGLE_LABEL);
                    let shortcut = ui.ctx().format_shortcut(&IGNORE_WHITESPACE_SHORTCUT);
                    if ui.add_enabled(has_session, item).on_hover_text(format!("{IGNORE_WHITESPACE_TOGGLE_HINT} ({shortcut})")).clicked() {
                        self.toggle_ignore_whitespace();
                    }
                });
                ui.menu_button("Settings", |ui| {
                    let configure = egui::Button::new("Configure Kdiff4...").shortcut_text(ui.ctx().format_shortcut(&SETTINGS_SHORTCUT));
                    if ui.add(configure).clicked() {
                        let ctx = ui.ctx().clone();
                        self.open_settings(&ctx);
                    }
                    if cfg!(windows) {
                        ui.separator();
                        let installing = self.shortcut_install.is_some();
                        if ui.add_enabled(!installing, egui::Button::new(ADD_TO_START_MENU_LABEL)).on_hover_text(ADD_TO_START_MENU_HINT).clicked() {
                            let ctx = ui.ctx().clone();
                            self.begin_shortcut_install(&ctx);
                        }
                    }
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("About Kdiff4...").clicked() {
                        self.update_window = Some(UpdateWindow::About);
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Check for Updates...").clicked() {
                        let ctx = ui.ctx().clone();
                        self.begin_update_check(&ctx, true);
                    }
                    let mut automatic = self.config.updates.check_automatically;
                    if ui.checkbox(&mut automatic, "Check for Updates Automatically").clicked() {
                        self.config.updates.check_automatically = automatic;
                        self.save_config(false);
                    }
                });
            });
        });
    }

    /// The strip under the menu bar: the void space toggle of the output pane, the overview strip, minimap, independent scrolling and ignore whitespace toggles.
    fn show_toolbar(&mut self, ui: &mut egui::Ui) {
        let is_merge = self.is_merge();
        let on = self.view().align_output;
        let overview_on = self.view().show_overview_strip;
        let mut toggled = false;
        let mut overview_toggled = false;
        let mut minimap_toggled = false;
        let minimap_on = self.view().show_minimap;
        let independent_on = self.view().independent_scroll;
        let mut independent_toggled = false;
        let ignore_whitespace_on = self.ignores_whitespace();
        let mut ignore_whitespace_toggled = false;
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                let response = toolbar_toggle(ui, VOID_TOGGLE_LABEL, on, is_merge, paint_void_icon).on_hover_text(VOID_TOGGLE_HINT);
                if is_merge && response.clicked() {
                    toggled = true;
                }
                if toolbar_toggle(ui, OVERVIEW_TOGGLE_LABEL, overview_on, true, paint_overview_icon)
                    .on_hover_text(OVERVIEW_TOGGLE_HINT)
                    .clicked()
                {
                    overview_toggled = true;
                }
                if toolbar_toggle(ui, MINIMAP_TOGGLE_LABEL, minimap_on, true, paint_minimap_icon)
                    .on_hover_text(MINIMAP_TOGGLE_HINT)
                    .clicked()
                {
                    minimap_toggled = true;
                }
                if toolbar_toggle(ui, INDEPENDENT_SCROLL_TOGGLE_LABEL, independent_on, true, paint_independent_scroll_icon)
                    .on_hover_text(INDEPENDENT_SCROLL_TOGGLE_HINT)
                    .clicked()
                {
                    independent_toggled = true;
                }
                if toolbar_toggle(ui, IGNORE_WHITESPACE_TOGGLE_LABEL, ignore_whitespace_on, true, paint_ignore_whitespace_icon)
                    .on_hover_text(IGNORE_WHITESPACE_TOGGLE_HINT)
                    .clicked()
                {
                    ignore_whitespace_toggled = true;
                }
            });
        });
        if toggled {
            self.toggle_void_space();
        }
        if overview_toggled {
            self.toggle_overview_strip();
        }
        if minimap_toggled {
            self.toggle_minimap();
        }
        if independent_toggled {
            self.toggle_independent_scroll();
        }
        if ignore_whitespace_toggled {
            self.toggle_ignore_whitespace();
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

    /// Shows or hides the minimap, from the toolbar, the View menu or Ctrl+G. Saved at once, and followed by an open settings dialog.
    fn toggle_minimap(&mut self) {
        let on = !self.view().show_minimap;
        self.config.view.show_minimap = on;
        if let Some(applied) = self.applied_view.as_mut() {
            applied.show_minimap = on;
        }
        if let Some(dialog) = self.settings.as_mut() {
            dialog.set_show_minimap(on);
        }
        self.save_config(false);
    }

    /// Switches between independent and synced scrolling of the panes, from the toolbar, the View menu or Ctrl+Shift+U. Saved at once, and followed by an open settings dialog.
    fn toggle_independent_scroll(&mut self) {
        let on = !self.view().independent_scroll;
        self.config.view.independent_scroll = on;
        if let Some(applied) = self.applied_view.as_mut() {
            applied.independent_scroll = on;
        }
        if let Some(dialog) = self.settings.as_mut() {
            dialog.set_independent_scroll(on);
        }
        self.save_config(false);
    }

    fn ignores_whitespace(&self) -> bool {
        self.mirror.as_ref().is_some_and(|mirror| mirror.options().ignore_whitespace)
    }


    /// Turns ignoring whitespace in the character diff on or off, from the toolbar, the View menu or Ctrl+Shift+W. The engine recomputes the character diffs; D keeps its choices and edits.
    fn toggle_ignore_whitespace(&mut self) {
        let Some(mirror) = self.mirror.as_ref() else { return };
        let options = kdiff4_protocol::engine::ComparisonOptions {
            ignore_whitespace: !mirror.options().ignore_whitespace,
            ..mirror.options().clone()
        };
        if let Some(dialog) = self.settings.as_mut() {
            dialog.set_ignore_whitespace(options.ignore_whitespace.clone());
        }
        self.send(Command::SetComparisonOptions(options), Pending::ComparisonOptions);
    }


    /// The view settings in effect: the settings dialog's draft while it is open, otherwise the saved ones.
    fn view(&self) -> &ViewConfig {
        self.applied_view.as_ref().unwrap_or(&self.config.view)
    }

    fn pane_options(&self) -> PaneOptions {
        let view = self.view();
        let [r, g, b, a] = view.current_region_color;
        PaneOptions {
            line_numbers: view.show_line_numbers,
            gutters: view.show_gutters,
            current_region_color: egui::Color32::from_rgba_unmultiplied(r, g, b, a),
            jump_context_rows: view.jump_context_lines as usize,
            scroll_step_rows: view.scroll_step_lines as usize,
            drag_scroll_lines_per_second: view.drag_scroll_lines_per_second,
            middle_scroll_lines_per_second: view.middle_scroll_lines_per_second,
            scroll_motion: ScrollMotion {
                smooth: view.smooth_scrolling,
                duration: view.smooth_scroll_ms as f32 / 1000.0,
            },
            region_corner_radius: view.region_corner_radius,
            other_region_border_percent: view.other_region_border_percent,
            whitespace: view.whitespace.clone(),
            overview_strip: view.show_overview_strip,
            minimap: view.show_minimap,
            independent_scroll: view.independent_scroll,
            alignment_color: egui::Color32::from_rgba_unmultiplied(view.alignment_color[0], view.alignment_color[1], view.alignment_color[2], view.alignment_color[3]),
            inset_arrow_gap: view.inset_arrow_gap,
        }
    }

    /// Always shown and one line high, so the panes never move when a message comes or goes. Long messages are cut, with the full text in a tooltip. The language selector sits at the right end, with the output's line ending selector left of it; returns what was picked in them.
    fn show_status(&self, ui: &mut egui::Ui) -> StatusPicks {
        let eol = self.mirror.as_ref().and_then(Mirror::output).map(|output| output.eol.clone());
        status_bar(ui, self.status.as_ref(), self.language.current(), eol.as_ref())
    }

    /// egui sets the window theme on the first frame, before the window has focus, and Windows then keeps the old title bar colors until focus changes. Sending the theme again once the window has focus repaints the title bar.
    fn refresh_title_bar(&mut self, ctx: &egui::Context) {
        if self.title_bar_refreshed || !ctx.input(|input| input.viewport().focused.unwrap_or(false)) {
            return;
        }
        let theme = dialog_window::title_bar_theme(ctx.options(|options| options.theme_preference));
        ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(theme));
        self.title_bar_refreshed = true;
    }

    fn update_window_title(&mut self, ctx: &egui::Context) {
        let title = window_title(self.mirror.as_ref());
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
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
        if self.update_window.is_none() && self.eol_prompt.is_none() {
            self.handle_shortcuts(&ctx);
        }
        self.handle_pane_keys(&ctx);
        self.unfold_around_caret();
        self.request_fold_ranges();
        self.refresh_find();
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
        let mut find_events = Vec::new();
        let mut eol_pick = None;
        match self.mirror.as_ref() {
            None => {
                egui::CentralPanel::default().show(ui, |ui| {
                    form_action = self.form.show(ui, self.error.as_deref(), busy, false);
                });
            }
            Some(mirror) => {
                let picks = self.show_status(ui);
                if let Some(picked) = picks.language {
                    self.language.pick(picked, mirror);
                }
                eol_pick = picks.eol;
                let find_corner = ui.available_rect_before_wrap().right_top() + egui::vec2(-FIND_BAR_MARGIN.x, FIND_BAR_MARGIN.y);
                pane_actions = panes::show_session(ui, mirror, &mut self.panes, &pane_options);
                self.panes.reveal_caret = false;
                if self.panes.find.open {
                    find_events = find::show_bar(&ctx, find_corner, &mut self.panes.find);
                }
                if self.form_open {
                    let spec = DialogSpec { id: "open_dialog", title: "Open", width: open_dialog::FORM_MAX_WIDTH, closable: !busy };
                    let closed = self.dialogs.show(&ctx, &spec, |ui| {
                        ui.heading("Open");
                        form_action = self.form.show(ui, self.error.as_deref(), busy, !busy);
                    });
                    if closed {
                        form_action = Some(FormAction::Cancel);
                    }
                }
            }
        }
        for action in pane_actions {
            self.handle_pane_action(&ctx, action);
        }
        for event in find_events {
            self.handle_find_event(&ctx, event);
        }
        if let Some(action) = form_action {
            self.handle_form_action(action);
        }
        if let Some(style) = eol_pick {
            self.send(Command::SetOutputEol(style), Pending::SetOutputEol);
        }
        self.show_inset_prompt(&ctx);
        self.show_eol_prompt(&ctx);
        self.show_heuristics(&ctx);
        self.show_settings(&ctx);
        self.show_update_window(&ctx);
        self.dialogs.end_frame(frame.window_handle().ok().map(|handle| handle.as_raw()));
        self.update_window_frame(frame);
        self.refresh_title_bar(&ctx);
        self.update_window_title(&ctx);
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
    let cursor = if extend {
        Cursor {
            anchor: caret.cursor.anchor,
            head,
        }
    } else {
        Cursor::at(head)
    };
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

/// What the user picked in the status bar this frame.
#[derive(Default)]
struct StatusPicks {
    language: Option<Language>,
    eol: Option<EolStyle>,
}

/// The message at the left, cut to what fits beside the selectors at the right: the language, and the output's line ending style when there is an output.
fn status_bar(ui: &mut egui::Ui, status: Option<&Status>, language: &Language, eol: Option<&EolChoice>) -> StatusPicks {
    let mut picks = StatusPicks::default();
    egui::Panel::bottom("status_bar").show(ui, |ui| {
        let (text, color) = match status {
            Some(status) if status.is_error => (status.text.as_str(), ui.visuals().error_fg_color),
            Some(status) => (status.text.as_str(), theme::palette_of(ui).weak_text),
            None => ("Ready", theme::palette_of(ui).weak_text),
        };
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            picks.language = language::show_selector(ui, language);
            if let Some(eol) = eol {
                picks.eol = eol::show_selector(ui, eol);
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(text).color(color)).truncate());
            });
        });
    });
    picks
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
    ui.painter().rect(
        rect,
        theme::corner_radius_of(ui),
        fill,
        egui::Stroke::new(1.0, palette.pane_border),
        egui::StrokeKind::Inside,
    );
    let icon = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.top() + padding.y + TOOLBAR_ICON_SIZE / 2.0),
        egui::vec2(TOOLBAR_ICON_SIZE, TOOLBAR_ICON_SIZE),
    );
    paint_icon(ui, icon, color);
    ui.painter()
        .galley(egui::pos2(rect.center().x - galley.size().x / 2.0, icon.bottom() + TOOLBAR_ICON_GAP), galley, color);
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
        clip.line_segment(
            [egui::pos2(sum - square.bottom(), square.bottom()), egui::pos2(sum - square.top(), square.top())],
            egui::Stroke::new(1.0, stroke_color),
        );
        sum += spacing;
    }
    ui.painter()
        .rect_stroke(square, radius.min(4), egui::Stroke::new(1.0, stroke_color), egui::StrokeKind::Inside);
}

/// A tall box with a smaller box inside it, standing for the view on the whole document.
fn paint_minimap_icon(ui: &egui::Ui, rect: egui::Rect, stroke_color: egui::Color32) {
    let radius = theme::corner_radius_of(ui).max(3);
    let square = egui::Rect::from_center_size(rect.center(), rect.size() * 0.8);
    let stroke = egui::Stroke::new(1.0, stroke_color);
    let view = egui::Rect::from_min_size(square.min + egui::vec2(3.0, square.height() * 0.3), egui::vec2(square.width() - 6.0, square.height() * 0.3));
    ui.painter().rect_filled(view, 0.0, stroke_color.gamma_multiply(0.4));
    ui.painter().rect_stroke(view, 0.0, stroke, egui::StrokeKind::Inside);
    ui.painter().rect_stroke(square, radius.min(4), stroke, egui::StrokeKind::Inside);
}

/// Two boxes side by side, each with a view band at a different height: panes scrolled apart.
fn paint_independent_scroll_icon(ui: &egui::Ui, rect: egui::Rect, stroke_color: egui::Color32) {
    let radius = theme::corner_radius_of(ui).max(3).min(4);
    let square = egui::Rect::from_center_size(rect.center(), rect.size() * 0.8);
    let stroke = egui::Stroke::new(1.0, stroke_color);
    let width = (square.width() - 2.0) / 2.0;
    for (index, band_top) in [0.15, 0.55].iter().enumerate() {
        let left = square.left() + index as f32 * (width + 2.0);
        let column = egui::Rect::from_min_size(egui::pos2(left, square.top()), egui::vec2(width, square.height()));
        let band = egui::Rect::from_min_size(egui::pos2(left + 2.0, square.top() + square.height() * band_top), egui::vec2(width - 4.0, square.height() * 0.3));
        ui.painter().rect_filled(band, 0.0, stroke_color.gamma_multiply(0.4));
        ui.painter().rect_stroke(column, radius, stroke, egui::StrokeKind::Inside);
    }
}


/// Two words with a dotted gap between them in a box: whitespace that doesn't count.
fn paint_ignore_whitespace_icon(ui: &egui::Ui, rect: egui::Rect, stroke_color: egui::Color32) {
    let radius = theme::corner_radius_of(ui).max(3);
    let square = egui::Rect::from_center_size(rect.center(), rect.size() * 0.8);
    let stroke = egui::Stroke::new(1.0, stroke_color);
    let left = square.left() + 3.0;
    let right = square.right() - 3.0;
    let width = right - left;
    for (index, y) in [0.35, 0.65].iter().enumerate() {
        let y = square.top() + square.height() * y;
        let words = if index == 0 { (0.3, 0.6) } else { (0.45, 0.7) };
        ui.painter().hline(left..=left + width * words.0, y, stroke);
        ui.painter().hline(left + width * words.1..=right, y, stroke);
        let dot = egui::pos2(left + width * (words.0 + words.1) / 2.0, y);
        ui.painter().circle_filled(dot, 1.0, stroke_color.gamma_multiply(0.5));
    }
    ui.painter().rect_stroke(square, radius.min(4), stroke, egui::StrokeKind::Inside);
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

/// The number of the applied alignment around `line` of `document` that is innermost: the one with the shortest part there, and of equal parts the one nested in the most alignments.
fn innermost_applied_alignment(alignments: &Alignments, document: Document, line: usize, documents: usize) -> Option<u32> {
    let mut numbers: Vec<u32> = alignments.marks.iter().filter(|mark| mark.status == MarkStatus::Applied).map(|mark| mark.number).collect();
    numbers.sort_unstable();
    numbers.dedup();
    let complete: Vec<(u32, Vec<std::ops::Range<usize>>)> = numbers
        .into_iter()
        .filter_map(|number| {
            let parts = Document::ALL
                .iter()
                .take(documents)
                .map(|document| alignments.of(document.clone()).find(|mark| mark.number == number && mark.status == MarkStatus::Applied).map(|mark| mark.lines.clone()))
                .collect::<Option<Vec<_>>>()?;
            Some((number, parts))
        })
        .collect();
    let index = Document::ALL.iter().position(|candidate| *candidate == document)?;
    let holds = |parts: &Vec<std::ops::Range<usize>>| parts.get(index).is_some_and(|part| part.start <= line && line < part.end);
    let depth = |parts: &Vec<std::ops::Range<usize>>| complete.iter().filter(|(_, other)| other != parts && other.iter().zip(parts.iter()).all(|(other, part)| other.start <= part.start && part.end <= other.end)).count();
    complete
        .iter()
        .filter(|(_, parts)| holds(parts))
        .min_by_key(|(_, parts)| (parts[index].len(), std::cmp::Reverse(depth(parts))))
        .map(|(number, _)| number.clone())
}


/// `lines` with the lines of the fold their last line is the header of, so marking a folded entity's header row marks all of it.
fn through_folds(folds: &Folds, lines: std::ops::Range<usize>) -> std::ops::Range<usize> {
    let last = lines.end.saturating_sub(1);
    match folds.folded().iter().find(|fold| fold.start == last) {
        Some(fold) => lines.start..lines.end.max(fold.end.clone()),
        None => lines,
    }
}


fn lines_of(mirror: &Mirror, document: Document) -> Option<Lines<'_>> {
    match document {
        Document::D => mirror.output().map(|output| output.lines()),
        document => mirror.document(document).map(|text| Lines {
            text: &text.text,
            ranges: &text.lines,
        }),
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

/// Whether `key` with the caret at `cursor` would join, split, add or remove lines, or edit around a void, which the strip's fixed row must not allow.
fn changes_line_structure(lines: Lines<'_>, cursor: Cursor, on_void: bool, key: &EditKey) -> bool {
    let (start, end) = cursor.range();
    if start.line != end.line {
        return true;
    }
    match key {
        EditKey::Insert(text) => text.contains(['\n', '\r']),
        _ if on_void => true,
        EditKey::Newline => true,
        EditKey::Backspace | EditKey::DeleteWordLeft => !cursor.has_selection() && cursor.head.column == 0,
        EditKey::Delete | EditKey::DeleteWordRight => !cursor.has_selection() && cursor.head.column >= lines.line_chars(cursor.head.line),
    }
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

/// A's file name followed by the app name, or only the app name when no files are open.
fn window_title(mirror: Option<&Mirror>) -> String {
    let name = mirror
        .and_then(|mirror| mirror.document(Document::A))
        .and_then(|text| text.path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty());
    match name {
        Some(name) => format!("{name} - {WINDOW_TITLE}"),
        None => WINDOW_TITLE.to_string(),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// A 2-way merge of two lines: a modified region resolved to B, then an unchanged one; D is "b\nx\n".
    fn resolved_snapshot() -> kdiff4_protocol::Snapshot {
        use kdiff4_protocol::engine::{Comparison, ComparisonOptions, EngineCapabilities, HunkChange, Region, RegionStatus, Row};
        use kdiff4_protocol::{Choice, DocumentText, EolStyle, OutputDocument, RegionMerge, Revision, Snapshot};
        use std::path::PathBuf;
        let source = |document: Document, text: &str| DocumentText {
            document: document.clone(),
            path: PathBuf::from(format!("{}.txt", document.letter())),
            label: None,
            eol: Some(EolStyle::Lf),
            text: text.to_string(),
            lines: vec![0..1, 2..3],
        };
        let region = |index: usize, status: RegionStatus| Region {
            hunks: vec![index..index + 1, index..index + 1],
            status,
            rows: vec![Row {
                lines: vec![Some(index), Some(index)],
            }],
        };
        let comparison = Comparison {
            regions: vec![region(0, RegionStatus::Diff(HunkChange::Modified)), region(1, RegionStatus::Diff(HunkChange::Unchanged))],
        };
        let merge = vec![
            RegionMerge {
                choice: Choice::Hunks(vec![Document::B]),
                edited: false,
                output_lines: 1,
            },
            RegionMerge {
                choice: Choice::Hunks(vec![Document::A]),
                edited: false,
                output_lines: 1,
            },
        ];
        let output = OutputDocument {
            path: None,
            text: "b\nx\n".to_string(),
            modified: false,
            eol: EolChoice::default(),
        };
        Snapshot {
            revision: Revision(1),
            documents: vec![source(Document::A, "a\nx\n"), source(Document::B, "b\nx\n")],
            output: Some(output),
            comparison,
            engine: EngineCapabilities::default(),
            merge,
            options: ComparisonOptions::default(),
            current_region: None,
            char_diffs: Vec::new(),
            alignments: kdiff4_protocol::Alignments::default(),
        }
    }


    fn resolved_mirror() -> Mirror {
        Mirror::from_snapshot(resolved_snapshot(), false)
    }


    #[test]
    fn window_title_is_app_name_without_files() {
        assert_eq!(window_title(None), "Kdiff4");
    }


    #[test]
    fn window_title_shows_a_file_name() {
        assert_eq!(window_title(Some(&resolved_mirror())), "A.txt - Kdiff4");
    }


    /// An app showing `resolved_mirror()` over an in-process transport, with the host end of it. The user's saved settings are not used.
    fn app_with_session() -> (KdiffApp, kdiff4_protocol::HostEndpoint, egui::Context) {
        let (client, host) = kdiff4_protocol::in_process();
        let ctx = egui::Context::default();
        let mut app = KdiffApp::new(&ctx, client, Startup::default());
        app.config = GuiConfig::default();
        app.mirror = Some(resolved_mirror());
        (app, host, ctx)
    }


    #[test]
    fn opening_jumps_to_the_snapshot_region_or_stays_at_the_top() {
        for current in [Some(0), Some(1), None] {
            let (mut app, _host, ctx) = app_with_session();
            app.panes.source_scroll.set(400.0);
            app.panes.output_scroll.set(200.0);
            for scroll in app.panes.pane_scrolls.iter_mut() {
                scroll.set(300.0);
            }
            app.panes.jump = Some(0);
            app.panes.set_caret(Document::D, Cursor::at(Position { line: 0, column: 0 }), None);
            app.send(Command::Open(kdiff4_protocol::OpenFiles::default()), Pending::Open);
            let mut snapshot = resolved_snapshot();
            snapshot.current_region = current.clone();
            if current == Some(1) {
                snapshot.documents[0].text.truncate(2);
                snapshot.documents[0].lines.truncate(1);
                snapshot.comparison.regions[1].hunks[0] = 1..1;
                snapshot.comparison.regions[1].rows[0].lines[0] = None;
                snapshot.comparison.regions[1].status = kdiff4_protocol::engine::RegionStatus::Diff(kdiff4_protocol::engine::HunkChange::Added);
            }
            app.handle_message(&ctx, HostMessage::Event(Event::Snapshot(snapshot)));
            assert_eq!(app.panes.jump, current);
            assert_eq!(app.panes.source_scroll.offset, 0.0);
            assert_eq!(app.panes.output_scroll.offset, 0.0);
            assert!(app.panes.pane_scrolls.iter().all(|scroll| scroll.offset == 0.0));
            match current {
                Some(region) => {
                    assert_eq!(app.caret_region(), Some(region));
                    assert_eq!(app.panes.caret.unwrap().0, Document::A);
                    assert!(app.panes.caret_hidden);
                    assert_eq!(app.panes.void_row, (region == 1).then_some(1));
                }
                None => assert!(app.panes.caret.is_none()),
            }
        }
    }


    #[test]
    fn resync_snapshot_does_not_jump_to_the_current_region() {
        let (mut app, _host, ctx) = app_with_session();
        app.panes.source_scroll.set(400.0);
        let mut snapshot = resolved_snapshot();
        snapshot.current_region = Some(1);
        app.handle_message(&ctx, HostMessage::Event(Event::Snapshot(snapshot)));
        assert_eq!(app.panes.jump, None);
        assert_eq!(app.panes.source_scroll.offset, 400.0);
    }


    fn press(app: &mut KdiffApp, ctx: &egui::Context, key: Key, modifiers: Modifiers) {
        let events = vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers,
            },
        ];
        ctx.run_pass(egui::RawInput { events, ..Default::default() }, |ctx| app.handle_shortcuts(ctx))
            .drop_without_applying_deltas();
    }

    /// What the app sent to the engine, in order. Drops the app, which ends the transport.
    fn sent(app: KdiffApp, host: kdiff4_protocol::HostEndpoint) -> Vec<Command> {
        drop(app);
        std::iter::from_fn(|| host.recv()).map(|request| request.command).collect()
    }

    /// Gives the mirror this change, the way a delta from the host does.
    fn apply_change(app: &mut KdiffApp, change: Change) {
        let mirror = app.mirror.as_mut().unwrap();
        let revision = mirror.revision();
        mirror.apply_delta(kdiff4_protocol::Delta {
            revision: kdiff4_protocol::Revision(revision.0 + 1),
            cause: None,
            change,
        });
    }

    /// The app's output gets a path, and the line ending style `eol`.
    fn name_output(app: &mut KdiffApp, eol: EolChoice) {
        apply_change(app, Change::OutputPathChanged(Some(PathBuf::from("out.txt"))));
        apply_change(app, Change::OutputEolChanged(eol));
    }

    fn mark(document: Document, number: u32, lines: std::ops::Range<usize>) -> kdiff4_protocol::AlignmentMark {
        kdiff4_protocol::AlignmentMark {
            document,
            number,
            lines,
            status: kdiff4_protocol::MarkStatus::Pending,
            replaced_by: None,
            heuristic: false,
        }
    }

    /// Gives the mirror these marks, the way a delta from the host does.
    fn set_marks(app: &mut KdiffApp, marks: Vec<kdiff4_protocol::AlignmentMark>, changed: bool) {
        let change = Change::AlignmentsChanged(kdiff4_protocol::Alignments { marks, changed, ..Default::default() });
        let mirror = app.mirror.as_mut().unwrap();
        let revision = mirror.revision();
        mirror.apply_delta(kdiff4_protocol::Delta {
            revision: kdiff4_protocol::Revision(revision.0 + 1),
            cause: None,
            change,
        });
    }

    #[test]
    fn saving_without_a_line_ending_conflict_saves_at_once() {
        let (mut app, host, ctx) = app_with_session();
        name_output(&mut app, EolChoice::Style(EolStyle::CrLf));
        press(&mut app, &ctx, Key::S, Modifiers::COMMAND);
        assert!(app.eol_prompt.is_none());
        assert_eq!(sent(app, host), [Command::Save]);
    }

    #[test]
    fn saving_in_a_line_ending_conflict_asks_for_the_style_instead() {
        let (mut app, host, ctx) = app_with_session();
        name_output(&mut app, EolChoice::Conflict);
        press(&mut app, &ctx, Key::S, Modifiers::COMMAND);
        assert_eq!(app.eol_prompt, Some(PendingSave::Save));
        assert_eq!(sent(app, host), []);
    }

    #[test]
    fn choosing_a_style_sets_it_and_then_saves() {
        let (mut app, host, _ctx) = app_with_session();
        name_output(&mut app, EolChoice::Conflict);
        app.eol_prompt = Some(PendingSave::Save);
        app.answer_eol_prompt(EolAnswer::Use(EolStyle::CrLf));
        assert!(app.eol_prompt.is_none());
        assert_eq!(sent(app, host), [Command::SetOutputEol(EolStyle::CrLf), Command::Save]);
    }

    #[test]
    fn choosing_a_style_for_save_as_saves_to_the_picked_path() {
        let (mut app, host, _ctx) = app_with_session();
        app.eol_prompt = Some(PendingSave::SaveAs(PathBuf::from("elsewhere.txt")));
        app.answer_eol_prompt(EolAnswer::Use(EolStyle::Cr));
        assert_eq!(sent(app, host), [Command::SetOutputEol(EolStyle::Cr), Command::SaveAs(PathBuf::from("elsewhere.txt"))]);
    }

    #[test]
    fn cancelling_the_line_ending_question_saves_nothing() {
        let (mut app, host, _ctx) = app_with_session();
        app.eol_prompt = Some(PendingSave::Save);
        app.answer_eol_prompt(EolAnswer::Cancel);
        assert!(app.eol_prompt.is_none());
        assert_eq!(sent(app, host), []);
    }

    #[test]
    fn the_line_ending_question_closes_with_the_window() {
        let (mut app, host, ctx) = app_with_session();
        app.eol_prompt = Some(PendingSave::Save);
        ctx.run_pass(egui::RawInput::default(), |ctx| app.show_eol_prompt(ctx)).drop_without_applying_deltas();
        let events = vec![egui::Event::Key {
            key: Key::Escape,
            physical_key: Some(Key::Escape),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }];
        ctx.run_pass(egui::RawInput { events, ..Default::default() }, |ctx| app.show_eol_prompt(ctx)).drop_without_applying_deltas();
        assert!(app.eol_prompt.is_none());
        assert_eq!(sent(app, host), []);
    }

    #[test]
    fn ctrl_space_marks_the_lines_of_the_selection() {
        let (mut app, host, ctx) = app_with_session();
        app.panes.set_caret(
            Document::B,
            Cursor {
                anchor: Position { line: 0, column: 0 },
                head: Position { line: 1, column: 1 },
            },
            None,
        );
        press(&mut app, &ctx, Key::Space, Modifiers::COMMAND);
        assert_eq!(
            sent(app, host),
            [Command::MarkAlignment {
                document: Document::B,
                lines: 0..2
            }]
        );
    }

    #[test]
    fn marking_needs_lines_of_a_source_document() {
        let (mut app, host, ctx) = app_with_session();
        press(&mut app, &ctx, Key::Space, Modifiers::COMMAND);
        app.panes.set_caret(Document::D, Cursor::at(Position { line: 0, column: 0 }), None);
        press(&mut app, &ctx, Key::Space, Modifiers::COMMAND);
        press(&mut app, &ctx, Key::Space, Modifiers::COMMAND.plus(Modifiers::SHIFT));
        app.panes.set_caret(Document::A, Cursor::at(Position { line: 1, column: 0 }), Some(1));
        press(&mut app, &ctx, Key::Space, Modifiers::COMMAND);
        assert!(app.status.as_ref().is_some_and(|status| status.is_error));
        assert_eq!(sent(app, host), []);
    }

    #[test]
    fn ctrl_shift_space_marks_the_function_around_the_caret_line() {
        let (mut app, host, ctx) = app_with_session();
        let language = app.language.current().name().to_string();
        app.panes.set_caret(Document::A, Cursor::at(Position { line: 1, column: 1 }), None);
        press(&mut app, &ctx, Key::Space, Modifiers::COMMAND.plus(Modifiers::SHIFT));
        assert_eq!(sent(app, host), [Command::MarkFunctionAlignment { document: Document::A, line: 1, language }]);
    }

    #[test]
    fn ctrl_y_applies_the_alignments_instead_of_redoing() {
        let (mut app, host, ctx) = app_with_session();
        press(&mut app, &ctx, Key::Y, Modifiers::COMMAND);
        assert!(app.status.as_ref().is_some_and(|status| !status.is_error), "nothing to apply says so");
        set_marks(&mut app, vec![mark(Document::A, 1, 0..1)], true);
        press(&mut app, &ctx, Key::Y, Modifiers::COMMAND);
        press(&mut app, &ctx, Key::Z, Modifiers::COMMAND.plus(Modifiers::SHIFT));
        assert_eq!(sent(app, host), [Command::ApplyAlignments { confirmed: false }, Command::Redo]);
    }

    #[test]
    fn f5_reloads_the_files() {
        let (mut app, host, ctx) = app_with_session();
        press(&mut app, &ctx, Key::F5, Modifiers::NONE);
        assert_eq!(sent(app, host), [Command::Reload { confirmed: false }]);
    }

    #[test]
    fn f5_does_nothing_without_a_session() {
        let (mut app, host, ctx) = app_with_session();
        app.mirror = None;
        press(&mut app, &ctx, Key::F5, Modifiers::NONE);
        assert_eq!(sent(app, host), []);
    }

    #[test]
    fn the_status_says_the_files_were_reloaded() {
        let (mut app, _host, _ctx) = app_with_session();
        let id = app.send(Command::Reload { confirmed: false }, Pending::Reload).unwrap();
        app.handle_response(id, Ok(()));
        assert!(app.status.as_ref().is_some_and(|status| !status.is_error && status.text.starts_with("Reloaded.")));
    }

    #[test]
    fn ctrl_h_opens_heuristics_alignment_with_the_saved_options_and_asks_for_matches() {
        let (mut app, host, ctx) = app_with_session();
        app.config.heuristics.types = true;
        app.config.heuristics.signature_similarity = 90;
        press(&mut app, &ctx, Key::H, Modifiers::COMMAND);
        assert!(app.heuristics.is_some());
        let language = app.language.current().name().to_string();
        let options = HeuristicOptions {
            types: true,
            signature_similarity: 90,
            ..HeuristicOptions::default()
        };
        assert_eq!(sent(app, host), [Command::ProposeHeuristicAlignments { language, options, within: None }]);
    }

    fn applied(document: Document, number: u32, lines: std::ops::Range<usize>) -> kdiff4_protocol::AlignmentMark {
        kdiff4_protocol::AlignmentMark {
            status: kdiff4_protocol::MarkStatus::Applied,
            ..mark(document, number, lines)
        }
    }

    fn caret_at(app: &mut KdiffApp, document: Document, line: usize) {
        app.panes.set_caret(document, Cursor::at(Position { line, column: 0 }), None);
    }

    #[test]
    fn ctrl_alt_h_needs_the_caret_inside_an_applied_alignment() {
        let (mut app, _host, ctx) = app_with_session();
        let hint = "Put the caret inside an applied alignment to align inside it";
        press(&mut app, &ctx, Key::H, Modifiers::COMMAND.plus(Modifiers::ALT));
        assert!(app.heuristics.is_none());
        assert_eq!(app.status.as_ref().map(|status| (status.text.as_str(), status.is_error)), Some((hint, true)));
        app.status = None;
        set_marks(&mut app, vec![mark(Document::A, 1, 0..2), mark(Document::B, 1, 0..2)], true);
        caret_at(&mut app, Document::A, 1);
        press(&mut app, &ctx, Key::H, Modifiers::COMMAND.plus(Modifiers::ALT));
        assert!(app.heuristics.is_none(), "a pending alignment does not count");
        assert_eq!(app.status.as_ref().map(|status| status.text.as_str()), Some(hint));
        app.status = None;
        set_marks(&mut app, vec![applied(Document::A, 1, 0..1), applied(Document::B, 1, 0..1)], false);
        caret_at(&mut app, Document::A, 1);
        press(&mut app, &ctx, Key::H, Modifiers::COMMAND.plus(Modifiers::ALT));
        assert!(app.heuristics.is_none(), "the caret is outside it");
        assert_eq!(app.status.as_ref().map(|status| status.text.as_str()), Some(hint));
    }

    #[test]
    fn ctrl_alt_h_opens_the_window_inside_the_innermost_alignment_and_sends_it_with_both_commands() {
        let (mut app, host, ctx) = app_with_session();
        let marks = vec![applied(Document::A, 1, 0..10), applied(Document::B, 1, 0..10), applied(Document::A, 2, 2..5), applied(Document::B, 2, 3..6)];
        set_marks(&mut app, marks, false);
        caret_at(&mut app, Document::B, 4);
        press(&mut app, &ctx, Key::H, Modifiers::COMMAND.plus(Modifiers::ALT));
        assert_eq!(app.heuristics.as_ref().and_then(|dialog| dialog.within()), Some(2));
        app.apply_heuristics(vec![vec![2..3, 3..4]], Some(2));
        let language = app.language.current().name().to_string();
        let options = HeuristicOptions::default();
        assert_eq!(
            sent(app, host),
            [
                Command::ProposeHeuristicAlignments { language, options, within: Some(2) },
                Command::ApplyHeuristicAlignments { alignments: vec![vec![2..3, 3..4]], confirmed: false, within: Some(2) }
            ]
        );
    }

    #[test]
    fn the_innermost_alignment_is_the_smallest_part_and_of_equal_parts_the_deepest() {
        let alignments = |marks: Vec<kdiff4_protocol::AlignmentMark>| kdiff4_protocol::Alignments { marks, ..Default::default() };
        let pair = |number: u32, a: std::ops::Range<usize>, b: std::ops::Range<usize>| [applied(Document::A, number, a), applied(Document::B, number, b)];
        let nested: Vec<_> = [pair(1, 0..10, 0..10), pair(2, 2..8, 2..8), pair(3, 3..4, 3..4)].into_iter().flatten().collect();
        assert_eq!(innermost_applied_alignment(&alignments(nested.clone()), Document::A, 3, 2), Some(3));
        assert_eq!(innermost_applied_alignment(&alignments(nested.clone()), Document::A, 5, 2), Some(2));
        assert_eq!(innermost_applied_alignment(&alignments(nested.clone()), Document::A, 9, 2), Some(1));
        assert_eq!(innermost_applied_alignment(&alignments(nested.clone()), Document::A, 10, 2), None);
        assert_eq!(innermost_applied_alignment(&alignments(nested), Document::D, 3, 2), None);
        let equal_in_a: Vec<_> = [pair(1, 0..6, 0..9), pair(2, 0..6, 2..4)].into_iter().flatten().collect();
        assert_eq!(innermost_applied_alignment(&alignments(equal_in_a.clone()), Document::A, 1, 2), Some(2));
        let mut one_document = equal_in_a;
        one_document.retain(|mark| !(mark.number == 2 && mark.document == Document::B));
        assert_eq!(innermost_applied_alignment(&alignments(one_document), Document::A, 1, 2), Some(1), "an alignment a document lacks is not complete");
    }

    #[test]
    fn a_heuristic_proposal_fills_the_window_only_when_it_answers_the_window() {
        let (mut app, _host, ctx) = app_with_session();
        press(&mut app, &ctx, Key::H, Modifiers::COMMAND);
        let id = app.pending.iter().find(|(_, kind)| **kind == Pending::HeuristicPropose).map(|(id, _)| id.clone()).unwrap();
        let proposal = |cause| kdiff4_protocol::HeuristicProposal {
            cause,
            matches: vec![kdiff4_protocol::HeuristicMatch {
                kind: kdiff4_protocol::EntityKind::Function,
                names: vec![Some("f".to_string()), Some("f".to_string())],
                parts: vec![0..1, 1..2],
                stage: kdiff4_protocol::MatchStage::Signature,
                parent: None,
                moved: false,
            }],
            found: vec![1, 1],
            blocked: 0,
        };
        app.handle_message(&ctx, HostMessage::Event(Event::HeuristicProposal(proposal(Some(RequestId(id.0 + 100))))));
        assert!(app.heuristics.as_ref().unwrap().checked_parts().is_empty());
        app.handle_message(&ctx, HostMessage::Event(Event::HeuristicProposal(proposal(Some(id)))));
        assert_eq!(app.heuristics.as_ref().unwrap().checked_parts(), vec![vec![0..1, 1..2]]);
    }

    fn inset_choice() -> InsetChoice {
        let option = |slot, above, below, moved: &[Document]| kdiff4_protocol::InsetOption {
            slot,
            above,
            below,
            moved: moved.to_vec(),
        };
        InsetChoice {
            number: 2,
            current: 0,
            options: vec![option(0, None, Some(1), &[Document::A]), option(1, Some(1), None, &[Document::B])],
        }
    }

    fn deliver_marks(app: &mut KdiffApp, ctx: &egui::Context, alignments: kdiff4_protocol::Alignments) {
        let revision = app.mirror.as_ref().unwrap().revision();
        let delta = kdiff4_protocol::Delta {
            revision: kdiff4_protocol::Revision(revision.0 + 1),
            cause: None,
            change: Change::AlignmentsChanged(alignments),
        };
        app.handle_message(ctx, HostMessage::Event(Event::Delta(delta)));
    }

    #[test]
    fn a_mark_that_completes_an_inset_asks_where_to_show_it() {
        let (mut app, host, ctx) = app_with_session();
        let asking = kdiff4_protocol::Alignments {
            insets: vec![inset_choice()],
            ask: Some(2),
            ..Default::default()
        };
        deliver_marks(&mut app, &ctx, asking);
        assert_eq!(app.inset_prompt.as_ref().map(|choice| choice.number), Some(2));
        deliver_marks(&mut app, &ctx, kdiff4_protocol::Alignments::default());
        assert!(app.inset_prompt.is_some(), "only the user's answer closes the question");
        drop(sent(app, host));
    }

    #[test]
    fn marks_that_complete_nothing_ask_nothing() {
        let (mut app, host, ctx) = app_with_session();
        let quiet = kdiff4_protocol::Alignments {
            insets: vec![inset_choice()],
            ..Default::default()
        };
        deliver_marks(&mut app, &ctx, quiet);
        assert!(app.inset_prompt.is_none());
        drop(sent(app, host));
    }

    #[test]
    fn choosing_another_place_sends_it_and_keeping_the_suggestion_sends_nothing() {
        let (mut app, host, _ctx) = app_with_session();
        app.inset_prompt = Some(inset_choice());
        app.answer_inset_prompt(InsetAnswer::Place(1));
        assert!(app.inset_prompt.is_none());
        app.inset_prompt = Some(inset_choice());
        app.answer_inset_prompt(InsetAnswer::Keep);
        assert!(app.inset_prompt.is_none());
        app.answer_inset_prompt(InsetAnswer::Place(0));
        assert_eq!(sent(app, host), [Command::PlaceAlignment { number: 2, slot: 1 }]);
    }

    #[test]
    fn placing_an_alignment_from_the_menu_sends_the_command() {
        let (mut app, host, ctx) = app_with_session();
        app.handle_pane_action(&ctx, PaneAction::PlaceAlignment(3, 0));
        assert_eq!(sent(app, host), [Command::PlaceAlignment { number: 3, slot: 0 }]);
    }

    #[test]
    fn ctrl_shift_y_clears_the_marks_and_is_not_apply() {
        let (mut app, host, ctx) = app_with_session();
        press(&mut app, &ctx, Key::Y, Modifiers::COMMAND.plus(Modifiers::SHIFT));
        set_marks(&mut app, vec![mark(Document::A, 1, 0..1)], true);
        press(&mut app, &ctx, Key::Y, Modifiers::COMMAND.plus(Modifiers::SHIFT));
        assert_eq!(sent(app, host), [Command::ClearAlignmentMarks]);
    }

    #[test]
    fn navigation_asks_for_alignment_stops_as_configured() {
        let (mut app, host, ctx) = app_with_session();
        app.navigate(Navigation::NextDifference);
        let view = ViewConfig {
            alignment_stops: false,
            ..ViewConfig::default()
        };
        app.apply_view(&ctx, &view);
        app.navigate(Navigation::NextDifference);
        let stops: Vec<bool> = sent(app, host)
            .into_iter()
            .filter_map(|command| match command {
                Command::Navigate { include_alignments, .. } => Some(include_alignments),
                _ => None,
            })
            .collect();
        assert_eq!(stops, [true, false]);
    }

    #[test]
    fn the_status_says_when_alignments_wait_for_apply() {
        let (mut app, host, _ctx) = app_with_session();
        set_marks(&mut app, vec![mark(Document::A, 1, 0..1)], true);
        let id = app.send(Command::ClearAlignmentMarks, Pending::AlignmentEdit).unwrap();
        app.handle_response(id, Ok(()));
        assert_eq!(app.status.as_ref().map(|status| status.text.as_str()), Some(ALIGNMENTS_CHANGED_HINT));
        set_marks(&mut app, vec![mark(Document::A, 1, 0..1)], false);
        let id = app.send(Command::ClearAlignmentMarks, Pending::AlignmentEdit).unwrap();
        app.handle_response(id, Ok(()));
        assert!(app.status.is_none());
        let id = app
            .send(
                Command::MarkAlignment {
                    document: Document::A,
                    lines: 5..6,
                },
                Pending::AlignmentEdit,
            )
            .unwrap();
        app.handle_response(
            id,
            Err(kdiff4_protocol::ProtocolError {
                kind: ErrorKind::InvalidRequest,
                message: "no".to_string(),
            }),
        );
        assert!(app.status.as_ref().is_some_and(|status| status.is_error && status.text.contains("no")));
        drop(sent(app, host));
    }

    #[test]
    fn caret_stays_in_a_region_that_loses_its_lines() {
        use kdiff4_protocol::engine::{HunkChange, Region, RegionStatus, Row};
        use kdiff4_protocol::{Choice, Delta, MergeUpdate, RegionMerge, Revision};
        let mut mirror = resolved_mirror();
        let (cursor, void_row) = region_start_caret(&mirror, Document::D, 0).unwrap();
        assert_eq!(void_row, None);
        let region = Region {
            hunks: vec![0..1, 0..1],
            status: RegionStatus::Diff(HunkChange::Modified),
            rows: vec![Row { lines: vec![Some(0), Some(0)] }],
        };
        let update = MergeUpdate {
            at: 0,
            removed: 1,
            regions: vec![region],
            merge: vec![RegionMerge {
                choice: Choice::Unresolved,
                edited: false,
                output_lines: 0,
            }],
        };
        assert_eq!(
            mirror.apply_delta(Delta {
                revision: Revision(2),
                cause: None,
                change: Change::OutputEdited {
                    range: 0..2,
                    text: String::new(),
                    modified: true,
                    merge: update
                }
            }),
            DeltaOutcome::Applied
        );
        let rows = rows_of(&mirror, Document::D).unwrap();
        assert_eq!(
            rows.region_of_row(text_view::caret_row(rows, Caret { cursor, void_row })),
            Some(1),
            "the old caret now points into the region below"
        );
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
    fn strip_edits_never_change_the_lines() {
        let ranges = [0..2, 3..5];
        let lines = Lines { text: "ab\ncd", ranges: &ranges };
        let at = |line, column| Cursor::at(Position { line, column });
        let insert = |text: &str| EditKey::Insert(text.to_string());
        assert!(!changes_line_structure(lines, at(0, 1), false, &insert("x")));
        assert!(changes_line_structure(lines, at(0, 1), false, &insert("x\ny")));
        assert!(changes_line_structure(lines, at(0, 1), false, &EditKey::Newline));
        assert!(changes_line_structure(lines, at(1, 0), false, &EditKey::Backspace));
        assert!(!changes_line_structure(lines, at(1, 1), false, &EditKey::Backspace));
        assert!(changes_line_structure(lines, at(0, 2), false, &EditKey::Delete));
        assert!(!changes_line_structure(lines, at(0, 1), false, &EditKey::Delete));
        assert!(
            changes_line_structure(
                lines,
                Cursor {
                    anchor: Position { line: 0, column: 0 },
                    head: Position { line: 1, column: 0 }
                },
                false,
                &insert("x")
            ),
            "a selection over a line break"
        );
        assert!(!changes_line_structure(
            lines,
            Cursor {
                anchor: Position { line: 0, column: 0 },
                head: Position { line: 0, column: 2 }
            },
            false,
            &EditKey::Backspace
        ));
        assert!(!changes_line_structure(lines, at(1, 0), true, &insert("x")));
        assert!(
            changes_line_structure(lines, at(1, 0), true, &EditKey::Delete),
            "on a void the key would act on the line after it"
        );
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
        let region = |hunk: std::ops::Range<usize>, line: Option<usize>| Region {
            hunks: vec![hunk.clone(), hunk],
            status: RegionStatus::Diff(HunkChange::Unchanged),
            rows: vec![Row { lines: vec![line, line] }],
        };
        let comparison = Comparison {
            regions: vec![region(0..1, Some(0)), region(1..1, None), region(1..2, Some(1))],
        };
        rows::RowMap::for_source(&comparison, &[], 0, 2, None, None)
    }

    /// Gives the mirror the fold ranges of `document`, as the host's answer to `FindFoldRanges` does.
    fn give_fold_ranges(app: &mut KdiffApp, document: Document, ranges: Vec<std::ops::Range<usize>>) {
        let mirror = app.mirror.as_mut().unwrap();
        let revision = mirror.revision();
        mirror.set_fold_ranges(kdiff4_protocol::FoldRanges {
            cause: None,
            revision,
            documents: vec![(document, ranges)],
        });
    }


    fn folded(app: &KdiffApp, document: Document) -> Vec<std::ops::Range<usize>> {
        app.mirror.as_ref().unwrap().folds(document).folded().to_vec()
    }


    #[test]
    fn fold_ranges_are_asked_for_every_document_then_for_d_after_it_changed() {
        let (mut app, host, ctx) = app_with_session();
        let language = app.language.current().name().to_string();
        app.request_fold_ranges();
        app.request_fold_ranges();
        let found = kdiff4_protocol::FoldRanges {
            cause: app.fold_request.clone(),
            revision: app.mirror.as_ref().unwrap().revision(),
            documents: vec![(Document::A, vec![0..2])],
        };
        app.handle_message(&ctx, HostMessage::Event(Event::FoldRanges(found)));
        assert_eq!(app.mirror.as_ref().unwrap().folds(Document::A).range_at(0), Some(0..2));
        app.request_fold_ranges();
        app.output_folds_stale = true;
        app.request_fold_ranges();
        let all = Command::FindFoldRanges {
            language: language.clone(),
            documents: Document::ALL.to_vec(),
        };
        let output = Command::FindFoldRanges { language, documents: vec![Document::D] };
        assert_eq!(sent(app, host), [all, output]);
    }


    #[test]
    fn ctrl_minus_folds_around_the_caret_and_puts_it_before_the_ellipsis_and_ctrl_plus_unfolds() {
        let (mut app, _host, ctx) = app_with_session();
        give_fold_ranges(&mut app, Document::A, vec![0..2]);
        let at = |line, column| Position { line, column };
        app.panes.set_caret(Document::A, Cursor::at(at(1, 1)), None);
        press(&mut app, &ctx, Key::Minus, Modifiers::COMMAND);
        assert_eq!(folded(&app, Document::A), [0..2]);
        assert_eq!(app.panes.caret, Some((Document::A, Cursor::at(at(0, 1)))));
        press(&mut app, &ctx, Key::Plus, Modifiers::COMMAND);
        assert!(folded(&app, Document::A).is_empty());
    }


    #[test]
    fn ctrl_shift_minus_and_plus_fold_and_unfold_everything_in_the_pane_with_the_caret_only() {
        let (mut app, _host, ctx) = app_with_session();
        give_fold_ranges(&mut app, Document::A, vec![0..2]);
        give_fold_ranges(&mut app, Document::D, vec![0..2]);
        app.panes.set_caret(Document::A, Cursor::at(Position { line: 0, column: 0 }), None);
        press(&mut app, &ctx, Key::Minus, Modifiers::COMMAND.plus(Modifiers::SHIFT));
        assert_eq!((folded(&app, Document::A), folded(&app, Document::D)), (vec![0..2], Vec::new()));
        press(&mut app, &ctx, Key::Plus, Modifiers::COMMAND.plus(Modifiers::SHIFT));
        assert!(folded(&app, Document::A).is_empty());
    }


    #[test]
    fn the_fold_button_folds_and_moves_a_caret_the_fold_would_hide_onto_its_header() {
        let (mut app, _host, ctx) = app_with_session();
        give_fold_ranges(&mut app, Document::A, vec![0..2]);
        let at = |line, column| Position { line, column };
        app.panes.set_caret(Document::A, Cursor::at(at(1, 0)), None);
        app.handle_pane_action(&ctx, PaneAction::ToggleFold(Document::A, 0));
        assert_eq!(folded(&app, Document::A), [0..2]);
        assert_eq!(app.panes.caret, Some((Document::A, Cursor::at(at(0, 1)))));
        app.handle_pane_action(&ctx, PaneAction::ToggleFold(Document::A, 0));
        assert!(folded(&app, Document::A).is_empty());
    }


    #[test]
    fn the_caret_entering_a_fold_unfolds_it_and_on_the_header_keeps_it() {
        let (mut app, _host, _ctx) = app_with_session();
        give_fold_ranges(&mut app, Document::A, vec![0..2]);
        app.mirror.as_mut().unwrap().change_folds(Document::A, |folds| folds.fold(0..2));
        app.panes.set_caret(Document::A, Cursor::at(Position { line: 0, column: 1 }), None);
        app.unfold_around_caret();
        assert_eq!(folded(&app, Document::A), [0..2]);
        app.panes.set_caret(Document::A, Cursor::at(Position { line: 1, column: 0 }), None);
        app.unfold_around_caret();
        assert!(folded(&app, Document::A).is_empty());
    }


    #[test]
    fn a_jump_unfolds_the_folds_that_hide_its_region_in_every_pane() {
        let (mut app, _host, _ctx) = app_with_session();
        give_fold_ranges(&mut app, Document::A, vec![0..2]);
        give_fold_ranges(&mut app, Document::D, vec![0..2]);
        for document in [Document::A, Document::D] {
            app.mirror.as_mut().unwrap().change_folds(document, |folds| folds.fold(0..2));
        }
        app.jump_to(0);
        assert_eq!((folded(&app, Document::A), folded(&app, Document::D)), (vec![0..2], vec![0..2]));
        app.jump_to(1);
        assert_eq!((folded(&app, Document::A), folded(&app, Document::D)), (Vec::new(), Vec::new()));
    }


    #[test]
    fn marking_a_folded_header_for_alignment_marks_every_line_of_the_fold() {
        let (mut app, host, _ctx) = app_with_session();
        give_fold_ranges(&mut app, Document::A, vec![0..2]);
        app.mirror.as_mut().unwrap().change_folds(Document::A, |folds| folds.fold(0..2));
        app.panes.set_caret(Document::A, Cursor::at(Position { line: 0, column: 0 }), None);
        app.mark_selected_lines();
        assert_eq!(sent(app, host), [Command::MarkAlignment { document: Document::A, lines: 0..2 }]);
    }


    #[test]
    fn a_selection_over_a_fold_takes_the_hidden_lines_so_copying_copies_them() {
        let text = "l0\nl1\nl2\nl3\n";
        let ranges = editing::editor_lines(text);
        let lines = Lines { text, ranges: &ranges };
        let map = rows::RowMap::from_rows((0..5).map(|line| rows::PaneRow { line: Some(line), ..Default::default() }).collect()).folded(&[0..3]);
        let start = Caret {
            cursor: Cursor::at(Position { line: 0, column: 0 }),
            void_row: None,
        };
        let (cursor, _) = move_by_rows(lines, Rows::Mapped(&map), start, Motion::Down, true);
        assert_eq!(cursor.head, Position { line: 3, column: 0 });
        assert_eq!(editing::selected_text(lines, cursor), "l0\nl1\nl2\n");
    }


    #[test]
    fn vertical_moves_stop_on_void_rows() {
        let map = rows_with_a_void();
        let rows = Rows::Mapped(&map);
        let ranges = [0..2, 3..4];
        let lines = Lines { text: "ab\nc", ranges: &ranges };
        let at = |line, column| Position { line, column };
        let start = Caret {
            cursor: Cursor::at(at(0, 2)),
            void_row: None,
        };
        let (cursor, void_row) = move_by_rows(lines, rows, start, Motion::Down, false);
        assert_eq!((cursor, void_row), (Cursor::at(at(1, 0)), Some(1)));
        let (cursor, void_row) = move_by_rows(lines, rows, Caret { cursor, void_row }, Motion::Down, false);
        assert_eq!((cursor, void_row), (Cursor::at(at(1, 0)), None));
        let (cursor, void_row) = move_by_rows(lines, rows, Caret { cursor, void_row }, Motion::PageUp(5), true);
        assert_eq!((cursor, void_row), (Cursor { anchor: at(1, 0), head: at(0, 0) }, None));
        let (cursor, void_row) = move_by_rows(
            lines,
            rows,
            Caret {
                cursor: Cursor::at(at(0, 1)),
                void_row: Some(1),
            },
            Motion::Right,
            false,
        );
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

    /// What the status bar drew for a message in a window 500 wide: the selector's height and right edge, and the message's right edge.
    fn status_bar_layout(text: Option<&str>) -> (f32, f32, f32) {
        use egui_kittest::kittest::Queryable;
        let status = text.map(|text| Status {
            text: text.to_string(),
            is_error: false,
        });
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(500.0, 200.0)).build_ui(move |ui| {
            egui_extras::install_image_loaders(ui.ctx());
            theme::apply_style(ui.ctx(), &config::ViewConfig::default());
            status_bar(ui, status.as_ref(), &Language::plain_text(), None);
        });
        harness.run();
        let selector = harness.get_by_label("Language of the documents").rect();
        let message = harness.get_by_label_contains(text.unwrap_or("Ready")).rect();
        (selector.height(), selector.right(), message.right())
    }

    #[test]
    fn the_status_bar_keeps_the_selector_at_the_right_and_cuts_long_messages() {
        let (height, selector_right, short_right) = status_bar_layout(None);
        assert!(selector_right > 450.0, "the selector sits at the right edge, not at {selector_right}");
        assert!(short_right < 100.0, "a short message stays at the left");
        let long = "A very long message that does not fit beside the language selector in a window five hundred pixels wide, so it has to be cut. ".repeat(3);
        let (long_height, long_selector_right, long_right) = status_bar_layout(Some(&long));
        assert_eq!(long_height, height, "the bar keeps its height");
        assert_eq!(long_selector_right, selector_right, "a long message doesn't move the selector");
        assert!(long_right <= selector_right - 100.0, "the message stops before the selector, not at {long_right}");
    }

    /// The status bar drawn in a window 500 wide with an output whose line ending style is `eol`.
    fn eol_status_bar(eol: EolChoice) -> egui_kittest::Harness<'static> {
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(500.0, 200.0)).build_ui(move |ui| {
            egui_extras::install_image_loaders(ui.ctx());
            theme::apply_style(ui.ctx(), &config::ViewConfig::default());
            status_bar(ui, None, &Language::plain_text(), Some(&eol));
        });
        harness.run();
        harness
    }

    #[test]
    fn the_status_bar_shows_the_output_line_ending_left_of_the_language() {
        use egui_kittest::kittest::Queryable;
        let harness = eol_status_bar(EolChoice::Style(EolStyle::CrLf));
        let eol = harness.get_by_label(eol::SELECTOR_HINT).rect();
        let language = harness.get_by_label("Language of the documents").rect();
        assert!(eol.right() <= language.left(), "the line ending selector sits left of the language selector");
        assert!(language.left() - eol.right() < 20.0, "and right beside it, not {} away", language.left() - eol.right());
    }

    #[test]
    fn a_line_ending_conflict_is_named_in_the_status_bar() {
        use egui_kittest::kittest::Queryable;
        let harness = eol_status_bar(EolChoice::Conflict);
        harness.get_by_label(eol::CONFLICT_HINT);
    }

    #[test]
    fn without_an_output_the_status_bar_has_no_line_ending_selector() {
        use egui_kittest::kittest::Queryable;
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(500.0, 200.0)).build_ui(|ui| {
            egui_extras::install_image_loaders(ui.ctx());
            theme::apply_style(ui.ctx(), &config::ViewConfig::default());
            status_bar(ui, None, &Language::plain_text(), None);
        });
        harness.run();
        assert!(harness.query_by_label(eol::SELECTOR_HINT).is_none());
        assert!(harness.query_by_label(eol::CONFLICT_HINT).is_none());
    }

    #[test]
    fn missing_navigation_targets_are_named() {
        assert_eq!(navigation_noun(Navigation::NextConflict), "next conflict");
        assert_eq!(navigation_noun(Navigation::PreviousDifference), "previous difference");
    }

    fn selected(app: &KdiffApp) -> Option<(Document, Position, Position)> {
        app.panes.caret.map(|(document, cursor)| (document, cursor.range().0, cursor.range().1))
    }

    fn at(line: usize, column: usize) -> Position {
        Position { line, column }
    }

    /// An app searching for every character.
    fn app_finding_every_character() -> (KdiffApp, kdiff4_protocol::HostEndpoint, egui::Context) {
        let (mut app, host, ctx) = app_with_session();
        app.panes.find.query = ".".to_string();
        app.panes.find.options.regex = true;
        (app, host, ctx)
    }

    #[test]
    fn ctrl_f_opens_the_find_bar_with_the_keyboard() {
        let (mut app, _host, ctx) = app_with_session();
        press(&mut app, &ctx, Key::F, Modifiers::COMMAND);
        assert!(app.panes.find.open && app.panes.find.focus);
    }

    #[test]
    fn ctrl_f_searches_for_the_selected_text() {
        let (mut app, _host, ctx) = app_with_session();
        app.panes.set_caret(Document::A, Cursor { anchor: at(1, 0), head: at(1, 1) }, None);
        press(&mut app, &ctx, Key::F, Modifiers::COMMAND);
        assert_eq!(app.panes.find.query, "x");
    }

    #[test]
    fn f3_without_a_query_opens_the_find_bar() {
        let (mut app, _host, ctx) = app_with_session();
        press(&mut app, &ctx, Key::F3, Modifiers::NONE);
        assert!(app.panes.find.open && app.panes.find.focus);
        assert_eq!(selected(&app), None);
    }

    #[test]
    fn f3_selects_the_next_match_in_the_pane_with_the_caret() {
        let (mut app, _host, ctx) = app_finding_every_character();
        app.panes.set_caret(Document::A, Cursor::at(at(0, 0)), None);
        press(&mut app, &ctx, Key::F3, Modifiers::NONE);
        assert_eq!(selected(&app), Some((Document::A, at(0, 0), at(0, 1))));
        assert_eq!(app.panes.find.current, Some(0));
        press(&mut app, &ctx, Key::F3, Modifiers::NONE);
        assert_eq!(selected(&app), Some((Document::A, at(1, 0), at(1, 1))));
        press(&mut app, &ctx, Key::F3, Modifiers::NONE);
        assert_eq!(selected(&app), Some((Document::A, at(0, 0), at(0, 1))), "wraps to the first match");
        assert!(app.panes.reveal_caret);
    }

    #[test]
    fn shift_f3_selects_the_previous_match_and_wraps() {
        let (mut app, _host, ctx) = app_finding_every_character();
        app.panes.set_caret(Document::A, Cursor::at(at(1, 0)), None);
        press(&mut app, &ctx, Key::F3, Modifiers::SHIFT);
        assert_eq!(selected(&app), Some((Document::A, at(0, 0), at(0, 1))));
        press(&mut app, &ctx, Key::F3, Modifiers::SHIFT);
        assert_eq!(selected(&app), Some((Document::A, at(1, 0), at(1, 1))), "wraps to the last match");
    }

    #[test]
    fn the_search_runs_in_the_pane_that_was_clicked() {
        let (mut app, _host, ctx) = app_with_session();
        app.panes.find.query = "b".to_string();
        app.panes.set_caret(Document::A, Cursor::at(at(0, 0)), None);
        press(&mut app, &ctx, Key::F3, Modifiers::NONE);
        assert_eq!(selected(&app), Some((Document::A, at(0, 0), at(0, 0))), "A has no b");
        app.panes.set_caret(Document::B, Cursor::at(at(1, 0)), None);
        press(&mut app, &ctx, Key::F3, Modifiers::NONE);
        assert_eq!(selected(&app), Some((Document::B, at(0, 0), at(0, 1))));
        assert_eq!(app.panes.find.document, Some(Document::B));
    }

    #[test]
    fn the_output_pane_can_be_searched() {
        let (mut app, _host, ctx) = app_with_session();
        app.panes.find.query = "x".to_string();
        app.panes.set_caret(Document::D, Cursor::at(at(0, 0)), None);
        press(&mut app, &ctx, Key::F3, Modifiers::NONE);
        assert_eq!(selected(&app), Some((Document::D, at(1, 0), at(1, 1))));
    }

    #[test]
    fn typing_in_the_find_bar_selects_the_first_match_from_the_caret() {
        let (mut app, _host, ctx) = app_with_session();
        app.panes.set_caret(Document::A, Cursor::at(at(0, 1)), None);
        app.panes.find.query = "x".to_string();
        app.panes.find.open = true;
        app.handle_find_event(&ctx, FindEvent::Changed);
        assert_eq!(selected(&app), Some((Document::A, at(1, 0), at(1, 1))));
    }

    #[test]
    fn closing_the_find_bar_keeps_the_query_and_the_selection() {
        let (mut app, _host, ctx) = app_finding_every_character();
        press(&mut app, &ctx, Key::F3, Modifiers::NONE);
        app.handle_find_event(&ctx, FindEvent::Close);
        assert!(!app.panes.find.open && app.panes.find.matches.is_empty());
        assert_eq!(app.panes.find.query, ".");
        assert!(selected(&app).is_some());
    }

    #[test]
    fn f3_after_closing_the_find_bar_continues_the_search() {
        let (mut app, _host, ctx) = app_finding_every_character();
        app.panes.set_caret(Document::A, Cursor::at(at(0, 0)), None);
        press(&mut app, &ctx, Key::F3, Modifiers::NONE);
        app.handle_find_event(&ctx, FindEvent::Close);
        press(&mut app, &ctx, Key::F3, Modifiers::NONE);
        assert_eq!(selected(&app), Some((Document::A, at(1, 0), at(1, 1))));
        assert!(app.panes.find.open);
    }

    fn update_release(url: &str) -> ReleaseInfo {
        ReleaseInfo {
            version: Version::new(9, 0, 0),
            tag: "v9.0.0".to_string(),
            notes: "notes".to_string(),
            page_url: "https://example.com/release".to_string(),
            assets: vec![crate::update_check::ReleaseAsset {
                name: crate::update_check::platform_asset_name(),
                size: 10,
                url: url.to_string(),
            }],
        }
    }

    fn app_without_config_file() -> (KdiffApp, egui::Context) {
        let (mut app, _host, ctx) = app_with_session();
        app.config_path = None;
        (app, ctx)
    }

    #[test]
    fn skipping_a_version_remembers_it() {
        let (mut app, ctx) = app_without_config_file();
        app.update_window = Some(UpdateWindow::Available(update_release("https://example.com/kdiff4")));
        let Some(UpdateWindow::Available(release)) = app.update_window.take() else {
            panic!("the window should offer the release");
        };
        app.handle_update_choice(&ctx, AvailableAction::SkipVersion, release);
        assert_eq!(app.config.updates.skipped_version.as_deref(), Some("9.0.0"));
        assert!(app.update_window.is_none());
    }

    #[test]
    fn remind_me_later_remembers_nothing() {
        let (mut app, ctx) = app_without_config_file();
        app.handle_update_choice(&ctx, AvailableAction::RemindLater, update_release("https://example.com/kdiff4"));
        assert_eq!(app.config.updates.skipped_version, None);
        assert!(app.update_window.is_none());
    }

    #[test]
    fn installing_over_an_insecure_address_is_refused() {
        let (mut app, ctx) = app_without_config_file();
        app.start_install(&ctx, update_release("http://example.com/kdiff4"));
        match app.update_window {
            Some(UpdateWindow::Message { ref text, .. }) => assert!(text.contains("insecure"), "{text}"),
            _ => panic!("the refusal should be shown"),
        }
    }

    #[test]
    fn failed_install_is_reported_in_a_window() {
        let (mut app, ctx) = app_without_config_file();
        app.finish_install(&ctx, Path::new("kdiff4.exe"), Err(UpdateError::Download("connection reset".to_string())));
        match app.update_window {
            Some(UpdateWindow::Message { ref title, ref text }) => {
                assert_eq!(title, "Update Failed");
                assert!(text.contains("connection reset"));
            }
            _ => panic!("the failure should be shown"),
        }
    }

    #[test]
    fn canceled_install_closes_quietly() {
        let (mut app, ctx) = app_without_config_file();
        app.finish_install(&ctx, Path::new("kdiff4.exe"), Err(UpdateError::Canceled));
        assert!(app.update_window.is_none());
    }

    #[test]
    fn an_installed_start_menu_shortcut_is_reported_with_its_path() {
        let (mut app, _ctx) = app_without_config_file();
        app.finish_shortcut_install(Ok(PathBuf::from("C:\\Start Menu\\Programs\\Kdiff4.lnk")));
        match app.update_window {
            Some(UpdateWindow::Message { ref title, ref text }) => {
                assert_eq!(title, "Start Menu Shortcut");
                assert!(text.contains("C:\\Start Menu\\Programs\\Kdiff4.lnk"), "{text}");
            }
            _ => panic!("the result should be shown"),
        }
    }

    #[test]
    fn a_failed_start_menu_shortcut_is_reported_with_the_reason() {
        let (mut app, _ctx) = app_without_config_file();
        app.finish_shortcut_install(Err(ShortcutError::Script("access denied".to_string())));
        match app.update_window {
            Some(UpdateWindow::Message { ref text, .. }) => assert!(text.contains("Cannot add Kdiff4 to the Start menu") && text.contains("access denied"), "{text}"),
            _ => panic!("the failure should be shown"),
        }
    }

    #[test]
    fn an_open_update_window_blocks_pane_keys() {
        let (mut app, ctx) = app_without_config_file();
        app.update_window = Some(UpdateWindow::Message {
            title: "t".to_string(),
            text: "x".to_string(),
        });
        app.panes.caret = Some((Document::D, Default::default()));
        let before = app.status.clone();
        ctx.input_mut(|input| input.events.push(egui::Event::Text("typed".to_string())));
        app.handle_pane_keys(&ctx);
        assert_eq!(app.status, before);
        assert!(app.mirror.as_ref().and_then(|mirror| mirror.output()).is_none_or(|output| !output.modified));
    }


    #[test]
    fn help_opens_about_and_escape_dismisses_it() {
        use egui_kittest::kittest::Queryable;

        let (app, _ctx) = app_without_config_file();
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(700.0, 700.0)).build_ui_state(
            |ui, app: &mut KdiffApp| {
                app.show_menu(ui);
                app.show_update_window(ui.ctx());
            },
            app,
        );
        harness.run_steps(3);
        harness.get_by_label("Help").click();
        harness.run_steps(2);
        harness.get_by_label("About Kdiff4...").click();
        harness.run_steps(3);
        assert!(matches!(harness.state().update_window, Some(UpdateWindow::About)));
        harness.get_by_label(env!("KDIFF4_BUILD_COMMIT"));
        harness.get_by_label(about_dialog::REPOSITORY_URL);
        assert!(harness.query_by_label("Close").is_none());
        harness.key_press(Key::Escape);
        harness.run_steps(2);
        assert!(harness.state().update_window.is_none());
    }


    #[test]
    fn escape_closes_about_without_sending_engine_commands() {
        let (mut app, host, ctx) = app_with_session();
        app.update_window = Some(UpdateWindow::About);
        ctx.run_pass(egui::RawInput::default(), |ctx| app.show_update_window(ctx)).drop_without_applying_deltas();
        let events = vec![egui::Event::Key {
            key: Key::Escape,
            physical_key: Some(Key::Escape),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }];
        ctx.run_pass(egui::RawInput { events, ..Default::default() }, |ctx| app.show_update_window(ctx)).drop_without_applying_deltas();
        assert!(app.update_window.is_none());
        assert_eq!(sent(app, host), []);
    }


    #[test]
    fn native_about_link_opens_from_mouse_and_keyboard_and_native_close_dismisses_it() {
        use std::sync::{Arc, Mutex};

        let (mut app, ctx) = app_without_config_file();
        app.update_window = Some(UpdateWindow::About);
        ctx.set_embed_viewports(false);
        ctx.enable_accesskit();
        let pending = Arc::new(Mutex::new((Vec::<egui::Event>::new(), Vec::<egui::ViewportEvent>::new())));
        let outputs = Arc::new(Mutex::new(Vec::<egui::PlatformOutput>::new()));
        let renderer_pending = pending.clone();
        let renderer_outputs = outputs.clone();
        egui::Context::set_immediate_viewport_renderer(move |ctx, mut viewport| {
            assert_eq!(viewport.ids.this, egui::ViewportId::from_hash_of("about_dialog"));
            assert_eq!(viewport.builder.close_button, Some(true));
            let (events, viewport_events) = std::mem::take(&mut *renderer_pending.lock().unwrap());
            let mut input = egui::RawInput {
                viewport_id: viewport.ids.this,
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport.builder.inner_size.expect("About test: missing native size"))),
                events,
                ..Default::default()
            };
            let info = input.viewports.entry(viewport.ids.this).or_default();
            info.focused = Some(true);
            info.events = viewport_events;
            let output = ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui));
            renderer_outputs.lock().unwrap().push(output.platform_output.clone());
            output.drop_without_applying_deltas();
        });
        let mut frame = || {
            ctx.run_pass(egui::RawInput::default(), |ctx| {
                app.show_update_window(ctx);
                app.dialogs.end_frame(None);
            }).drop_without_applying_deltas();
            matches!(app.update_window, Some(UpdateWindow::About))
        };
        for _ in 0..4 {
            assert!(frame());
        }
        let opened_urls = || outputs.lock().unwrap()
            .iter()
            .flat_map(|output| {
                output.commands
                    .iter()
                    .filter_map(|command| match command {
                        egui::OutputCommand::OpenUrl(url) => Some(url.url.clone()),
                        _ => None,
                    })
            })
            .collect::<Vec<_>>();
        assert!(opened_urls().is_empty());
        for key in [Key::Tab, Key::Enter] {
            for pressed in [true, false] {
                pending.lock().unwrap().0.push(egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: Modifiers::NONE });
                assert!(frame());
            }
            assert!(frame());
        }
        assert_eq!(opened_urls(), [about_dialog::REPOSITORY_URL]);
        let bounds = outputs.lock().unwrap().iter().rev().find_map(|output| {
            output.accesskit_update.as_ref()?.nodes.iter().find_map(|(_, node)| {
                (node.label().as_deref() == Some(about_dialog::REPOSITORY_URL)).then(|| node.bounds()).flatten()
            })
        }).expect("About test: missing repository link in native window");
        let pos = egui::pos2((bounds.x0 + bounds.x1) as f32 / 2.0, (bounds.y0 + bounds.y1) as f32 / 2.0);
        pending.lock().unwrap().0.push(egui::Event::PointerMoved(pos));
        assert!(frame());
        assert_eq!(outputs.lock().unwrap().last().unwrap().cursor_icon, egui::CursorIcon::PointingHand);
        for pressed in [true, false] {
            pending.lock().unwrap().0.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            assert!(frame());
            if pressed {
                assert_eq!(opened_urls(), [about_dialog::REPOSITORY_URL]);
            }
        }
        assert_eq!(opened_urls(), [about_dialog::REPOSITORY_URL, about_dialog::REPOSITORY_URL]);
        pending.lock().unwrap().1.push(egui::ViewportEvent::Close);
        assert!(!frame());
        assert_eq!(opened_urls().len(), 2);
    }
}
