use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};

use eframe::egui_wgpu::wgpu::rwh::HasWindowHandle;
use egui::{Align, Button, Color32, Id, Label, Layout, Margin, Rect, RichText, TextEdit, TextStyle, TextWrapMode, Ui, UiBuilder, WidgetText, pos2, vec2};
use kdiff4_protocol::{Document, Labels, OpenFiles};

use crate::{GuiError, config, theme};
use crate::dialog_window::DialogWindows;
use crate::paths::clean_path_text;
use crate::updater::{UpdateWindow, Updater};

/// The form fills the window or dialog up to this width; path fields take whatever the labels and buttons leave.
const FORM_MAX_WIDTH: f32 = 720.0;
const MIN_FIELD_WIDTH: f32 = 80.0;
const WINDOW_MIN_WIDTH: f32 = 460.0;
const WINDOW_MARGIN: Margin = Margin::same(12);

/// Paths typed or picked in the Open dialog, in `Document::ALL` order, plus labels from the command line, which the dialog keeps but doesn't show.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenForm {
    paths: [String; 4],
    labels: Labels,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormAction {
    Browse(Document),
    Run,
    Cancel,
}

impl OpenForm {
    /// Returns `None` for an empty or blank field. Stray line breaks and quotes around a pasted path are dropped.
    pub fn path(&self, document: Document) -> Option<PathBuf> {
        clean_path_text(&self.paths[index(document)]).map(PathBuf::from)
    }

    pub fn set_path(&mut self, document: Document, path: &Path) {
        self.paths[index(document)] = path.display().to_string();
    }

    pub fn set_labels(&mut self, labels: Labels) {
        self.labels = labels;
    }

    pub fn set_files(&mut self, files: &OpenFiles) {
        *self = Self::default();
        for (document, path) in [(Document::A, Some(files.a.as_path())), (Document::B, Some(files.b.as_path())),
            (Document::C, files.c.as_deref()), (Document::D, files.output.as_deref())]
        {
            if let Some(path) = path {
                self.set_path(document, path);
            }
        }
        self.set_labels(files.labels.clone());
    }

    /// The files to open. A and B are required, which is what enables Run; C makes a 3-way merge, and D names the output now (a merge without D keeps it in memory until it is saved).
    pub fn files(&self) -> Option<OpenFiles> {
        let a = self.path(Document::A)?;
        let b = self.path(Document::B)?;
        Some(OpenFiles {
            a,
            b,
            c: self.path(Document::C),
            output: self.path(Document::D),
            labels: self.labels.clone(),
        })
    }

    pub fn confirmed_files(&self) -> Option<OpenFiles> {
        let files = self.files()?;
        if let Some(output) = output_to_confirm(&files) && !confirm_overwrite(output) {
            return None;
        }
        Some(files)
    }

    /// Draws the form. `cancelable` adds a Cancel button.
    pub fn show(&mut self, ui: &mut Ui, tab: Option<usize>, error: Option<&str>, cancelable: bool) -> Option<FormAction> {
        let mut action = None;
        ui.set_width(ui.available_width().min(FORM_MAX_WIDTH));
        let height = path_row_height(ui);
        let label_width = Document::ALL
            .into_iter()
            .map(|document| {
                WidgetText::from(label(document))
                    .into_galley(ui, Some(TextWrapMode::Extend), f32::INFINITY, TextStyle::Body)
                    .size()
                    .x
            })
            .fold(0.0, f32::max);
        for document in Document::ALL {
            ui.horizontal(|ui| {
                let label = ui.allocate_ui_with_layout(vec2(label_width, height), Layout::left_to_right(Align::Center), |ui| {
                    ui.set_min_width(label_width);
                    ui.label(label(document))
                }).inner;
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(Button::new("Browse...").min_size(vec2(0.0, height))).clicked() {
                        action = Some(FormAction::Browse(document));
                    }
                    let width = ui.available_width().max(MIN_FIELD_WIDTH);
                    ui.add(TextEdit::singleline(&mut self.paths[index(document)]).id(field_id(tab, document)).desired_width(width).min_size(vec2(0.0, height))).labelled_by(label.id);
                });
            });
        }
        ui.add(Label::new(RichText::new("A and B are required. Add C for a 3-way merge, D to merge into a file.").weak()).wrap());
        if let Some(error) = error {
            ui.add(Label::new(RichText::new(error).color(Color32::RED)).wrap());
        }
        ui.horizontal_wrapped(|ui| {
            let can_run = self.files().is_some();
            let run = ui.add_enabled(can_run, egui::Button::new("Run"));
            let enter = can_run && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if run.clicked() || enter {
                action = Some(FormAction::Run);
            }
            if cancelable && ui.button("Cancel").clicked() {
                action = Some(FormAction::Cancel);
            }
        });
        action
    }
}


