use std::ops::Range;

use egui::{DragValue, Grid, RichText, ScrollArea, Ui};
use kdiff4_protocol::{Document, EntityKind, HeuristicMatch, HeuristicOptions, HeuristicProposal, MatchStage};

pub(crate) const DIALOG_WIDTH: f32 = 760.0;
const TABLE_HEIGHT: f32 = 320.0;
pub const BLOCKS_LABEL: &str = "Blocks inside functions";
pub const FIND_LABEL: &str = "Find Matches";
pub const ALIGN_LABEL: &str = "Align";
pub const CANCEL_LABEL: &str = "Cancel";
pub const CHECK_ALL_LABEL: &str = "Check All";
pub const CHECK_NONE_LABEL: &str = "Check None";
const INDENT: f32 = 16.0;
const STALE_HINT: &str = "The options changed: Find Matches again to update the list.";


#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeuristicsAction {
    /// Ask the engine for matches with the current options.
    Find,
    /// Align the checked matches: one part per source document each.
    Align(Vec<Vec<Range<usize>>>),
    Cancel,
}


/// The Heuristics Alignment window: the options, and the matches the engine proposed for them, each with a checkbox.
#[derive(Debug, Clone, PartialEq)]
pub struct HeuristicsDialog {
    pub options: HeuristicOptions,
    language: String,
    documents: usize,
    /// The applied alignment the search is limited to.
    within: Option<u32>,
    proposal: Option<HeuristicProposal>,
    checked: Vec<bool>,
    /// The options the proposal was found with. The list is stale when the options differ.
    found_with: Option<HeuristicOptions>,
    searching: bool,
    error: Option<String>,
}

impl HeuristicsDialog {
    pub fn new(options: HeuristicOptions, language: &str, documents: usize) -> HeuristicsDialog {
        HeuristicsDialog {
            options,
            language: language.to_string(),
            documents,
            within: None,
            proposal: None,
            checked: Vec::new(),
            found_with: None,
            searching: false,
            error: None,
        }
    }

