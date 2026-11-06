use egui::{Button, Checkbox, Color32, ComboBox, DragValue, Label, RichText, ScrollArea, Theme, Ui};
use kdiff4_protocol::Document;
use kdiff4_protocol::engine::{ComparisonOptions, DiffAlgorithm};

use crate::config::{
    MAX_CORNER_RADIUS, MAX_FONT_SIZE, MAX_JUMP_CONTEXT_LINES, MAX_OTHER_REGION_BORDER_PERCENT, MAX_REGION_CORNER_RADIUS, MAX_SCROLL_STEP_LINES, MAX_SMOOTH_SCROLL_MS,
    MIN_FONT_SIZE, MIN_SCROLL_STEP_LINES, MIN_SMOOTH_SCROLL_MS, ThemeChoice, ViewConfig,
};
use crate::fonts::MonospaceFamily;
use crate::theme;

const BUILTIN_FONT_LABEL: &str = "Built-in (Hack)";
const MAX_CHAR_DIFF_LINE_LIMIT: usize = 1_000_000;
/// The dialog keeps this size whatever the tab; a tab taller than the body scrolls.
const DIALOG_WIDTH: f32 = 720.0;
const BODY_HEIGHT: f32 = 440.0;
const PREVIEW_LINES: [&str; 3] = [
    "fn main() { println!(\"Hello, world!\"); }",
    "0O 1lI| {}[]() <=> != -> 'x' \"y\"",
    "The quick brown fox jumps over the lazy dog.",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsAction {
    Ok,
    Cancel,
    /// Opens `gui.toml` in the system's default editor; the dialog stays open.
    OpenConfigFile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Panes,
    Appearance,
    Comparison,
}

/// Edits a draft of the view settings and, while a session is open, of its comparison options. The app previews the view draft live and keeps or reverts it on close; comparison options are applied on OK only, since they rebuild D.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsDialog {
    original: ViewConfig,
    draft: ViewConfig,
    /// The session's options and their draft; `None` without a session.
    comparison: Option<(ComparisonOptions, ComparisonOptions)>,
    tab: Tab,
}

impl SettingsDialog {
    pub fn new(view: &ViewConfig, comparison: Option<&ComparisonOptions>) -> SettingsDialog {
        SettingsDialog {
            original: view.clone(),
            draft: view.clone(),
            comparison: comparison.map(|options| (options.clone(), options.clone())),
            tab: Tab::Panes,
        }
    }

    /// The comparison options to apply on OK, if the user changed them.
    pub fn changed_comparison(&self) -> Option<&ComparisonOptions> {
        self.comparison.as_ref().filter(|(original, draft)| original != draft).map(|(_, draft)| draft)
    }

    /// Follows the toolbar's void space toggle, which also works while the dialog is open.
    pub fn set_align_output(&mut self, align: bool) {
        self.original.align_output = align;
        self.draft.align_output = align;
    }

    /// Follows the toolbar's overview strip toggle, which also works while the dialog is open.
    pub fn set_show_overview_strip(&mut self, show: bool) {
        self.original.show_overview_strip = show;
        self.draft.show_overview_strip = show;
    }

    /// Follows the toolbar's and the shortcut's minimap toggle, which also works while the dialog is open.
    pub fn set_show_minimap(&mut self, show: bool) {
        self.original.show_minimap = show;
        self.draft.show_minimap = show;
    }

    pub fn original(&self) -> &ViewConfig {
        &self.original
    }

    pub fn draft(&self) -> &ViewConfig {
        &self.draft
    }

    pub fn reset_to_defaults(&mut self) {
        self.draft = ViewConfig::default();
        if let Some((_, draft)) = self.comparison.as_mut() {
            *draft = ComparisonOptions::default();
        }
    }

