use egui::Ui;
use kdiff4_protocol::InsetChoice;

use crate::alignment_gutter;

const DIALOG_WIDTH: f32 = 440.0;
const SUGGESTED_SUFFIX: &str = " (suggested)";
pub const KEEP_LABEL: &str = "Keep Suggestion";


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsetAnswer {
    /// Show the alignment at this place among the others.
    Place(usize),
    /// Leave it where the suggestion put it.
    Keep,
}


/// The window that asks where an alignment the documents disagree on should be shown. The suggestion is in effect already, so closing the window keeps it.
pub fn show(ui: &mut Ui, choice: &InsetChoice) -> Option<InsetAnswer> {
    let mut answer = None;
    ui.set_width(DIALOG_WIDTH);
    ui.heading(format!("Where Should Alignment {} Be Shown?", choice.number));
    ui.add_space(4.0);
    ui.label(format!(
        "The documents don't agree on the order of alignment {}. It is shown in one place, and the documents that have it elsewhere exchange its hunk with the one that has its place there. Apply Alignments does the exchange; an arrow then shows where each moved hunk was.",
        choice.number
    ));
    ui.add_space(6.0);
    for (index, option) in choice.options.iter().enumerate() {
        let suggested = index == choice.current;
        let label = alignment_gutter::option_label(option);
        let label = if suggested { format!("{label}{SUGGESTED_SUFFIX}") } else { label };
        if ui.button(label).clicked() {
            answer = Some(if suggested { InsetAnswer::Keep } else { InsetAnswer::Place(option.slot) });
        }
    }
    ui.add_space(6.0);
    if ui.button(KEEP_LABEL).clicked() {
        answer = Some(InsetAnswer::Keep);
    }
    answer
}


#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;
    use kdiff4_protocol::{Document, InsetOption};

    use super::*;

    fn choice() -> InsetChoice {
        InsetChoice {
            number: 2,
            current: 0,
            options: vec![
                InsetOption {
                    slot: 0,
                    above: None,
                    below: Some(1),
                    moved: vec![Document::A, Document::C],
                },
                InsetOption {
                    slot: 1,
                    above: Some(1),
                    below: None,
                    moved: vec![Document::B],
                },
            ],
        }
    }

    fn dialog() -> egui_kittest::Harness<'static, Option<InsetAnswer>> {
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(600.0, 400.0)).build_ui_state(
            |ui, answer: &mut Option<InsetAnswer>| {
                if let Some(given) = show(ui, &choice()) {
                    *answer = Some(given);
                }
            },
            None,
        );
        harness.run_steps(3);
        harness
    }

    #[test]
    fn the_window_names_the_alignment_and_every_place() {
        let harness = dialog();
        harness.get_by_label("Where Should Alignment 2 Be Shown?");
        harness.get_by_label("Above alignment 1 (moves A and C) (suggested)");
        harness.get_by_label("Below alignment 1 (moves B)");
    }

    #[test]
    fn picking_another_place_places_the_alignment_there() {
        let mut harness = dialog();
        harness.get_by_label("Below alignment 1 (moves B)").click();
        harness.run_steps(2);
        assert_eq!(*harness.state(), Some(InsetAnswer::Place(1)));
    }

    #[test]
    fn picking_the_suggestion_or_keeping_it_changes_nothing() {
        let mut harness = dialog();
        harness.get_by_label("Above alignment 1 (moves A and C) (suggested)").click();
        harness.run_steps(2);
        assert_eq!(*harness.state(), Some(InsetAnswer::Keep));
        let mut harness = dialog();
        harness.get_by_label(KEEP_LABEL).click();
        harness.run_steps(2);
        assert_eq!(*harness.state(), Some(InsetAnswer::Keep));
    }
}
