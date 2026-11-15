use egui::{Align, Button, Checkbox, Color32, ComboBox, DragValue, Label, Layout, RichText, ScrollArea, TextEdit, Theme, Ui, vec2};
use kdiff4_protocol::{Document, Extractor};
use kdiff4_protocol::engine::{ComparisonOptions, DiffAlgorithm};

use crate::config::{
    MAX_CORNER_RADIUS, MAX_FONT_SIZE, MAX_INSET_ARROW_GAP, MAX_JUMP_CONTEXT_LINES, MAX_OTHER_REGION_BORDER_PERCENT, MAX_REGION_CORNER_RADIUS, MAX_SCROLL_STEP_LINES,
    MAX_SMOOTH_SCROLL_MS, MIN_FONT_SIZE, MIN_SCROLL_STEP_LINES, MAX_AUTOSCROLL_LINES_PER_SECOND, MIN_AUTOSCROLL_LINES_PER_SECOND, MIN_SMOOTH_SCROLL_MS, MAX_TAB_SIZE, TabIndentation, ThemeChoice, ViewConfig,
};
use crate::fonts::MonospaceFamily;
use crate::dialog_window::Resizable;
use crate::theme;

const BUILTIN_FONT_LABEL: &str = "Built-in (Hack)";
const MAX_CHAR_DIFF_LINE_LIMIT: usize = 1_000_000;
/// The dialog opens with this width whatever the tab; a tab taller than the body scrolls.
pub(crate) const DIALOG_WIDTH: f32 = 720.0;
pub(crate) const RESIZABLE: Resizable = Resizable { height: 640.0, min_size: vec2(480.0, 320.0) };
const SEARCH_WIDTH: f32 = 220.0;
const NO_MATCH_LABEL: &str = "No settings match the search.";
const PANES_TEXT: Section = Section { id: "panes_text", title: "Panes: Text", words: "font family size rendering hinting subpixel positioning text color preview monospace" };
const OUTPUT_INDENTATION: Section = Section { id: "output_indentation", title: "Merge output: Indentation", words: "tab inserts tabs spaces auto-detect tab size indent" };
const PANES_CONTENT: Section = Section { id: "panes_content", title: "Panes: Content", words: "show line numbers region gutters void space output pane overview strip minimap independent scrolling whitespace render" };
const PANES_SCROLLING: Section = Section { id: "panes_scrolling", title: "Panes: Navigation and scrolling", words: "context above jumps smooth scrolling animation wheel step selection drag speed middle-button scroll speed" };
const REGION_BOXES: Section = Section { id: "region_boxes", title: "Region boxes: Borders", words: "corner radius opacity of other regions" };
const JUMPED_TO_REGION: Section = Section { id: "jumped_to_region", title: "Jumped-to region: Highlight", words: "background color current region" };
const ALIGNMENTS: Section = Section { id: "alignments", title: "Alignments: Gutter and navigation", words: "gutter color inset arrow gap jump stops previous next difference" };
const DOCUMENT_COLORS: Section = Section { id: "document_colors", title: "Documents: Colors", words: "a b c differs from all red fold ellipsis color" };
pub const RESTORE_TABS_LABEL: &str = "Reopen the tabs of the last run on start";
const WINDOW_APPEARANCE: Section = Section { id: "window_appearance", title: "Window: Appearance", words: "theme light dark system ui text color corner radius" };
const WINDOW_TABS: Section = Section { id: "window_tabs", title: "Window: Tabs", words: "reopen restore tabs last run start" };
const BOTTOM_CONTROLS: Section = Section { id: "bottom_controls_appearance", title: "Bottom Controls", words: "bottom switch tint outline color" };
const SUCCESS_BLINK: Section = Section { id: "success_blink_appearance", title: "Success Blink", words: "enable blink color peak alpha animation time save" };
const HEURISTICS: Section = Section { id: "comparison_heuristics", title: "Heuristics Alignment", words: "extractor tree-sitter functions entities" };
const ALGORITHM: Section = Section { id: "comparison_algorithm", title: "Comparison: Algorithm", words: "algorithm diff myers patience histogram try hard pair similar changed lines" };
const CHAR_DIFF: Section = Section { id: "comparison_char_diff", title: "Comparison: Character diff", words: "line limit characters" };
const SECTIONS: [&Section; 15] = [&PANES_TEXT, &OUTPUT_INDENTATION, &PANES_CONTENT, &PANES_SCROLLING, &REGION_BOXES, &JUMPED_TO_REGION, &ALIGNMENTS, &DOCUMENT_COLORS, &WINDOW_APPEARANCE, &WINDOW_TABS, &BOTTOM_CONTROLS, &SUCCESS_BLINK, &HEURISTICS, &ALGORITHM, &CHAR_DIFF];