    /// `families` is `None` while the font scan is still running.
    pub fn show(&mut self, ui: &mut Ui, families: Option<&[MonospaceFamily]>) -> Option<SettingsAction> {
        let mut action = None;
        ui.set_width(DIALOG_WIDTH);
        ui.heading("Settings");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Panes, "Panes");
            ui.selectable_value(&mut self.tab, Tab::Appearance, "Appearance");
            ui.selectable_value(&mut self.tab, Tab::Comparison, "Comparison");
        });
        ui.separator();
        ScrollArea::vertical()
            .id_salt("settings_body")
            .auto_shrink(false)
            .min_scrolled_height(BODY_HEIGHT)
            .max_height(BODY_HEIGHT)
            .show(ui, |ui| match self.tab {
                Tab::Panes => self.panes_tab(ui, families),
                Tab::Appearance => self.appearance_tab(ui),
                Tab::Comparison => self.comparison_tab(ui),
            });
        ui.separator();
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
            if ui.button("Open in editor").on_hover_text("Open gui.toml in the default editor").clicked() {
                action = Some(SettingsAction::OpenConfigFile);
            }
        });
        action
    }

    /// How the panes draw text and regions.
    fn panes_tab(&mut self, ui: &mut Ui, families: Option<&[MonospaceFamily]>) {
        let theme = active_theme(ui);
        section(ui, "panes_text", "Panes: Text", |ui| {
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
            ui.add(
                DragValue::new(&mut self.draft.font_size)
                    .range(MIN_FONT_SIZE..=MAX_FONT_SIZE)
                    .speed(0.25)
                    .max_decimals(1)
                    .suffix(" pt"),
            );
            ui.end_row();

            ui.label("Rendering:");
            ui.vertical(|ui| {
                ui.checkbox(&mut self.draft.font_hinting, "Hinting")
                    .on_hover_text("Snap glyphs to the pixel grid: sharper at small sizes, slightly distorted shapes.");
                ui.checkbox(&mut self.draft.subpixel_positioning, "Subpixel positioning")
                    .on_hover_text("Place glyphs at fractional pixel offsets: more even spacing, slightly blurrier.");
            });
            ui.end_row();

            ui.label("Text color:");
            color_override(
                ui,
                &mut self.draft.colors_mut(theme == Theme::Dark).pane_text,
                theme::palette(theme).pane_text,
                &format!("Text in the panes, in the {} theme.", theme_name(theme)),
            );
            ui.end_row();

            ui.label("");
            ui.vertical(|ui| {
                ui.add_space(4.0);
                show_preview(ui);
                ui.add_space(4.0);
            });
            ui.end_row();
        });

        section(ui, "panes_content", "Panes: Content", |ui| {
            ui.label("Show:");
            ui.vertical(|ui| {
                ui.checkbox(&mut self.draft.show_line_numbers, "Line numbers");
                ui.checkbox(&mut self.draft.show_gutters, "Region gutters in A, B and C")
                    .on_hover_text("A bordered block left of each region. D always shows where each line came from.");
                ui.checkbox(&mut self.draft.align_output, "Void space in the output pane")
                    .on_hover_text("Line D up with A, B and C: every region gets the same rows in every pane, padded with void space.");
                ui.checkbox(&mut self.draft.show_overview_strip, "Overview strip")
                    .on_hover_text("The current line of every document, with its character diff, between A, B and C and the output pane. D can be edited there.");
                ui.checkbox(&mut self.draft.show_minimap, "Minimap")
                    .on_hover_text("The whole document as a narrow column right of the panes, with a block for every difference. Click or drag in it to scroll.");
            });
            ui.end_row();

            ui.label("Whitespace:");
            ui.vertical(|ui| {
                let whitespace = &mut self.draft.whitespace;
                ui.checkbox(&mut whitespace.render, "Render whitespace")
                    .on_hover_text("Mark whitespace characters in the panes.");
                let enabled = whitespace.render;
                ui.indent("whitespace_kinds", |ui| {
                    ui.add_enabled(enabled, Checkbox::new(&mut whitespace.spaces, "Spaces"));
                    ui.add_enabled(enabled, Checkbox::new(&mut whitespace.tabs, "Tabs"));
                    ui.add_enabled(enabled, Checkbox::new(&mut whitespace.line_feeds, "Line feeds"))
                        .on_hover_text("The line feed at the end of a line, alone or after a carriage return.");
                    ui.add_enabled(enabled, Checkbox::new(&mut whitespace.carriage_returns, "Carriage returns"))
                        .on_hover_text("A carriage return ending a line, as in CRLF and CR files, or inside one.");
                    ui.add_enabled(enabled, Checkbox::new(&mut whitespace.other, "Other spaces"))
                        .on_hover_text("Non-breaking and other Unicode spaces.");
                });
            });
            ui.end_row();
        });

        section(ui, "panes_scrolling", "Panes: Navigation and scrolling", |ui| {
            ui.label("Context above jumps:");
            ui.add(
                DragValue::new(&mut self.draft.jump_context_lines)
                    .range(0..=MAX_JUMP_CONTEXT_LINES)
                    .speed(0.1)
                    .suffix(" lines"),
            )
            .on_hover_text("Lines kept above a difference or conflict that navigation jumps to.");
            ui.end_row();

            ui.label("Scrolling:");
            ui.vertical(|ui| {
                ui.checkbox(&mut self.draft.smooth_scrolling, "Smooth scrolling")
                    .on_hover_text("Wheel scrolling and jumps glide to where they go. Off, they move at once.");
                let smooth = self.draft.smooth_scrolling;
                ui.horizontal(|ui| {
                    ui.add_enabled(smooth, Label::new("Duration"));
                    ui.add_enabled(
                        smooth,
                        DragValue::new(&mut self.draft.smooth_scroll_ms)
                            .range(MIN_SMOOTH_SCROLL_MS..=MAX_SMOOTH_SCROLL_MS)
                            .speed(2.0)
                            .suffix(" ms"),
                    )
                    .on_hover_text("How long a smooth scroll takes: shorter is snappier, longer is smoother.");
                });
                ui.horizontal(|ui| {
                    ui.label("Wheel step");
                    ui.add(
                        DragValue::new(&mut self.draft.scroll_step_lines)
                            .range(MIN_SCROLL_STEP_LINES..=MAX_SCROLL_STEP_LINES)
                            .speed(0.1)
                            .suffix(" lines"),
                    )
                    .on_hover_text("Lines one notch of the mouse wheel scrolls.");
                });
            });
            ui.end_row();
        });

        section(ui, "region_boxes", "Region boxes: Borders", |ui| {
            ui.label("Corner radius:");
            ui.add(
                DragValue::new(&mut self.draft.region_corner_radius)
                    .range(0..=MAX_REGION_CORNER_RADIUS)
                    .speed(0.1)
                    .suffix(" px"),
            )
            .on_hover_text("Rounding of the boxes drawn around the regions that differ.");
            ui.end_row();

            ui.label("Opacity of other regions:");
            ui.add(
                DragValue::new(&mut self.draft.other_region_border_percent)
                    .range(0..=MAX_OTHER_REGION_BORDER_PERCENT)
                    .speed(0.5)
                    .suffix(" %"),
            )
            .on_hover_text("Opacity of the borders of all regions except the one navigation is at.");
            ui.end_row();
        });

        section(ui, "jumped_to_region", "Jumped-to region: Highlight", |ui| {
            ui.label("Background:");
            ui.color_edit_button_srgba_unmultiplied(&mut self.draft.current_region_color)
                .on_hover_text("The background of the region navigation is at, in every pane, void space included.");
            ui.end_row();
        });

        section(ui, "alignments", "Alignments: Gutter and navigation", |ui| {
            ui.label("Gutter color:");
            ui.color_edit_button_srgba_unmultiplied(&mut self.draft.alignment_color)
                .on_hover_text("The color of the alignment marks in the gutters of the source panes.");
            ui.end_row();

            ui.label("Jump stops:");
            ui.checkbox(&mut self.draft.alignment_stops, "Previous and Next Difference stop at alignments")
                .on_hover_text("Also stop at the first region of every applied alignment, whether or not its documents differ there.");
            ui.end_row();
        });

        let dark = theme == Theme::Dark;
        let defaults = theme::palette(theme);
        section(ui, "document_colors", "Documents: Colors", |ui| {
            let colors = self.draft.colors_mut(dark);
            let name = theme_name(theme);
            ui.label("A:");
            color_override(
                ui,
                &mut colors.document_a,
                defaults.colors(Document::A).gutter_border,
                &format!("The color of A in the {name} theme: its pane header, and the gutter blocks and characters of B and C that differ from A only."),
            );
            ui.end_row();

            ui.label("B:");
            color_override(
                ui,
                &mut colors.document_b,
                defaults.colors(Document::B).gutter_border,
                &format!("The color of B in the {name} theme: its pane header, and the gutter blocks and characters of A and C that differ from B only."),
            );
            ui.end_row();

            ui.label("C:");
            color_override(
                ui,
                &mut colors.document_c,
                defaults.colors(Document::C).gutter_border,
                &format!("The color of C in the {name} theme: its pane header, and the gutter blocks and characters of A and B that differ from C only."),
            );
            ui.end_row();

            ui.label("Differs from all:");
            color_override(
                ui,
                &mut colors.red,
                defaults.red(),
                &format!("The color in the {name} theme of gutter blocks and characters that differ from every other document, and of the <Merge Conflict> marker."),
            );
            ui.end_row();
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "Colors are kept per theme; these are the {} theme's. Fills and borders are derived from them.",
                theme_name(theme)
            ))
            .weak(),
        );
    }

    /// Colors and theming of the window around the panes.
    fn appearance_tab(&mut self, ui: &mut Ui) {
        let theme = active_theme(ui);
        section(ui, "window_appearance", "Window: Appearance", |ui| {
            ui.label("Theme:");
            ComboBox::from_id_salt("theme_choice")
                .selected_text(theme_choice_name(self.draft.theme.clone()))
                .show_ui(ui, |ui| {
                    for choice in [ThemeChoice::Light, ThemeChoice::Dark, ThemeChoice::System] {
                        ui.selectable_value(&mut self.draft.theme, choice.clone(), theme_choice_name(choice));
                    }
                });
            ui.end_row();

            ui.label("UI text color:");
            color_override(
                ui,
                &mut self.draft.colors_mut(theme == Theme::Dark).ui_text,
                theme::palette(theme).text,
                &format!("Text of menus, dialogs and buttons, in the {} theme.", theme_name(theme)),
            );
            ui.end_row();

            ui.label("Corner radius:");
            ui.add(DragValue::new(&mut self.draft.corner_radius).range(0..=MAX_CORNER_RADIUS).speed(0.1).suffix(" px"))
                .on_hover_text("Rounding of widgets, menus, dialogs, panes and the window frame.");
            ui.end_row();
        });
        ui.add_space(4.0);
        ui.label(RichText::new(format!("Colors are kept per theme; these are the {} theme's.", theme_name(theme))).weak());
    }

    fn comparison_tab(&mut self, ui: &mut Ui) {
        let Some((_, draft)) = self.comparison.as_mut() else {
            ui.label(RichText::new("Open files to change how they are compared.").weak());
            return;
        };
        section(ui, "comparison_algorithm", "Comparison: Algorithm", |ui| {
            ui.label("Algorithm:");
            ComboBox::from_id_salt("diff_algorithm").selected_text(algorithm_name(draft.algorithm)).show_ui(ui, |ui| {
                for algorithm in [DiffAlgorithm::Histogram, DiffAlgorithm::Myers] {
                    ui.selectable_value(&mut draft.algorithm, algorithm, algorithm_name(algorithm));
                }
            });
            ui.end_row();

            ui.label("");
            let myers = draft.algorithm == DiffAlgorithm::Myers;
            ui.add_enabled(myers, egui::Checkbox::new(&mut draft.try_hard, "Minimal diff"))
                .on_hover_text("Myers only: find the smallest diff, which can be slow on large files.");
            ui.end_row();
        });
        section(ui, "comparison_char_diff", "Comparison: Character diff", |ui| {
            ui.label("Line limit:");
            ui.add(
                DragValue::new(&mut draft.char_diff_line_limit)
                    .range(0..=MAX_CHAR_DIFF_LINE_LIMIT)
                    .speed(10.0)
                    .suffix(" chars"),
            )
            .on_hover_text("Longer lines are marked as different as a whole.");
            ui.end_row();
        });
        ui.add_space(4.0);
        ui.label(RichText::new("Applied on OK. The files are compared again and D is rebuilt: its choices, edits and undo history are dropped.").weak());
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

/// A titled group of settings in its own rect, laid out as label and control rows. `title` starts with the name of what the settings change.
fn section(ui: &mut Ui, id: &str, title: &str, rows: impl FnOnce(&mut Ui)) {
    ui.add_space(6.0);
    ui.group(|ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).strong());
        ui.separator();
        egui::Grid::new(id).num_columns(2).spacing([12.0, 8.0]).show(ui, rows);
    });
}

