use egui::{RichText, Ui};
use kdiff4_protocol::{EolChoice, EolStyle};

use crate::theme;

pub(crate) const DIALOG_WIDTH: f32 = 380.0;
const SELECTOR_WIDTH: f32 = 150.0;
const STYLES: [EolStyle; 3] = [EolStyle::Lf, EolStyle::Cr, EolStyle::CrLf];
pub const CONFLICT_LABEL: &str = "Conflict line ending";
pub const SELECTOR_HINT: &str = "Line ending style of the output";
pub const CONFLICT_HINT: &str = "The sources disagree on the line ending style. Choose one for the output.";
pub const CANCEL_LABEL: &str = "Cancel";


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EolAnswer {
    Use(EolStyle),
    Cancel,
}


pub fn name(style: EolStyle) -> &'static str {
    match style {
        EolStyle::Lf => "LF",
        EolStyle::Cr => "CR",
        EolStyle::CrLf => "CRLF",
    }
}


/// The drop-down for the status bar. In a conflict it reads "Conflict line ending" in red and has none of its entries selected. Returns the style the user picked this frame.
pub fn show_selector(ui: &mut Ui, current: &EolChoice) -> Option<EolStyle> {
    let mut picked = None;
    let selected = match current {
        EolChoice::Style(style) => RichText::new(name(style.clone())),
        EolChoice::Conflict => RichText::new(CONFLICT_LABEL).color(theme::palette_of(ui).red()),
    };
    let combo = egui::ComboBox::from_id_salt("output_eol_selector")
        .width(SELECTOR_WIDTH)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for style in STYLES {
                let selected = *current == EolChoice::Style(style.clone());
                if ui.selectable_label(selected, name(style.clone())).clicked() {
                    picked = Some(style);
                }
            }
        });
    combo.response.on_hover_text(if *current == EolChoice::Conflict { CONFLICT_HINT } else { SELECTOR_HINT });
    picked
}


/// The window that asks for the output's line ending style when the sources disagree and the user saves.
pub fn show_dialog(ui: &mut Ui) -> Option<EolAnswer> {
    let mut answer = None;
    ui.set_width(DIALOG_WIDTH);
    ui.heading("Choose the Line Ending Style");
    ui.add_space(4.0);
    ui.label("The sources use different line ending styles. Choose the one the output is saved with.");
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        for style in STYLES {
            if ui.button(name(style.clone())).clicked() {
                answer = Some(EolAnswer::Use(style));
            }
        }
        if ui.button(CANCEL_LABEL).clicked() {
            answer = Some(EolAnswer::Cancel);
        }
    });
    answer
}


#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;

    use super::*;

    fn dialog() -> egui_kittest::Harness<'static, Option<EolAnswer>> {
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(500.0, 300.0)).build_ui_state(
            |ui, answer: &mut Option<EolAnswer>| {
                if let Some(given) = show_dialog(ui) {
                    *answer = Some(given);
                }
            },
            None,
        );
        harness.run_steps(3);
        harness
    }

    #[test]
    fn styles_are_named_as_users_know_them() {
        assert_eq!([name(EolStyle::Lf), name(EolStyle::Cr), name(EolStyle::CrLf)], ["LF", "CR", "CRLF"]);
    }

    #[test]
    fn each_style_button_answers_with_its_style() {
        for style in STYLES {
            let mut harness = dialog();
            harness.get_by_label(name(style.clone())).click();
            harness.run_steps(2);
            assert_eq!(*harness.state(), Some(EolAnswer::Use(style)));
        }
    }

    #[test]
    fn cancel_answers_cancel() {
        let mut harness = dialog();
        harness.get_by_label(CANCEL_LABEL).click();
        harness.run_steps(2);
        assert_eq!(*harness.state(), Some(EolAnswer::Cancel));
    }
}