/// A titled group of settings. `words` are the lowercase words of its settings the search finds it by, besides its title.
struct Section {
    id: &'static str,
    title: &'static str,
    words: &'static str,
}


/// The search text as lowercase words. A section matches when it has every one of them; no words match everything.
struct Filter {
    words: Vec<String>,
}


impl Filter {
    fn new(search: &str) -> Self {
        let words = search
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        Self { words }
    }

    fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    fn matches(&self, section: &Section) -> bool {
        let title = section.title.to_lowercase();
        self.words
            .iter()
            .all(|word| title.contains(word.as_str()) || section.words.contains(word.as_str()))
    }
}
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
    /// What the search field holds. While it holds something, the matching sections of every tab are shown.
    search: String,
    /// Give the search field the keyboard on the next frame.
    focus_search: bool,
}

impl SettingsDialog {
    pub fn new(view: &ViewConfig, comparison: Option<&ComparisonOptions>) -> SettingsDialog {
        SettingsDialog {
            original: view.clone(),
            draft: view.clone(),
            comparison: comparison.map(|options| (options.clone(), options.clone())),
            tab: Tab::Panes,
            search: String::new(),
            focus_search: true,
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

    /// Follows the toolbar's ignore whitespace toggle, which also works while the dialog is open, so its OK doesn't undo it.
    pub fn set_ignore_whitespace(&mut self, ignore: bool) {
        if let Some((original, draft)) = self.comparison.as_mut() {
            original.ignore_whitespace = ignore.clone();
            draft.ignore_whitespace = ignore;
        }
    }

    /// Follows the toolbar's and the shortcut's minimap toggle, which also works while the dialog is open.
    pub fn set_show_minimap(&mut self, show: bool) {
        self.original.show_minimap = show;
        self.draft.show_minimap = show;
    }

    /// Follows the toolbar's and the shortcut's independent scrolling toggle, which also works while the dialog is open.
    pub fn set_independent_scroll(&mut self, independent: bool) {
        self.original.independent_scroll = independent;
        self.draft.independent_scroll = independent;
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
        ui.heading("Settings");
        ui.add_space(4.0);
        let filter = Filter::new(&self.search);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(filter.is_empty(), |ui| {
                ui.selectable_value(&mut self.tab, Tab::Panes, "Panes");
                ui.selectable_value(&mut self.tab, Tab::Appearance, "Appearance");
                ui.selectable_value(&mut self.tab, Tab::Comparison, "Comparison");
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let field = ui.add(TextEdit::singleline(&mut self.search).hint_text("Search settings").desired_width(SEARCH_WIDTH));
                if self.focus_search {
                    self.focus_search = false;
                    field.request_focus();
                }
                field.on_hover_text("Show the sections of every tab that have a setting with these words");
            });
        });
        ui.separator();
        // The buttons go in first, from the bottom, so the body gets whatever height the window leaves.
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
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
            ui.separator();
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                ScrollArea::vertical()
                    .id_salt("settings_body")
                    .auto_shrink(false)
                    .show(ui, |ui| self.body(ui, families, &filter));
            });
        });
        action
    }

    /// The sections of the chosen tab, or while searching the matching sections of every tab.
    fn body(&mut self, ui: &mut Ui, families: Option<&[MonospaceFamily]>, filter: &Filter) {
        if filter.is_empty() {
            match self.tab {
                Tab::Panes => self.panes_tab(ui, families, filter),
                Tab::Appearance => self.appearance_tab(ui, filter),
                Tab::Comparison => self.comparison_tab(ui, filter),
            }
            return;
        }
        if !SECTIONS.iter().any(|section| filter.matches(section)) {
            ui.add_space(6.0);
            ui.label(RichText::new(NO_MATCH_LABEL).weak());
            return;
        }
        self.panes_tab(ui, families, filter);
        self.appearance_tab(ui, filter);
        self.comparison_tab(ui, filter);
    }

    /// How the panes draw text and regions.
    fn panes_tab(&mut self, ui: &mut Ui, families: Option<&[MonospaceFamily]>, filter: &Filter) {
        let theme = active_theme(ui);
        section(ui, filter, &PANES_TEXT, |ui| {
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

        section(ui, filter, &OUTPUT_INDENTATION, |ui| {
            ui.label("Tab inserts:");
            ComboBox::from_id_salt("tab_indentation")
                .selected_text(match self.draft.tab_indentation {
                    TabIndentation::Tabs => "Tabs",
                    TabIndentation::Spaces => "Spaces",
                    TabIndentation::Auto => "Auto-detect",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.draft.tab_indentation, TabIndentation::Tabs, "Tabs");
                    ui.selectable_value(&mut self.draft.tab_indentation, TabIndentation::Spaces, "Spaces");
                    ui.selectable_value(&mut self.draft.tab_indentation, TabIndentation::Auto, "Auto-detect");
                })
                .response
                .on_hover_text("Auto-detect samples the merge output and uses tabs if the indentation is unclear.");
            ui.end_row();

            ui.label("Tab size:");
            ui.add(DragValue::new(&mut self.draft.tab_size).range(1..=MAX_TAB_SIZE))
                .on_hover_text("Columns per tab stop when inserting spaces. Auto-detect uses this size if it finds spaces but no clear width.");
            ui.end_row();
        });

        section(ui, filter, &PANES_CONTENT, |ui| {
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
                ui.checkbox(&mut self.draft.independent_scroll, "Independent scrolling")
                    .on_hover_text("Every pane scrolls on its own. Turned off, the panes line up again with the one last clicked or scrolled.");
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

        section(ui, filter, &PANES_SCROLLING, |ui| {
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
                ui.horizontal(|ui| {
                    ui.label("Selection drag speed");
                    ui.add(
                        DragValue::new(&mut self.draft.drag_scroll_lines_per_second)
                            .range(MIN_AUTOSCROLL_LINES_PER_SECOND..=MAX_AUTOSCROLL_LINES_PER_SECOND)
                            .speed(0.2)
                            .suffix(" lines/s"),
                    )
                    .on_hover_text("How fast a pane scrolls while you drag a selection past its edge, with the pointer just past it. It goes faster the farther out the pointer is.");
                });
                ui.horizontal(|ui| {
                    ui.label("Middle-button scroll speed");
                    ui.add(
                        DragValue::new(&mut self.draft.middle_scroll_lines_per_second)
                            .range(MIN_AUTOSCROLL_LINES_PER_SECOND..=MAX_AUTOSCROLL_LINES_PER_SECOND)
                            .speed(0.2)
                            .suffix(" lines/s"),
                    )
                    .on_hover_text("How fast a pane scrolls with the pointer 100 pixels from where the middle button went down. Hold the middle button and move, or click it to scroll until the next click or Escape.");
                });
            });
            ui.end_row();
        });

        section(ui, filter, &REGION_BOXES, |ui| {
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

        section(ui, filter, &JUMPED_TO_REGION, |ui| {
            ui.label("Background:");
            ui.color_edit_button_srgba_unmultiplied(&mut self.draft.current_region_color)
                .on_hover_text("The background of the region navigation is at, in every pane, void space included.");
            ui.end_row();
        });

        section(ui, filter, &ALIGNMENTS, |ui| {
            ui.label("Gutter color:");
            ui.color_edit_button_srgba_unmultiplied(&mut self.draft.alignment_color)
                .on_hover_text("The color of the alignment marks in the gutters of the source panes.");
            ui.end_row();

            ui.label("Inset arrow gap:");
            ui.add(DragValue::new(&mut self.draft.inset_arrow_gap).range(0..=MAX_INSET_ARROW_GAP).speed(0.1).suffix(" px"))
                .on_hover_text("Space between the lines of the arrows left of the line numbers that show where the hunks of an inset alignment were.");
            ui.end_row();

            ui.label("Jump stops:");
            ui.checkbox(&mut self.draft.alignment_stops, "Previous and Next Difference stop at alignments")
                .on_hover_text("Also stop at the first region of every applied alignment, whether or not its documents differ there.");
            ui.end_row();
        });

        let dark = theme == Theme::Dark;
        let defaults = theme::palette(theme);
        section(ui, filter, &DOCUMENT_COLORS, |ui| {
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

            ui.label("Fold Ellipsis:");
            color_override(
                ui,
                &mut colors.fold_ellipsis,
                defaults.fold_ellipsis,
                &format!("The color in the {name} theme of the ... that stands for the hidden lines of folded code, and of the button of a fold."),
            );
            ui.end_row();
        });
        if filter.matches(&DOCUMENT_COLORS) {
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "Colors are kept per theme; these are the {} theme's. Fills and borders are derived from them.",
                    theme_name(theme)
                ))
                .weak(),
            );
        }
    }

    /// Colors and theming of the window around the panes.
    fn appearance_tab(&mut self, ui: &mut Ui, filter: &Filter) {
        let theme = active_theme(ui);
        section(ui, filter, &WINDOW_APPEARANCE, |ui| {
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

            let label = ui.label("Corner radius:");
            ui.add(DragValue::new(&mut self.draft.corner_radius).range(0..=MAX_CORNER_RADIUS).speed(0.1).suffix(" px"))
                .labelled_by(label.id)
                .on_hover_text("Shared rounding for panes, buttons, tabs, Bottom Controls, menus, dialogs, and the window frame.");
            ui.end_row();
        });
        section(ui, filter, &WINDOW_TABS, |ui| {
            ui.label("");
            ui.checkbox(&mut self.draft.restore_tabs, RESTORE_TABS_LABEL)
                .on_hover_text("When Kdiff4 starts without files, open the comparisons its window had when it was closed, instead of the Open window. Only the files are opened again, not unsaved merge work, and tabs opened from a command line are not kept.");
            ui.end_row();
        });
        section(ui, filter, &BOTTOM_CONTROLS, |ui| {
            let colors = self.draft.colors_mut(theme == Theme::Dark);
            let palette = theme::palette(theme);
            ui.label("Bottom Switch tint:");
            color_override(ui, &mut colors.bottom_switch_tint, palette.bottom_switch_tint, "Background of a hovered or open Bottom Switch.");
            ui.end_row();
            ui.label("Bottom Switch outline:");
            color_override(ui, &mut colors.bottom_switch_outline, palette.bottom_switch_outline, "Thin outline around a hovered or open Bottom Switch.");
            ui.end_row();
        });
        if filter.matches(&WINDOW_APPEARANCE) || filter.matches(&BOTTOM_CONTROLS) {
            ui.add_space(4.0);
            ui.label(RichText::new(format!("Colors are kept per theme; these are the {} theme's.", theme_name(theme))).weak());
        }
        section(ui, filter, &SUCCESS_BLINK, |ui| {
            let blink = &mut self.draft.success_blink;
            ui.label("");
            ui.checkbox(&mut blink.enabled, "Enable Success Blink");
            ui.end_row();
            let label = ui.label("Blink color:");
            ui.color_edit_button_srgb(&mut blink.color).labelled_by(label.id);
            ui.end_row();
            let label = ui.label("Peak alpha:");
            ui.add(DragValue::new(&mut blink.alpha).range(0..=u8::MAX)).labelled_by(label.id);
            ui.end_row();
            let label = ui.label("Animation time:");
            ui.add(DragValue::new(&mut blink.duration_ms).range(1..=u16::MAX).suffix(" ms")).labelled_by(label.id);
            ui.end_row();
        });
    }

    fn comparison_tab(&mut self, ui: &mut Ui, filter: &Filter) {
        section(ui, filter, &HEURISTICS, |ui| {
            ui.label("Extractor:");
            ComboBox::from_id_salt("heuristic_extractor").selected_text(extractor_name(self.draft.heuristic_extractor)).show_ui(ui, |ui| {
                for extractor in [Extractor::TreeSitter] {
                    ui.selectable_value(&mut self.draft.heuristic_extractor, extractor, extractor_name(extractor));
                }
            })
            .response
            .on_hover_text("The parser that finds the functions, methods and types Heuristics Alignment pairs.");
            ui.end_row();
        });
        if !filter.matches(&ALGORITHM) && !filter.matches(&CHAR_DIFF) {
            return;
        }
        let Some((_, draft)) = self.comparison.as_mut() else {
            ui.label(RichText::new("Open files to change how they are compared.").weak());
            return;
        };
        section(ui, filter, &ALGORITHM, |ui| {
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

            ui.label("");
            ui.checkbox(&mut draft.pair_similar_lines, "Pair similar changed lines")
                .on_hover_text("Inside a changed region, put a line that changed a little next to its old version, even when lines were added or removed before it. Off pairs the lines by position, like kdiff3.");
            ui.end_row();
        });
        section(ui, filter, &CHAR_DIFF, |ui| {
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

/// A titled group of settings in its own rect, laid out as label and control rows, left out while the search doesn't match it. Its title starts with the name of what the settings change.
fn section(ui: &mut Ui, filter: &Filter, section: &Section, rows: impl FnOnce(&mut Ui)) {
    if !filter.matches(section) {
        return;
    }
    ui.add_space(6.0);
    ui.group(|ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(section.title).strong());
        ui.separator();
        egui::Grid::new(section.id).num_columns(2).spacing([12.0, 8.0]).show(ui, rows);
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


fn extractor_name(extractor: Extractor) -> &'static str {
    match extractor {
        Extractor::TreeSitter => "Tree-Sitter",
    }
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

    fn search_harness(search: &str) -> egui_kittest::Harness<'static, SettingsDialog> {
        let mut dialog = SettingsDialog::new(&ViewConfig::default(), Some(&ComparisonOptions::default()));
        dialog.search = search.to_string();
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 1600.0)).with_accessibility_check(false)
            .build_ui_state(|ui, dialog: &mut SettingsDialog| { dialog.show(ui, Some(&[])); }, dialog);
        harness.run();
        harness
    }

    #[test]
    fn sections_have_their_own_ids_and_lowercase_search_words() {
        for (index, section) in SECTIONS.iter().enumerate() {
            assert!(SECTIONS[..index].iter().all(|other| other.id != section.id), "{} is used twice", section.id);
            assert_eq!(section.words, section.words.to_lowercase(), "{}", section.id);
            assert!(!section.title.is_empty() && !section.words.is_empty(), "{}", section.id);
        }
    }

    #[test]
    fn the_search_matches_sections_by_every_word_in_title_or_settings() {
        assert!(Filter::new("").matches(&PANES_TEXT));
        assert!(Filter::new("  HINTING ").matches(&PANES_TEXT));
        assert!(Filter::new("panes font").matches(&PANES_TEXT));
        assert!(!Filter::new("font minimap").matches(&PANES_TEXT));
        assert!(Filter::new("corner").matches(&REGION_BOXES) && Filter::new("corner").matches(&WINDOW_APPEARANCE));
    }

    #[test]
    fn searching_shows_the_matching_sections_of_every_tab() {
        use egui_kittest::kittest::Queryable;

        let harness = search_harness("");
        harness.get_by_label(PANES_TEXT.title);
        assert!(harness.query_by_label(WINDOW_APPEARANCE.title).is_none(), "another tab");

        let harness = search_harness("corner radius");
        harness.get_by_label(REGION_BOXES.title);
        harness.get_by_label(WINDOW_APPEARANCE.title);
        assert!(harness.query_by_label(PANES_TEXT.title).is_none());
        assert!(harness.query_by_label(CHAR_DIFF.title).is_none());

        assert_eq!(harness.query_all_by_label_contains("Colors are kept per theme").count(), 1, "only the note of the window section");

        let harness = search_harness("line limit");
        harness.get_by_label(CHAR_DIFF.title);
        assert_eq!(harness.query_all_by_label_contains("Colors are kept per theme").count(), 0);
        harness.get_by_label_contains("Applied on OK");
        assert!(search_harness("hinting").query_by_label_contains("Applied on OK").is_none(), "the note belongs to the comparison sections");
    }

    #[test]
    fn the_search_field_has_the_keyboard_when_the_dialog_opens() {
        use egui_kittest::kittest::Queryable;

        let mut harness = search_harness("");
        assert!(harness.get_by_role(egui::Role::TextInput).is_focused());
        harness.event(egui::Event::Text("blink".to_string()));
        harness.run();
        harness.get_by_label(SUCCESS_BLINK.title);
        assert!(harness.query_by_label(PANES_TEXT.title).is_none());
    }

    #[test]
    fn the_tabs_section_turns_reopening_the_tabs_off() {
        use egui_kittest::kittest::Queryable;

        let mut harness = search_harness("reopen tabs");
        harness.get_by_label(WINDOW_TABS.title);
        assert!(harness.state().draft().restore_tabs);
        harness.get_by_label(RESTORE_TABS_LABEL).click();
        harness.run();
        assert!(!harness.state().draft().restore_tabs);
    }

    #[test]
    fn a_search_without_a_match_says_so() {
        use egui_kittest::kittest::Queryable;

        let harness = search_harness("no such setting");
        harness.get_by_label(NO_MATCH_LABEL);
        for section in SECTIONS {
            assert!(harness.query_by_label(section.title).is_none(), "{}", section.title);
        }
    }

    #[test]
    fn the_buttons_stay_at_the_bottom_and_the_body_takes_the_height_between() {
        use egui_kittest::kittest::Queryable;

        let harness = search_harness("");
        let ok = harness.get_by_label("OK").rect();
        assert!(ok.bottom() > 1500.0, "OK sits at the bottom of a tall window, not at {}", ok.bottom());
        assert!(harness.get_by_label(PANES_TEXT.title).rect().top() < 200.0);
    }

    #[test]
    fn appearance_exposes_the_shared_corner_radius() {
        use egui_kittest::kittest::Queryable;

        let view = ViewConfig { corner_radius: 7, ..ViewConfig::default() };
        let mut dialog = SettingsDialog::new(&view, None);
        dialog.tab = Tab::Appearance;
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 700.0)).with_accessibility_check(false)
            .build_ui_state(|ui, dialog: &mut SettingsDialog| { dialog.show(ui, Some(&[])); }, dialog);
        let control = harness.get_by_role_and_label(egui::Role::SpinButton, "Corner radius:");
        assert!(control.rect().is_positive());
        harness.get_by_label("Bottom Controls");
        harness.get_by_label("Bottom Switch tint:");
        harness.get_by_label("Bottom Switch outline:");
        harness.get_by_label("Success Blink");
        harness.get_by_label("Blink color:");
        harness.get_by_role_and_label(egui::Role::CheckBox, "Enable Success Blink").click();
        harness.get_by_role_and_label(egui::Role::SpinButton, "Peak alpha:").set_value_accesskit(128.0);
        harness.get_by_role_and_label(egui::Role::SpinButton, "Animation time:").set_value_accesskit(350.0);
        harness.run();
        let blink = harness.state().draft().success_blink;
        assert!(!blink.enabled);
        assert_eq!(blink.alpha, 128);
        assert_eq!(blink.duration_ms, 350);
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
    fn ignore_whitespace_toggle_reaches_the_open_dialog_without_counting_as_a_change() {
        let mut dialog = SettingsDialog::new(&ViewConfig::default(), Some(&ComparisonOptions::default()));
        dialog.set_ignore_whitespace(true);
        assert_eq!(dialog.changed_comparison(), None);
        if let Some((_, draft)) = dialog.comparison.as_mut() {
            draft.try_hard = true;
        }
        assert_eq!(dialog.changed_comparison().map(|options| options.ignore_whitespace), Some(true));
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
    fn independent_scroll_toggle_reaches_the_open_dialog() {
        let mut dialog = SettingsDialog::new(&ViewConfig::default(), None);
        assert!(!dialog.draft().independent_scroll);
        dialog.set_independent_scroll(true);
        assert!(dialog.original().independent_scroll && dialog.draft().independent_scroll);
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
    fn the_comparison_tab_toggles_similar_line_pairing_and_shows_the_extractor_without_a_session() {
        use egui_kittest::kittest::Queryable;
        let mut dialog = SettingsDialog::new(&ViewConfig::default(), Some(&ComparisonOptions::default()));
        dialog.tab = Tab::Comparison;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(900.0, 1200.0))
            .with_accessibility_check(false)
            .build_ui_state(
                |ui, dialog: &mut SettingsDialog| {
                    dialog.show(ui, Some(&[]));
                },
                dialog,
            );
        harness.run_steps(3);
        harness.get_by_label("Pair similar changed lines").click();
        harness.run_steps(3);
        assert!(harness.state().changed_comparison().is_some_and(|options| !options.pair_similar_lines));
        let mut without_session = SettingsDialog::new(&ViewConfig::default(), None);
        without_session.tab = Tab::Comparison;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(900.0, 1200.0))
            .with_accessibility_check(false)
            .build_ui_state(
                |ui, dialog: &mut SettingsDialog| {
                    dialog.show(ui, Some(&[]));
                },
                without_session,
            );
        harness.run_steps(3);
        harness.get_by_label("Extractor:");
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
        view.light_colors.fold_ellipsis = Some([4, 4, 4]);
        view.whitespace.render = false;
        let mut dialog = SettingsDialog::new(&view, None);
        dialog.reset_to_defaults();
        assert_eq!(dialog.draft().theme, ThemeChoice::System);
        assert_eq!(dialog.draft().dark_colors.pane_text, None);
        assert_eq!(dialog.draft().dark_colors.document_b, None);
        assert_eq!(dialog.draft().light_colors.red, None);
        assert_eq!(dialog.draft().light_colors.fold_ellipsis, None);
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
