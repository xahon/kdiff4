use std::path::{Path, PathBuf};

use egui::{Align, Color32, Label, Layout, RichText, TextEdit, TextStyle, TextWrapMode, Ui, WidgetText, vec2};
use kdiff4_protocol::{Document, Labels, OpenFiles};

use crate::paths::clean_path_text;

/// The form fills the window or dialog up to this width; path fields take whatever the labels and buttons leave.
const FORM_MAX_WIDTH: f32 = 720.0;
const MIN_FIELD_WIDTH: f32 = 80.0;

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

    /// Draws the dialog contents. `busy` disables Run while an open is in flight; `cancelable` adds a Cancel button.
    pub fn show(&mut self, ui: &mut Ui, error: Option<&str>, busy: bool, cancelable: bool) -> Option<FormAction> {
        let mut action = None;
        ui.set_width(ui.available_width().min(FORM_MAX_WIDTH));
        let height = ui.spacing().interact_size.y;
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
                ui.allocate_ui_with_layout(vec2(label_width, height), Layout::left_to_right(Align::Center), |ui| ui.label(label(document)));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Browse...").clicked() {
                        action = Some(FormAction::Browse(document));
                    }
                    let width = ui.available_width().max(MIN_FIELD_WIDTH);
                    ui.add(TextEdit::singleline(&mut self.paths[index(document)]).desired_width(width));
                });
            });
        }
        ui.add(Label::new(RichText::new("A and B are required. Add C for a 3-way merge, D to merge into a file.").weak()).wrap());
        if let Some(error) = error {
            ui.add(Label::new(RichText::new(error).color(Color32::RED)).wrap());
        }
        ui.horizontal_wrapped(|ui| {
            let can_run = !busy && self.files().is_some();
            let run = ui.add_enabled(can_run, egui::Button::new("Run"));
            let enter = can_run && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if run.clicked() || enter {
                action = Some(FormAction::Run);
            }
            if cancelable && ui.button("Cancel").clicked() {
                action = Some(FormAction::Cancel);
            }
            if busy {
                ui.spinner();
            }
        });
        action
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

pub fn needs_overwrite_confirmation(files: &OpenFiles) -> bool {
    files.output.as_deref().is_some_and(Path::is_file)
}

/// Asks with a native message box; anything but Yes means no.
pub fn confirm_overwrite(output: &Path) -> bool {
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
        assert!(needs_overwrite_confirmation(&files(Some(existing))));
        assert!(!needs_overwrite_confirmation(&files(Some(dir.join("new.txt")))));
        assert!(!needs_overwrite_confirmation(&files(Some(dir.clone()))));
        assert!(!needs_overwrite_confirmation(&files(None)));
    }
}