/// Enough height for a single line and its frame in either control of a path row.
pub fn path_row_height(ui: &Ui) -> f32 {
    let spacing = ui.spacing();
    let padding = Margin::from(spacing.button_padding).sum().y;
    let text = ui.text_style_height(&TextStyle::Body) + spacing.extra_text_line_spacing.max(0.0);
    spacing.interact_size.y.max((text + padding).ceil())
}


pub fn run_window(
    check_for_updates: bool,
    submit: impl Fn(OpenFiles, Option<PathBuf>) -> Result<(), String> + Send + Sync + 'static,
) -> Result<Option<PathBuf>, GuiError> {
    let options = eframe::NativeOptions {
        centered: true,
        viewport: egui::ViewportBuilder::default()
            .with_title("Open")
            .with_inner_size(vec2(FORM_MAX_WIDTH, 200.0) + WINDOW_MARGIN.sum())
            .with_min_inner_size(vec2(WINDOW_MIN_WIDTH, 0.0)),
        ..Default::default()
    };
    let (restart, restarted) = mpsc::channel();
    eframe::run_native("Open", options, Box::new(move |cc| {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let loaded = config::default_config_path().and_then(|path| match config::load(&path) {
            Ok(settings) => Some((path, settings)),
            Err(error) => {
                log::error!("{error}");
                None
            }
        });
        let (config_path, settings) = loaded.map_or_else(|| (None, config::GuiConfig::default()), |(path, settings)| (Some(path), settings));
        cc.egui_ctx.set_theme(theme::preference(settings.view.theme.clone()));
        theme::apply_style(&cc.egui_ctx, &settings.view);
        let mut updater = Updater::default();
        if check_for_updates {
            updater.begin_startup_update_tasks(&cc.egui_ctx, &settings.updates, crate::update_check::release_url());
        }
        Ok(Box::new(OpenWindow {
            form: OpenForm::default(),
            last_dir: settings.recent.last_dir,
            picked_dir: None,
            submit: Arc::new(submit),
            pending: None,
            error: None,
            form_height: None,
            updater,
            updates: settings.updates,
            dialogs: DialogWindows::default(),
            config_path,
            restart: Some(restart),
        }))
    })).map_err(|error| GuiError::Eframe(error.to_string()))?;
    Ok(restarted.try_recv().ok())
}


struct OpenWindow<F> {
    form: OpenForm,
    last_dir: Option<PathBuf>,
    picked_dir: Option<PathBuf>,
    submit: Arc<F>,
    pending: Option<mpsc::Receiver<Result<(), String>>>,
    error: Option<String>,
    form_height: Option<f32>,
    updater: Updater,
    updates: config::UpdateConfig,
    dialogs: DialogWindows,
    config_path: Option<PathBuf>,
    restart: Option<mpsc::Sender<PathBuf>>,
}


