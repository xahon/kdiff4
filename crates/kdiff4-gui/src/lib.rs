mod about_dialog;
mod alignment_gutter;
mod app;
mod autoscroll;
mod char_diff;
mod config;
mod dialog_window;
mod editing;
mod eol;
mod find;
mod folding;
mod fonts;
mod heuristics_dialog;
mod icons;
mod inset_dialog;
mod language;
mod minimap;
mod mirror;
mod open_dialog;
mod overview;
mod panes;
mod paths;
mod rows;
mod scroll;
mod self_update;
mod settings_dialog;
mod start_menu;
mod text_view;
mod theme;
mod update_check;
mod update_dialog;
mod updater;
mod window_frame;
mod window_placement;

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use kdiff4_protocol::{Client, Document, Labels, OpenFiles};
use thiserror::Error;

pub use paths::{clean_path, clean_path_text};

#[derive(Debug, Error)]
pub enum GuiError {
    #[error("cannot start the window: {0}")]
    Eframe(String),
}

/// Files to open in a tab, or paths to prefill when the invocation supplied fewer than two inputs.
#[derive(Debug, PartialEq, Eq)]
pub enum OpenDocument {
    Open(OpenFiles),
    Prefill(Vec<(Document, PathBuf)>, Labels),
}


pub struct OpenRequest {
    pub document: OpenDocument,
    pub completion: OpenCompletion,
}


pub enum OpenCompletion {
    TabClosed(Sender<bool>),
    Dispatched { result: Sender<bool>, last_dir: Option<PathBuf> },
}


/// Runs the window on the calling thread until the user closes it.
pub fn run(
    client: Client,
    new_client: impl Fn() -> std::io::Result<Client> + 'static,
    incoming: Receiver<OpenRequest>,
    set_waker: impl FnOnce(Box<dyn Fn() + Send + Sync>),
    open_window: impl Fn() -> std::io::Result<()> + 'static,
) -> Result<Option<PathBuf>, GuiError> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title(app::WINDOW_TITLE).with_inner_size(app::INITIAL_WINDOW_SIZE).with_icon(icons::application()),
        ..Default::default()
    };
    let (restart, restarted) = std::sync::mpsc::channel();
    eframe::run_native(app::WINDOW_TITLE, options, Box::new(move |cc| {
            let ctx = cc.egui_ctx.clone();
            set_waker(Box::new(move || ctx.request_repaint()));
            let mut app = app::KdiffApp::new(&cc.egui_ctx, client, new_client);
            app.restore_window(cc);
            app.connect_launches(incoming, restart);
            app.set_open_window(open_window);
            app.receive_startup_launches(&cc.egui_ctx);
            app.begin_startup_update_tasks(&cc.egui_ctx);
            Ok(Box::new(app))
        })).map_err(|error| GuiError::Eframe(error.to_string()))?;
    Ok(restarted.try_recv().ok())
}


/// Runs an independent Open window and submits the chosen files to the comparison window.
pub fn run_open_window(
    check_for_updates: bool,
    submit: impl Fn(OpenFiles, Option<PathBuf>) -> Result<(), String> + Send + Sync + 'static,
) -> Result<Option<PathBuf>, GuiError> {
    open_dialog::run_window(check_for_updates, submit)
}