    /// The same window limited to what lies inside the applied alignment `number`.
    pub fn inside(mut self, number: u32) -> HeuristicsDialog {
        self.within = Some(number);
        self
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    pub fn within(&self) -> Option<u32> {
        self.within
    }

    /// The engine was asked for matches with the current options.
    pub fn start_search(&mut self) {
        self.searching = true;
        self.error = None;
    }

    pub fn set_proposal(&mut self, proposal: HeuristicProposal) {
        self.checked = vec![true; proposal.matches.len()];
        self.proposal = Some(proposal);
        self.found_with = Some(self.options.clone());
        self.searching = false;
        self.error = None;
    }

    pub fn set_error(&mut self, message: String) {
        self.searching = false;
        self.error = Some(message);
    }

    /// Drops the matches, whose lines no longer fit documents that were compared again.
    pub fn forget_proposal(&mut self) {
        self.proposal = None;
        self.checked.clear();
        self.found_with = None;
    }

    pub fn is_stale(&self) -> bool {
        self.found_with.as_ref() != Some(&self.options)
    }

    pub fn checked_parts(&self) -> Vec<Vec<Range<usize>>> {
        let Some(proposal) = self.proposal.as_ref() else {
            return Vec::new();
        };
        proposal
            .matches
            .iter()
            .zip(self.checked.iter())
            .filter(|(_, checked)| **checked)
            .map(|(found, _)| found.parts.clone())
            .collect()
    }


    pub fn show(&mut self, ui: &mut Ui) -> Option<HeuristicsAction> {
        let mut action = None;
        let width = DIALOG_WIDTH.min(ui.available_width());
        ui.set_width(width);
        ui.heading("Heuristics Alignment");
        if let Some(number) = self.within {
            ui.strong(format!("Inside alignment {number}"));
        }
        ui.add_space(4.0);
        let scope = if self.within.is_some() { "inside that alignment" } else { "in the documents" };
        ui.label(format!(
            "Finds the functions, methods, types and blocks {scope} of the {} documents, pairs them across the documents and aligns the checked pairs; what lies inside a pair is paired in turn. Pairs that a document holds in another order are exchanged like inset alignments. Your own marks stay, and pairs that collide with them are left out.",
            self.language
        ));
        ui.add_space(6.0);
        self.options_grid(ui);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(!self.searching, egui::Button::new(FIND_LABEL)).clicked() {
                action = Some(HeuristicsAction::Find);
            }
            self.summary(ui);
        });
        ui.add_space(4.0);
        self.table(ui, width);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let has_matches = !self.checked.is_empty();
            if ui.add_enabled(has_matches, egui::Button::new(CHECK_ALL_LABEL)).clicked() {
                self.checked.iter_mut().for_each(|checked| *checked = true);
            }
            if ui.add_enabled(has_matches, egui::Button::new(CHECK_NONE_LABEL)).clicked() {
                self.checked.iter_mut().for_each(|checked| *checked = false);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(CANCEL_LABEL).clicked() {
                    action = Some(HeuristicsAction::Cancel);
                }
                let can_align = !self.is_stale() && !self.searching && self.checked.iter().any(|checked| *checked);
                if ui.add_enabled(can_align, egui::Button::new(ALIGN_LABEL)).clicked() {
                    action = Some(HeuristicsAction::Align(self.checked_parts()));
                }
            });
        });
        action
    }


    fn options_grid(&mut self, ui: &mut Ui) {
        let three_way = self.documents > 2;
        let scoped = self.within.is_some();
        let options = &mut self.options;
        Grid::new("heuristics_options").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Entities:");
            ui.horizontal(|ui| {
                ui.checkbox(&mut options.functions, "Functions");
                ui.checkbox(&mut options.methods, "Methods")
                    .on_hover_text("Functions defined in a class, or with a qualified name such as Widget::draw.");
                ui.checkbox(&mut options.types, "Types")
                    .on_hover_text("Classes, structs, unions and enums. A type holds the methods inside it, which pair in turn; one without methods to align is aligned as a whole.");
                if scoped {
                    let mut always = true;
                    ui.add_enabled(false, egui::Checkbox::new(&mut always, BLOCKS_LABEL))
                        .on_disabled_hover_text("Blocks are always looked for inside an alignment.");
                } else {
                    ui.checkbox(&mut options.blocks, BLOCKS_LABEL)
                        .on_hover_text("Conditions, loops, switches with their cases, try statements and lambdas of at least 3 lines inside functions and methods. They pair in order only.");
                }
            });
            ui.end_row();

            ui.label("Pairing:");
            ui.vertical(|ui| {
                ui.label(RichText::new("Equal signatures always pair.").weak());
                ui.checkbox(&mut options.match_names, "Same name when the signature changed");
                ui.checkbox(&mut options.match_similar, "Similar signature and body, for renamed entities");
                ui.add_enabled_ui(options.match_similar, |ui| {
                    ui.horizontal(|ui| {
                        let label = ui.label("Signature at least");
                        ui.add(DragValue::new(&mut options.signature_similarity).range(50..=100).suffix("%")).labelled_by(label.id);
                        let label = ui.label("Body at least");
                        ui.add(DragValue::new(&mut options.body_similarity).range(0..=100).suffix("%")).labelled_by(label.id);
                    });
                });
                ui.checkbox(&mut options.overload_leftovers, "Pair the one overload left after equal signatures")
                    .on_hover_text("Otherwise overloads pair only by an equal signature, so a changed overload is never paired with the wrong one.");
                ui.add_enabled(three_way, egui::Checkbox::new(&mut options.missing_by_neighbors, "Anchor entities one document lacks between their neighbors"))
                    .on_hover_text("When the entities above and below it are next to each other in that document too. Needs three documents.");
            });
            ui.end_row();
        });
    }


    fn summary(&self, ui: &mut Ui) {
        if self.searching {
            ui.spinner();
            ui.label("Finding matches…");
            return;
        }
        if let Some(error) = self.error.as_ref() {
            ui.colored_label(ui.visuals().error_fg_color, error);
            return;
        }
        let Some(proposal) = self.proposal.as_ref() else { return };
        if self.is_stale() {
            ui.label(RichText::new(STALE_HINT).weak());
            return;
        }
        ui.label(summary_text(proposal));
    }


    fn table(&mut self, ui: &mut Ui, width: f32) {
        let Some(proposal) = self.proposal.as_ref() else { return };
        let stale = self.is_stale();
        let documents = self.documents;
        let checked = &mut self.checked;
        let mut toggled = None;
        ScrollArea::both().max_width(width).max_height(TABLE_HEIGHT).auto_shrink([false, true]).show(ui, |ui| {
            ui.add_enabled_ui(!stale, |ui| {
                Grid::new("heuristics_matches").num_columns(documents + 4).striped(true).spacing([12.0, 4.0]).show(ui, |ui| {
                    ui.label("");
                    ui.strong("Kind");
                    for document in Document::ALL.iter().take(documents) {
                        ui.strong(document.letter());
                    }
                    ui.strong("Paired by");
                    ui.strong("Moved");
                    ui.end_row();
                    for (row, (found, checked)) in proposal.matches.iter().zip(checked.iter_mut()).enumerate() {
                        let indent = depth(&proposal.matches, row) as f32 * INDENT;
                        ui.horizontal(|ui| {
                            ui.add_space(indent);
                            if ui.checkbox(checked, format!("{}", row + 1)).changed() {
                                toggled = Some((row, *checked));
                            }
                        });
                        ui.label(kind_label(&found.kind));
                        for index in 0..documents {
                            ui.horizontal(|ui| {
                                ui.add_space(indent);
                                ui.label(part_label(found, index));
                            });
                        }
                        ui.label(stage_label(&found.stage));
                        ui.label(if found.moved { "moved" } else { "" });
                        ui.end_row();
                    }
                });
            });
        });
        if let Some((row, value)) = toggled {
            set_checked(&proposal.matches, &mut self.checked, row, value);
        }
    }
}


