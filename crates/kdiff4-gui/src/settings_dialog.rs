use egui::{ComboBox, DragValue, RichText, Ui};

use crate::config::{MAX_CORNER_RADIUS, MAX_FONT_SIZE, MIN_FONT_SIZE, ViewConfig};
use crate::fonts::MonospaceFamily;
use crate::theme;


const BUILTIN_FONT_LABEL: &str = "Built-in (Hack)";
const PREVIEW_LINES: [&str; 3] = ["fn main() { println!(\"Hello, world!\"); }", "0O 1lI| {}[]() <=> != -> 'x' \"y\"", "The quick brown fox jumps over the lazy dog."];


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsAction {
    Ok,
    Cancel,
}


/// Edits a draft of the view settings. The app previews the draft live and keeps or reverts it on close.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsDialog {
    original: ViewConfig,
    draft: ViewConfig,
}


impl SettingsDialog {
    pub fn new(view: &ViewConfig) -> SettingsDialog {
        SettingsDialog { original: view.clone(), draft: view.clone() }
    }


    pub fn original(&self) -> &ViewConfig {
        &self.original
    }


    pub fn draft(&self) -> &ViewConfig {
        &self.draft
    }


    pub fn reset_to_defaults(&mut self) {
        self.draft = ViewConfig::default();
    }


    /// `families` is `None` while the font scan is still running.
    pub fn show(&mut self, ui: &mut Ui, families: Option<&[MonospaceFamily]>) -> Option<SettingsAction> {
        let mut action = None;
        ui.heading("Settings");
        ui.add_space(4.0);
        egui::Grid::new("settings_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Font:");
            ui.horizontal(|ui| {
                self.font_family_combo(ui, families);
                if families.is_none() {
                    ui.spinner();
                    ui.label(RichText::new("Looking for monospace fonts...").weak());
                }
            });
            ui.end_row();

            ui.label("Size:");
            ui.add(DragValue::new(&mut self.draft.font_size).range(MIN_FONT_SIZE..=MAX_FONT_SIZE).speed(0.25).max_decimals(1).suffix(" pt"));
            ui.end_row();

            ui.label("Corner radius:");
            ui.add(DragValue::new(&mut self.draft.corner_radius).range(0..=MAX_CORNER_RADIUS).speed(0.1).suffix(" px"));
            ui.end_row();

            ui.label("Rendering:");
            ui.vertical(|ui| {
                ui.checkbox(&mut self.draft.font_hinting, "Hinting").on_hover_text("Snap glyphs to the pixel grid: sharper at small sizes, slightly distorted shapes.");
                ui.checkbox(&mut self.draft.subpixel_positioning, "Subpixel positioning").on_hover_text("Place glyphs at fractional pixel offsets: more even spacing, slightly blurrier.");
            });
            ui.end_row();
        });
        ui.add_space(8.0);
        show_preview(ui);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("OK").clicked() {
                action = Some(SettingsAction::Ok);
            }
            if ui.button("Cancel").clicked() {
                action = Some(SettingsAction::Cancel);
            }
            if ui.button("Defaults").clicked() {
                self.reset_to_defaults();
            }
        });
        action
    }


    fn font_family_combo(&mut self, ui: &mut Ui, families: Option<&[MonospaceFamily]>) {
        let selected = self.draft.font_family.clone().unwrap_or_else(|| BUILTIN_FONT_LABEL.to_string());
        ComboBox::from_id_salt("font_family").selected_text(selected).width(260.0).height(400.0).show_ui(ui, |ui| {
            ui.selectable_value(&mut self.draft.font_family, None, BUILTIN_FONT_LABEL);
            let names = family_choices(families.unwrap_or_default(), self.draft.font_family.as_deref());
            for name in names {
                ui.selectable_value(&mut self.draft.font_family, Some(name.clone()), name);
            }
        });
    }
}


/// The scanned family names, plus `current` if the scan didn't find it (not installed, or the scan is still running), so the combo box can show it.
pub fn family_choices(families: &[MonospaceFamily], current: Option<&str>) -> Vec<String> {
    let mut names: Vec<String> = families
        .iter()
        .map(|family| family.name.clone())
        .collect();
    if let Some(current) = current
        && !names.iter().any(|name| name == current)
    {
        names.insert(0, current.to_string());
    }
    names
}


fn show_preview(ui: &mut Ui) {
    let palette = theme::palette_of(ui);
    theme::pane_frame(palette, theme::corner_radius_of(ui)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        theme::pane_body_frame().show(ui, |ui| {
            for line in PREVIEW_LINES {
                ui.label(RichText::new(line).monospace().color(palette.text));
            }
        });
    });
}


#[cfg(test)]
mod tests {
    use egui::FontDefinitions;

    use super::*;


    fn family(name: &str) -> MonospaceFamily {
        let data = (*FontDefinitions::default().font_data["Hack"]).clone();
        MonospaceFamily { name: name.to_string(), data }
    }


    #[test]
    fn new_dialog_starts_from_current_view() {
        let view = ViewConfig { font_family: Some("Consolas".to_string()), font_size: 15.0, font_hinting: false, subpixel_positioning: true, corner_radius: 4 };
        let dialog = SettingsDialog::new(&view);
        assert_eq!(dialog.original(), &view);
        assert_eq!(dialog.draft(), &view);
    }


    #[test]
    fn reset_changes_only_the_draft() {
        let view = ViewConfig { font_family: Some("Consolas".to_string()), font_size: 20.0, font_hinting: false, subpixel_positioning: false, corner_radius: 9 };
        let mut dialog = SettingsDialog::new(&view);
        dialog.reset_to_defaults();
        assert_eq!(dialog.draft(), &ViewConfig::default());
        assert_eq!(dialog.original(), &view);
    }


    #[test]
    fn family_choices_list_scanned_families() {
        let families = [family("Cascadia Mono"), family("Consolas")];
        assert_eq!(family_choices(&families, None), ["Cascadia Mono", "Consolas"]);
        assert_eq!(family_choices(&families, Some("Consolas")), ["Cascadia Mono", "Consolas"]);
    }


    #[test]
    fn family_choices_keep_a_current_family_the_scan_did_not_find() {
        let families = [family("Consolas")];
        assert_eq!(family_choices(&families, Some("Gone Mono")), ["Gone Mono", "Consolas"]);
        assert_eq!(family_choices(&[], Some("Gone Mono")), ["Gone Mono"]);
    }
}