/// The theme in effect: the chosen one, or the system's while following it.
fn active_theme(ui: &Ui) -> Theme {
    if ui.visuals().dark_mode { Theme::Dark } else { Theme::Light }
}

fn theme_name(theme: Theme) -> &'static str {
    match theme {
        Theme::Light => "light",
        Theme::Dark => "dark",
    }
}

fn theme_choice_name(choice: ThemeChoice) -> &'static str {
    match choice {
        ThemeChoice::Light => "Light",
        ThemeChoice::Dark => "Dark",
        ThemeChoice::System => "Follow the system",
    }
}

/// The color an override shows: the override, or the theme's own color.
fn shown_rgb(color: Option<[u8; 3]>, theme_color: Color32) -> [u8; 3] {
    color.unwrap_or([theme_color.r(), theme_color.g(), theme_color.b()])
}

/// A color button for an override of `theme_color`, and a button that goes back to the theme's color.
fn color_override(ui: &mut Ui, color: &mut Option<[u8; 3]>, theme_color: Color32, hint: &str) {
    ui.horizontal(|ui| {
        let mut rgb = shown_rgb(color.clone(), theme_color);
        if ui.color_edit_button_srgb(&mut rgb).on_hover_text(hint).changed() {
            *color = Some(rgb);
        }
        if ui.add_enabled(color.is_some(), Button::new("Theme default")).clicked() {
            *color = None;
        }
    });
}