/// How many matches hold the match at `row`.
fn depth(matches: &[HeuristicMatch], row: usize) -> usize {
    let mut depth = 0;
    let mut current = matches.get(row).and_then(|found| found.parent);
    while let Some(parent) = current {
        depth += 1;
        current = matches.get(parent).and_then(|found| found.parent);
    }
    depth
}


fn set_checked(matches: &[HeuristicMatch], checked: &mut [bool], row: usize, value: bool) {
    let Some(slot) = checked.get_mut(row) else { return };
    *slot = value;
    if value {
        let mut current = matches.get(row).and_then(|found| found.parent);
        while let Some(parent) = current {
            if let Some(slot) = checked.get_mut(parent) {
                *slot = true;
            }
            current = matches.get(parent).and_then(|found| found.parent);
        }
        return;
    }
    for later in row + 1..matches.len().min(checked.len()) {
        let mut current = matches[later].parent;
        while let Some(parent) = current {
            if parent == row {
                checked[later] = false;
                break;
            }
            current = matches.get(parent).and_then(|found| found.parent);
        }
    }
}


pub fn summary_text(proposal: &HeuristicProposal) -> String {
    let found = proposal
        .found
        .iter()
        .zip(Document::ALL.iter())
        .map(|(count, document)| format!("{} {count}", document.letter()))
        .collect::<Vec<_>>()
        .join(", ");
    let moved = proposal.matches.iter().filter(|found| found.moved).count();
    let mut text = format!("Entities found: {found}. {} pairs, {moved} moved.", proposal.matches.len());
    if proposal.blocked > 0 {
        text.push_str(&format!(" {} left out for your marks.", proposal.blocked));
    }
    text
}


fn kind_label(kind: &EntityKind) -> &'static str {
    match kind {
        EntityKind::Function => "function",
        EntityKind::Method => "method",
        EntityKind::Type => "type",
        EntityKind::Block => "block",
    }
}


/// The name and 1-based lines of the part of document `index`, or where its anchor is.
pub fn part_label(found: &HeuristicMatch, index: usize) -> String {
    let lines = found.parts.get(index).cloned().unwrap_or(0..0);
    match found.names.get(index).cloned().flatten() {
        Some(name) => format!("{name}  {}–{}", lines.start + 1, lines.end),
        None => format!("anchor before line {}", lines.start + 1),
    }
}


pub fn stage_label(stage: &MatchStage) -> String {
    match stage {
        MatchStage::Signature => "signature".to_string(),
        MatchStage::Name => "name".to_string(),
        MatchStage::Similar { signature, body } => format!("similar: signature {signature}%, body {body}%"),
    }
}


