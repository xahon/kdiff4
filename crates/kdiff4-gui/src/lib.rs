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
mod window_frame;

use std::path::PathBuf;

use kdiff4_protocol::{Client, Document, Labels, RequestId};
use thiserror::Error;

pub use paths::{clean_path, clean_path_text};

#[derive(Debug, Error)]
pub enum GuiError {
    #[error("cannot start the window: {0}")]
    Eframe(String),
}

/// What the binary already knows when the window opens.
#[derive(Debug, Clone, Default)]
pub struct Startup {
    /// Paths to pre-fill the Open window with.
    pub paths: Vec<(Document, PathBuf)>,
    pub labels: Labels,
    /// An open request the binary already sent; the window waits for its result instead of asking for files.
    pub pending_open: Option<RequestId>,
}

/// Runs the window on the calling thread until the user closes it.
pub fn run(client: Client, startup: Startup) -> Result<(), GuiError> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title(app::WINDOW_TITLE).with_inner_size(app::INITIAL_WINDOW_SIZE).with_icon(icons::application()),
        ..Default::default()
    };
    eframe::run_native(app::WINDOW_TITLE, options, Box::new(|cc| {
            let mut app = app::KdiffApp::new(&cc.egui_ctx, client, startup);
            app.begin_startup_update_tasks(&cc.egui_ctx);
            Ok(Box::new(app))
        })).map_err(|error| GuiError::Eframe(error.to_string()))
}
