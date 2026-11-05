mod app;
mod char_diff;
mod config;
mod editing;
mod fonts;
mod language;
mod mirror;
mod open_dialog;
mod overview;
mod panes;
mod paths;
mod rows;
mod scroll;
mod settings_dialog;
mod text_view;
mod theme;
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
        viewport: egui::ViewportBuilder::default().with_title("Kdiff4").with_inner_size(app::INITIAL_WINDOW_SIZE),
        ..Default::default()
    };
    eframe::run_native("Kdiff4", options, Box::new(|cc| Ok(Box::new(app::KdiffApp::new(&cc.egui_ctx, client, startup)))))
        .map_err(|error| GuiError::Eframe(error.to_string()))
}