#[cfg(test)]
mod tests {
    use egui_kittest::kittest::{NodeT, Queryable};

    use super::*;

    fn found(name: &str, lines: Range<usize>, moved: bool) -> HeuristicMatch {
        HeuristicMatch {
            kind: EntityKind::Function,
            names: vec![Some(name.to_string()), Some(name.to_string()), None],
            parts: vec![lines.clone(), lines.clone(), lines.start..lines.start],
            stage: MatchStage::Signature,
            parent: None,
            moved,
        }
    }

    fn proposal() -> HeuristicProposal {
        HeuristicProposal {
            cause: None,
            matches: vec![found("trim", 0..3, true), found("split", 4..9, false)],
            found: vec![2, 2, 0],
            blocked: 1,
        }
    }

    fn dialog() -> egui_kittest::Harness<'static, (HeuristicsDialog, Option<HeuristicsAction>)> {
        let mut dialog = HeuristicsDialog::new(HeuristicOptions::default(), "C++", 3);
        dialog.start_search();
        dialog.set_proposal(proposal());
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(
            |ui, (dialog, action): &mut (HeuristicsDialog, Option<HeuristicsAction>)| {
                if let Some(given) = dialog.show(ui) {
                    *action = Some(given);
                }
            },
            (dialog, None),
        );
        harness.run();
        harness
    }


    #[test]
    fn labels_name_lines_anchors_and_stages() {
        let found = found("trim", 0..3, false);
        assert_eq!(part_label(&found, 0), "trim  1–3");
        assert_eq!(part_label(&found, 2), "anchor before line 1");
        assert_eq!(stage_label(&MatchStage::Similar { signature: 90, body: 75 }), "similar: signature 90%, body 75%");
        assert_eq!(summary_text(&proposal()), "Entities found: A 2, B 2, C 0. 2 pairs, 1 moved. 1 left out for your marks.");
    }


    #[test]
    fn a_very_long_name_scrolls_the_table_instead_of_widening_the_window() {
        let long = "n".repeat(400);
        let mut dialog = HeuristicsDialog::new(HeuristicOptions::default(), "C++", 3);
        dialog.start_search();
        dialog.set_proposal(HeuristicProposal { cause: None, matches: vec![found(&long, 0..3, false)], found: vec![1, 1, 0], blocked: 0 });
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(2400.0, 700.0)).build_ui_state(
            |ui, (dialog, width, action): &mut (HeuristicsDialog, f32, Option<HeuristicsAction>)| {
                if let Some(given) = dialog.show(ui) {
                    *action = Some(given);
                }
                *width = ui.min_rect().width();
            },
            (dialog, 0.0, None),
        );
        harness.run();
        assert!(harness.state().1 <= DIALOG_WIDTH + 2.0, "dialog is {} wide", harness.state().1);
        harness.get_by_label(ALIGN_LABEL).click();
        harness.run();
        assert!(matches!(harness.state().2, Some(HeuristicsAction::Align(_))));
    }


    #[test]
    fn align_sends_the_checked_matches() {
        let mut harness = dialog();
        harness.get_by_label(ALIGN_LABEL).click();
        harness.run();
        assert_eq!(harness.state().1, Some(HeuristicsAction::Align(vec![vec![0..3, 0..3, 0..0], vec![4..9, 4..9, 4..4]])));
    }


    #[test]
    fn unchecked_matches_are_left_out_and_none_checked_disables_align() {
        let mut dialog = HeuristicsDialog::new(HeuristicOptions::default(), "C++", 3);
        dialog.set_proposal(proposal());
        dialog.checked[0] = false;
        assert_eq!(dialog.checked_parts(), vec![vec![4..9, 4..9, 4..4]]);
        let mut harness = self::dialog();
        harness.get_by_label(CHECK_NONE_LABEL).click();
        harness.run();
        harness.get_by_label(ALIGN_LABEL).click();
        harness.run();
        assert_eq!(harness.state().1, None);
    }


    #[test]
    fn changed_options_make_the_list_stale_until_found_again() {
        let mut dialog = HeuristicsDialog::new(HeuristicOptions::default(), "C++", 3);
        dialog.set_proposal(proposal());
        assert!(!dialog.is_stale());
        dialog.options.types = true;
        assert!(dialog.is_stale());
        dialog.set_proposal(proposal());
        assert!(!dialog.is_stale());
        dialog.forget_proposal();
        assert!(dialog.is_stale());
        assert!(dialog.checked_parts().is_empty());
    }


    #[test]
    fn find_and_cancel_answer() {
        let mut harness = dialog();
        harness.get_by_label(FIND_LABEL).click();
        harness.run();
        assert_eq!(harness.state().1, Some(HeuristicsAction::Find));
        harness.get_by_label(CANCEL_LABEL).click();
        harness.run();
        assert_eq!(harness.state().1, Some(HeuristicsAction::Cancel));
    }


    fn nested_proposal() -> HeuristicProposal {
        HeuristicProposal {
            cause: None,
            matches: vec![
                found("outer", 0..9, false),
                HeuristicMatch {
                    parent: Some(0),
                    ..inside_of(found("inner", 2..5, false))
                },
                HeuristicMatch {
                    parent: Some(1),
                    ..inside_of(found("deepest", 3..4, false))
                },
                found("other", 10..12, false),
            ],
            found: vec![4, 4, 0],
            blocked: 0,
        }
    }

    fn inside_of(found: HeuristicMatch) -> HeuristicMatch {
        HeuristicMatch {
            kind: EntityKind::Block,
            ..found
        }
    }

    fn nested_dialog(scope: Option<u32>) -> egui_kittest::Harness<'static, (HeuristicsDialog, Option<HeuristicsAction>)> {
        let mut dialog = HeuristicsDialog::new(HeuristicOptions::default(), "C++", 3);
        if let Some(number) = scope {
            dialog = dialog.inside(number);
        }
        dialog.start_search();
        dialog.set_proposal(nested_proposal());
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(
            |ui, (dialog, action): &mut (HeuristicsDialog, Option<HeuristicsAction>)| {
                if let Some(given) = dialog.show(ui) {
                    *action = Some(given);
                }
            },
            (dialog, None),
        );
        harness.run();
        harness
    }


    #[test]
    fn nested_matches_are_indented_by_depth_and_blocks_are_labeled() {
        let harness = nested_dialog(None);
        let left = |text: &str| harness.get_all_by_label(text).next().expect("Test: the label is shown").rect().left();
        assert!(left("inner  3–5") > left("outer  1–9"));
        assert!(left("deepest  4–4") > left("inner  3–5"));
        assert_eq!(left("other  11–12"), left("outer  1–9"));
        assert_eq!(harness.get_all_by_label("block").count(), 2);
        assert_eq!(harness.get_all_by_label("function").count(), 2);
    }


    #[test]
    fn unchecking_a_match_unchecks_the_ones_inside_it_and_checking_brings_the_ones_around() {
        let mut harness = nested_dialog(None);
        harness.get_by_label("2").click();
        harness.run();
        assert_eq!(harness.state().0.checked, vec![true, false, false, true]);
        harness.get_by_label("3").click();
        harness.run();
        assert_eq!(harness.state().0.checked, vec![true, true, true, true]);
        harness.get_by_label("1").click();
        harness.run();
        assert_eq!(harness.state().0.checked, vec![false, false, false, true]);
        harness.get_by_label("3").click();
        harness.run();
        assert_eq!(harness.state().0.checked, vec![true, true, true, true]);
        harness.get_by_label(ALIGN_LABEL).click();
        harness.run();
        assert_eq!(harness.state().1, Some(HeuristicsAction::Align(harness.state().0.checked_parts())));
    }


    #[test]
    fn a_scoped_window_names_the_alignment_and_locks_blocks_on() {
        let harness = nested_dialog(Some(3));
        assert_eq!(harness.state().0.within(), Some(3));
        harness.get_by_label("Inside alignment 3");
        assert!(harness.get_by_label(BLOCKS_LABEL).accesskit_node().is_disabled());
        let harness = nested_dialog(None);
        assert!(harness.query_by_label("Inside alignment 3").is_none());
        assert!(!harness.get_by_label(BLOCKS_LABEL).accesskit_node().is_disabled());
    }
}