impl<F> OpenWindow<F>
where
    F: Fn(OpenFiles, Option<PathBuf>) -> Result<(), String> + Send + Sync + 'static,
{
    fn show_panel(&mut self, ui: &mut Ui) {
        let update_was_open = self.updater.update_window.is_some();
        if self.pending.is_none() {
            let previous = self.updates.clone();
            self.updater.show(ui.ctx(), &mut self.dialogs, &mut self.updates, false);
            if self.updates != previous && let Some(path) = &self.config_path {
                if let Err(error) = config::load(path).and_then(|mut settings| {
                    if self.updates.last_check_unix != previous.last_check_unix {
                        settings.updates.last_check_unix = self.updates.last_check_unix;
                    }
                    if self.updates.skipped_version != previous.skipped_version {
                        settings.updates.skipped_version = self.updates.skipped_version.clone();
                    }
                    config::save(path, &settings)
                }) {
                    log::error!("{error}");
                }
            }
            if let Some(exe) = self.updater.restart.take() {
                if let Some(restart) = &self.restart {
                    let _ = restart.send(exe);
                }
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }
        if matches!(self.updater.update_window, Some(UpdateWindow::Installing { .. })) && ui.input(|input| input.viewport().close_requested()) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        let update_open = update_was_open || self.updater.update_window.is_some();
        let frame = egui::Frame::central_panel(ui.style()).inner_margin(WINDOW_MARGIN);
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            ui.add_enabled_ui(!update_open, |ui| self.show(ui));
        });
        if update_was_open && self.updater.update_window.is_none() {
            ui.ctx().request_repaint();
        }
    }


    fn show(&mut self, ui: &mut Ui) {
        if let Some(pending) = &self.pending {
            match pending.try_recv() {
                Ok(Ok(())) => {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                Ok(Err(error)) => self.error = Some(error),
                Err(mpsc::TryRecvError::Disconnected) => self.error = Some("The request to open files was interrupted.".into()),
                Err(mpsc::TryRecvError::Empty) => {
                    if ui.input(|input| input.viewport().close_requested()) {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    }
                    ui.centered_and_justified(|ui| {
                        ui.label("Sending files to Kdiff4…");
                    });
                    return;
                }
            }
            self.pending = None;
        }
        let room = ui.available_rect_before_wrap();
        let width = room.width().min(FORM_MAX_WIDTH);
        let height = self.form_height.unwrap_or(room.height());
        let top = room.top() + ((room.height() - height) / 2.0).max(0.0);
        let rect = Rect::from_min_size(pos2(room.left(), top), vec2(width, room.bottom() - top));
        let form = ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| self.form.show(ui, None, self.error.as_deref(), true));
        self.fit_height(ui.ctx(), form.response.rect.height().ceil());
        let action = form.inner;
        if !ui.is_enabled() {
            return;
        }
        let action = if ui.input(|input| input.key_pressed(egui::Key::Escape)) { Some(FormAction::Cancel) } else { action };
        match action {
            Some(FormAction::Browse(document)) => {
                if let Some(path) = pick_file(document, self.picked_dir.as_deref().or(self.last_dir.as_deref())) {
                    self.picked_dir = remembered_dir(&path);
                    self.form.set_path(document, &path);
                }
            }
            Some(FormAction::Run) => {
                if let Some(files) = self.form.confirmed_files() {
                    self.send(ui.ctx(), files);
                }
            }
            Some(FormAction::Cancel) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
            None => {}
        }
    }


    fn fit_height(&mut self, ctx: &egui::Context, height: f32) {
        if self.form_height == Some(height) {
            return;
        }
        self.form_height = Some(height);
        let viewport = ctx.viewport_rect().size();
        let fitted = height + WINDOW_MARGIN.sum().y;
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(vec2(WINDOW_MIN_WIDTH, fitted)));
        if (viewport.y - fitted).abs() > 1.0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(viewport.x, fitted)));
        }
        ctx.request_repaint();
    }


    fn send(&mut self, ctx: &egui::Context, files: OpenFiles) {
        let (complete, pending) = mpsc::channel();
        let submit = self.submit.clone();
        let dir = self.picked_dir.clone();
        let ctx = ctx.clone();
        match std::thread::Builder::new().name("kdiff4-open".into()).spawn(move || {
            let _ = complete.send(submit(files, dir));
            ctx.request_repaint();
        }) {
            Ok(_) => {
                self.pending = Some(pending);
                self.error = None;
            }
            Err(error) => self.error = Some(format!("Cannot send files to Kdiff4: {error}")),
        }
    }
}