fn algorithm_name(algorithm: DiffAlgorithm) -> &'static str {
    match algorithm {
        DiffAlgorithm::Histogram => "Histogram",
        DiffAlgorithm::Myers => "Myers",
    }
}

/// The scanned family names, plus `current` if the scan didn't find it (not installed, or the scan is still running), so the combo box can show it.
pub fn family_choices(families: &[MonospaceFamily], current: Option<&str>) -> Vec<String> {
    let mut names: Vec<String> = families.iter().map(|family| family.name.clone()).collect();
    if let Some(current) = current
        && !names.iter().any(|name| name == current)
    {
        names.insert(0, current.to_string());
    }
    names
}

fn show_preview(ui: &mut Ui) {
    let palette = &theme::palette_of(ui);
    theme::pane_frame(palette, theme::corner_radius_of(ui)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        theme::pane_body_frame().show(ui, |ui| {
            for line in PREVIEW_LINES {
                ui.label(RichText::new(line).monospace().color(palette.pane_text));
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
        let view = ViewConfig {
            font_family: Some("Consolas".to_string()),
            font_size: 15.0,
            font_hinting: false,
            subpixel_positioning: true,
            corner_radius: 4,
            ..ViewConfig::default()
        };
        let dialog = SettingsDialog::new(&view, None);
        assert_eq!(dialog.original(), &view);
        assert_eq!(dialog.draft(), &view);
        assert_eq!(dialog.changed_comparison(), None);
    }

    #[test]
    fn comparison_changes_are_reported_and_reset() {
        let options = ComparisonOptions {
            algorithm: DiffAlgorithm::Myers,
            ..ComparisonOptions::default()
        };
        let mut dialog = SettingsDialog::new(&ViewConfig::default(), Some(&options));
        assert_eq!(dialog.changed_comparison(), None);
        if let Some((_, draft)) = dialog.comparison.as_mut() {
            draft.try_hard = true;
        }
        assert_eq!(dialog.changed_comparison().map(|options| options.try_hard), Some(true));
        dialog.reset_to_defaults();
        assert_eq!(dialog.changed_comparison(), Some(&ComparisonOptions::default()));
    }

    #[test]
    fn toolbar_toggle_reaches_the_open_dialog() {
        let mut dialog = SettingsDialog::new(&ViewConfig::default(), None);
        dialog.set_align_output(true);
        assert!(dialog.original().align_output && dialog.draft().align_output);
    }

    #[test]
    fn minimap_toggle_reaches_the_open_dialog() {
        let mut dialog = SettingsDialog::new(&ViewConfig::default(), None);
        assert!(dialog.draft().show_minimap);
        dialog.set_show_minimap(false);
        assert!(!dialog.original().show_minimap && !dialog.draft().show_minimap);
    }

    #[test]
    fn overview_strip_toggle_reaches_the_open_dialog() {
        let mut dialog = SettingsDialog::new(&ViewConfig::default(), None);
        assert!(dialog.draft().show_overview_strip);
        dialog.set_show_overview_strip(false);
        assert!(!dialog.original().show_overview_strip && !dialog.draft().show_overview_strip);
    }

    #[test]
    fn the_alignment_checkbox_edits_the_draft_and_reset_restores_it() {
        use egui_kittest::kittest::Queryable;
        let dialog = SettingsDialog::new(&ViewConfig::default(), None);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(900.0, 3000.0))
            .with_accessibility_check(false)
            .build_ui_state(
                |ui, dialog: &mut SettingsDialog| {
                    dialog.show(ui, Some(&[]));
                },
                dialog,
            );
        harness.run_steps(3);
        harness.get_by_label("Previous and Next Difference stop at alignments").scroll_to_me();
        harness.run_steps(3);
        harness.get_by_label("Previous and Next Difference stop at alignments").click();
        harness.run_steps(3);
        assert!(!harness.state().draft().alignment_stops);
        harness.state_mut().reset_to_defaults();
        assert!(harness.state().draft().alignment_stops);
        assert_eq!(harness.state().draft().alignment_color, crate::config::DEFAULT_ALIGNMENT_COLOR);
    }

    #[test]
    fn reset_changes_only_the_draft() {
        let view = ViewConfig {
            font_family: Some("Consolas".to_string()),
            font_size: 20.0,
            font_hinting: false,
            subpixel_positioning: false,
            corner_radius: 9,
            ..ViewConfig::default()
        };
        let mut dialog = SettingsDialog::new(&view, None);
        dialog.reset_to_defaults();
        assert_eq!(dialog.draft(), &ViewConfig::default());
        assert_eq!(dialog.original(), &view);
    }

    #[test]
    fn overrides_show_the_theme_color_until_set() {
        assert_eq!(shown_rgb(None, Color32::from_rgb(7, 8, 9)), [7, 8, 9]);
        assert_eq!(shown_rgb(Some([1, 2, 3]), Color32::from_rgb(7, 8, 9)), [1, 2, 3]);
    }

    #[test]
    fn reset_restores_theme_colors_and_whitespace() {
        let mut view = ViewConfig {
            theme: ThemeChoice::Dark,
            ..ViewConfig::default()
        };
        view.dark_colors.pane_text = Some([1, 1, 1]);
        view.dark_colors.document_b = Some([2, 2, 2]);
        view.light_colors.red = Some([3, 3, 3]);
        view.whitespace.render = false;
        let mut dialog = SettingsDialog::new(&view, None);
        dialog.reset_to_defaults();
        assert_eq!(dialog.draft().theme, ThemeChoice::Light);
        assert_eq!(dialog.draft().dark_colors.pane_text, None);
        assert_eq!(dialog.draft().dark_colors.document_b, None);
        assert_eq!(dialog.draft().light_colors.red, None);
        assert!(dialog.draft().whitespace.render);
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