impl<F> eframe::App for OpenWindow<F>
where
    F: Fn(OpenFiles, Option<PathBuf>) -> Result<(), String> + Send + Sync + 'static,
{
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        self.show_panel(ui);
        self.dialogs.end_frame(frame.window_handle().ok().map(|handle| handle.as_raw()));
    }
}


/// D is an output that may not exist yet, so it gets a Save dialog; A, B and C get an Open dialog.
pub fn pick_file(document: Document, start_dir: Option<&Path>) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().set_title(format!("Choose {}", label(document).trim_end_matches(':')));
    if let Some(dir) = start_dir {
        dialog = dialog.set_directory(dir);
    }
    match document {
        Document::D => dialog.save_file(),
        Document::A | Document::B | Document::C => dialog.pick_file(),
    }
}

/// The directory the next file dialog starts in after `picked` was confirmed.
pub fn remembered_dir(picked: &Path) -> Option<PathBuf> {
    picked.parent().filter(|dir| !dir.as_os_str().is_empty()).map(Path::to_path_buf)
}

fn output_to_confirm(files: &OpenFiles) -> Option<&Path> {
    files.output.as_deref().filter(|path| path.is_file())
}


/// Asks with a native message box; anything but Yes means no.
fn confirm_overwrite(output: &Path) -> bool {
    let answer = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title("Overwrite file?")
        .set_description(format!("The output file already exists:\n{}\n\nDo you want to overwrite it?", output.display()))
        .set_buttons(rfd::MessageButtons::YesNo)
        .show();
    answer == rfd::MessageDialogResult::Yes
}

const PROCEED_LABEL: &str = "Proceed";

/// Asks with a native message box before applying alignments discards the choices and edits in D; anything but Proceed means no.
pub fn confirm_apply_alignments() -> bool {
    let answer = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title("Apply alignments?")
        .set_description("Applying the alignments compares all documents against each other again.\n\nThe conflicts you resolved and the changes you made to the output will be lost, and this cannot be undone.")
        .set_buttons(rfd::MessageButtons::OkCancelCustom(PROCEED_LABEL.to_string(), "Cancel".to_string()))
        .show();
    proceeded(&answer)
}

/// Asks with a native message box before reloading discards the choices and edits in D; anything but Proceed means no.
pub fn confirm_reload() -> bool {
    let answer = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title("Reload files?")
        .set_description("Reloading reads all input files again and compares them anew.

The conflicts you resolved and the changes you made to the output will be lost, and this cannot be undone.")
        .set_buttons(rfd::MessageButtons::OkCancelCustom(PROCEED_LABEL.to_string(), "Cancel".to_string()))
        .show();
    proceeded(&answer)
}

pub fn confirm_choose_everywhere(document: Document) -> bool {
    let answer = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title(format!("Choose {} everywhere?", document.letter()))
        .set_description(format!("This replaces the merge output, including your manual edits, with source {}. You can undo this in one step.", document.letter()))
        .set_buttons(rfd::MessageButtons::OkCancelCustom(PROCEED_LABEL.to_string(), "Cancel".to_string()))
        .show();
    proceeded(&answer)
}


fn proceeded(answer: &rfd::MessageDialogResult) -> bool {
    *answer == rfd::MessageDialogResult::Ok || *answer == rfd::MessageDialogResult::Custom(PROCEED_LABEL.to_string())
}

fn index(document: Document) -> usize {
    match document {
        Document::A => 0,
        Document::B => 1,
        Document::C => 2,
        Document::D => 3,
    }
}

fn field_id(tab: Option<usize>, document: Document) -> Id {
    Id::unique(("open_form_path", tab, index(document)))
}


fn label(document: Document) -> &'static str {
    match document {
        Document::A => "A (Base):",
        Document::B => "B (Left):",
        Document::C => "C (Right):",
        Document::D => "D (Output):",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn form(paths: [&str; 4]) -> OpenForm {
        let mut form = OpenForm::default();
        for (document, path) in Document::ALL.into_iter().zip(paths) {
            form.set_path(document, Path::new(path));
        }
        form
    }

    #[test]
    fn each_open_window_submits_only_its_own_form_on_run() {
        use egui_kittest::kittest::Queryable;
        use std::time::Duration;

        let (sent, requests) = mpsc::channel();
        let submit = Arc::new(move |files, last_dir| {
            sent.send((files, last_dir)).unwrap();
            Ok(())
        });
        let make_window = || OpenWindow {
            form: OpenForm::default(),
            last_dir: None,
            picked_dir: None,
            submit: submit.clone(),
            pending: None,
            error: None,
            form_height: None,
            updater: Updater::default(),
            updates: config::UpdateConfig::default(),
            dialogs: DialogWindows::default(),
            config_path: None,
            restart: None,
        };
        let mut first = make_window();
        first.form = form(["a.txt", "b.txt", "", ""]);
        let expected = first.form.files().unwrap();
        let mut second = make_window();
        second.form = form(["c.txt", "d.txt", "", ""]);
        let mut cancel = egui_kittest::Harness::builder().build_ui_state(|ui, window| window.show_panel(ui), second);
        cancel.get_by_label("Cancel").click();
        cancel.run();
        assert!(requests.try_recv().is_err());

        let mut run = egui_kittest::Harness::builder().build_ui_state(|ui, window| window.show_panel(ui), first);
        run.get_by_label("Run").click();
        run.run_steps(4);
        assert_eq!(requests.recv_timeout(Duration::from_secs(5)).unwrap(), (expected, None));
    }


    fn layout_test_window() -> OpenWindow<impl Fn(OpenFiles, Option<PathBuf>) -> Result<(), String>> {
        OpenWindow {
            form: form(["a.txt", "b.txt", "", ""]),
            last_dir: None,
            picked_dir: None,
            submit: Arc::new(|_, _| Ok(())),
            pending: None,
            error: None,
            form_height: None,
            updater: Updater::default(),
            updates: config::UpdateConfig::default(),
            dialogs: DialogWindows::default(),
            config_path: None,
            restart: None,
        }
    }


    #[test]
    fn standalone_open_window_offers_a_newer_release_at_startup() {
        use egui_kittest::kittest::Queryable;
        use std::time::{Duration, Instant};

        let mut url = Some(crate::update_check::tests::serve_once("200 OK", r#"{"tag_name":"v99.0.0","html_url":"https://example.com/release"}"#));
        let mut harness = egui_kittest::Harness::builder().with_size(vec2(720.0, 400.0))
            .build_ui_state(move |ui, window| {
                if let Some(url) = url.take() {
                    window.updater.begin_startup_update_tasks(ui.ctx(), &window.updates, url);
                }
                window.show_panel(ui);
            }, layout_test_window());

        let deadline = Instant::now() + Duration::from_secs(10);
        while harness.state().updater.update_window.is_none() && Instant::now() < deadline {
            harness.run();
            std::thread::sleep(Duration::from_millis(5));
        }
        harness.get_by_label("Update Available");
        assert_ne!(harness.state().updates.last_check_unix, 0);
        harness.get_by_label(crate::update_dialog::REMIND_LABEL).click();
        harness.run_steps(2);
        assert!(harness.state().updater.update_window.is_none());
    }


    #[test]
    fn the_open_window_fits_the_form_height_and_grows_for_errors() {
        let mut window = layout_test_window();
        let ctx = egui::Context::default();
        let width = 520.0;
        let fitted_height = |window: &mut OpenWindow<_>, height| {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(width, height))), ..Default::default() };
            let output = ctx.run_ui(input, |ui| window.show_panel(ui));
            let size = output.viewport_output[&egui::ViewportId::ROOT].commands.iter().find_map(|command| {
                if let egui::ViewportCommand::InnerSize(size) = command { Some(size.clone()) } else { None }
            });
            output.drop_without_applying_deltas();
            size
        };
        let normal = fitted_height(&mut window, 224.0).unwrap();
        assert_eq!(normal.x, width);
        assert!(normal.y < 180.0, "{normal:?}");
        assert_eq!(normal.y, window.form_height.unwrap() + WINDOW_MARGIN.sum().y);
        assert_eq!(fitted_height(&mut window, normal.y), None);

        window.error = Some("Could not send the files to Kdiff4. ".repeat(8));
        let with_error = fitted_height(&mut window, normal.y).unwrap();
        assert!(with_error.y > normal.y);
        assert_eq!(fitted_height(&mut window, with_error.y), None);
        window.error = None;
        assert_eq!(fitted_height(&mut window, with_error.y), Some(normal));
    }


    #[test]
    fn the_open_window_centers_its_form_vertically_without_a_heading() {
        for width in [WINDOW_MIN_WIDTH, 900.0] {
            let height = 250.0;
            let mut window = layout_test_window();
            let ctx = egui::Context::default();
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(width, height))), ..Default::default() };
            ctx.run_ui(input.clone(), |ui| window.show_panel(ui)).drop_without_applying_deltas();
            ctx.enable_accesskit();
            let output = ctx.run_ui(input, |ui| window.show_panel(ui));
            let nodes = &output.platform_output.accesskit_update.as_ref().unwrap().nodes;
            let bounds = |label: &str| nodes.iter()
                .filter(|(_, node)| node.label().as_deref() == Some(label) || node.value().as_deref() == Some(label))
                .filter_map(|(_, node)| node.bounds())
                .min_by(|a, b| a.y0.total_cmp(&b.y0))
                .unwrap();
            let run = bounds("Run");
            let cancel = bounds("Cancel");
            let browse = bounds("Browse...");
            assert!(!nodes.iter().any(|(_, node)| node.label().as_deref() == Some("Open") || node.value().as_deref() == Some("Open")));
            let form_width = (width - WINDOW_MARGIN.sum().x).min(FORM_MAX_WIDTH);
            assert_eq!(run.x0, f64::from(WINDOW_MARGIN.left));
            assert!(cancel.x0 > run.x1);
            assert!(run.x1 - run.x0 < cancel.x1 - cancel.x0);
            assert!((browse.x1 - f64::from(form_width + f32::from(WINDOW_MARGIN.left))).abs() <= 2.0, "{browse:?}");
            assert!((browse.y0 + cancel.y1 - f64::from(height)).abs() <= 4.0, "{browse:?} {cancel:?}");
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn empty_form_cannot_run() {
        assert_eq!(OpenForm::default().files(), None);
    }

    #[test]
    fn a_and_b_open_a_diff() {
        let files = form(["a.txt", "b.txt", "", ""]).files().unwrap();
        assert_eq!(
            files,
            OpenFiles {
                a: PathBuf::from("a.txt"),
                b: PathBuf::from("b.txt"),
                c: None,
                output: None,
                labels: Labels::default()
            }
        );
        assert!(!files.is_merge());
    }

    #[test]
    fn merges_without_c_or_without_d_can_run() {
        let two_way = form(["a.txt", "b.txt", "", "d.txt"]).files().unwrap();
        assert_eq!(two_way.c, None);
        assert_eq!(two_way.output, Some(PathBuf::from("d.txt")));
        assert!(two_way.is_merge());
        let in_memory = form(["a.txt", "b.txt", "c.txt", ""]).files().unwrap();
        assert_eq!(in_memory.output, None);
        assert!(in_memory.is_merge());
    }

    #[test]
    fn labels_are_carried_into_files() {
        let mut form = form(["a.txt", "b.txt", "", ""]);
        let labels = Labels {
            a: Some("Base".to_string()),
            b: None,
            c: None,
        };
        form.set_labels(labels.clone());
        assert_eq!(form.files().unwrap().labels, labels);
    }

    #[test]
    fn all_four_open_a_merge() {
        let files = form(["a.txt", "b.txt", "c.txt", "d.txt"]).files().unwrap();
        assert_eq!(files.c, Some(PathBuf::from("c.txt")));
        assert_eq!(files.output, Some(PathBuf::from("d.txt")));
    }

    #[test]
    fn missing_a_or_b_cannot_run() {
        for paths in [
            ["a.txt", "", "", ""],
            ["", "b.txt", "", ""],
            ["", "b.txt", "c.txt", "d.txt"],
            ["a.txt", "", "c.txt", "d.txt"],
            ["", "", "c.txt", "d.txt"],
        ] {
            assert_eq!(form(paths).files(), None, "{paths:?}");
        }
    }

    #[test]
    fn blank_fields_count_as_empty_and_paths_are_trimmed() {
        let files = form(["  a.txt ", "b.txt\r\n", "   ", "\t"]).files().unwrap();
        assert_eq!(files.a, PathBuf::from("a.txt"));
        assert_eq!(files.b, PathBuf::from("b.txt"));
        assert_eq!(files.c, None);
    }

    #[test]
    fn path_fields_are_aligned() {
        let context = egui::Context::default();
        let mut form = OpenForm::default();
        for _ in 0..2 {
            let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                form.show(ui, Some(0), None, true);
            });
            output.textures_delta.clear();
        }
        let rects: Vec<_> = Document::ALL
            .into_iter()
            .map(|document| context.read_response(field_id(Some(0), document)).unwrap().rect)
            .collect();
        for rect in &rects[1..] {
            assert_eq!(rect.left(), rects[0].left(), "{rects:?}");
            assert_eq!(rect.right(), rects[0].right(), "{rects:?}");
        }
    }


    #[test]
    fn browse_buttons_match_the_open_form_path_fields() {
        use egui_kittest::kittest::Queryable;

        let harness = egui_kittest::Harness::builder().build_ui_state(|ui, form| {
            form.show(ui, Some(0), None, false);
        }, OpenForm::default());
        let mut fields: Vec<_> = harness.query_all_by_role(egui::Role::TextInput).map(|node| node.rect()).collect();
        let mut buttons: Vec<_> = harness.query_all_by_label("Browse...").map(|node| node.rect()).collect();
        fields.sort_by(|a, b| a.top().total_cmp(&b.top()));
        buttons.sort_by(|a, b| a.top().total_cmp(&b.top()));
        assert_eq!(fields.len(), 4);
        assert_eq!(buttons.len(), 4);
        for (field, button) in fields.into_iter().zip(buttons) {
            assert_eq!(field.y_range(), button.y_range());
        }
    }


    #[test]
    fn remembered_dir_is_parent_of_picked_file() {
        let picked = Path::new("C:/work/project/file.txt");
        assert_eq!(remembered_dir(picked), Some(PathBuf::from("C:/work/project")));
        assert_eq!(remembered_dir(Path::new("file.txt")), None);
    }

    #[test]
    fn only_proceed_confirms_applying_alignments() {
        assert!(proceeded(&rfd::MessageDialogResult::Custom("Proceed".to_string())));
        assert!(proceeded(&rfd::MessageDialogResult::Ok));
        assert!(!proceeded(&rfd::MessageDialogResult::Custom("Cancel".to_string())));
        assert!(!proceeded(&rfd::MessageDialogResult::Cancel));
        assert!(!proceeded(&rfd::MessageDialogResult::No));
    }

    #[test]
    fn overwrite_confirmation_only_for_existing_output_file() {
        let dir = std::env::temp_dir().join(format!("kdiff4-gui-overwrite-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let existing = dir.join("existing.txt");
        fs::write(&existing, "x").unwrap();
        let files = |output: Option<PathBuf>| OpenFiles {
            a: PathBuf::from("a"),
            b: PathBuf::from("b"),
            c: Some(PathBuf::from("c")),
            output,
            labels: Labels::default(),
        };
        assert!(output_to_confirm(&files(Some(existing))).is_some());
        assert!(output_to_confirm(&files(Some(dir.join("new.txt")))).is_none());
        assert!(output_to_confirm(&files(Some(dir.clone()))).is_none());
        assert!(output_to_confirm(&files(None)).is_none());
    }
}
