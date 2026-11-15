use egui::{Align2, Area, Context, Frame, Id, Key, Order, Pos2, RichText, TextEdit};
use thiserror::Error;

use crate::editing::Position;

const FIELD_ID: &str = "go_to_line_field";
const FIELD_WIDTH: f32 = 110.0;
const RANGE_WIDTH: f32 = 130.0;


/// The Go to Line bar: a field for a line, or a line and a column, of the pane with the caret.
#[derive(Debug, Clone, Default)]
pub struct GoToLineState {
    pub open: bool,
    pub text: String,
    /// Give the field the keyboard on the next frame.
    pub focus: bool,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoToLineEvent {
    /// Enter on a position; the column may be past the end of its line.
    Go(Position),
    Close,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum GoToLineError {
    #[error("Not a line number")]
    NotANumber,
    #[error("Lines start at 1")]
    Zero,
}


/// A number counted from 1, or `usize::MAX` for one too big to hold.
fn number(text: &str) -> Result<usize, GoToLineError> {
    let text = text.trim();
    if text.is_empty() || !text.chars().all(|c| c.is_ascii_digit()) {
        return Err(GoToLineError::NotANumber);
    }
    Ok(text.parse().unwrap_or(usize::MAX))
}


/// The position `text` names among `line_count` lines: `line`, or `line:column` (a comma works too), both counted from 1. A line past the end is the last one, and a column of 0 is the first. Blank text names nothing.
pub fn parse(text: &str, line_count: usize) -> Result<Option<Position>, GoToLineError> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    let (line_text, column_text) = match text.split_once([':', ',']) {
        Some((line, column)) => (line, Some(column)),
        None => (text, None),
    };
    let line = number(line_text)?;
    if line == 0 {
        return Err(GoToLineError::Zero);
    }
    let column = match column_text {
        Some(column) if !column.trim().is_empty() => number(column)?,
        _ => 1,
    };
    Ok(Some(Position { line: (line - 1).min(line_count.saturating_sub(1)), column: column.saturating_sub(1) }))
}


/// The Go to Line bar, floating at the top right of `top_right`: a field and the lines there are, or what is wrong with the text. Enter goes to the position and Escape closes.
pub fn show_bar(ctx: &Context, tab: usize, top_right: Pos2, state: &mut GoToLineState, line_count: usize) -> Option<GoToLineEvent> {
    let mut event = None;
    let field_id = Id::unique((FIELD_ID, tab));
    Area::new(Id::unique(("go_to_line_bar", tab)))
        .order(Order::Foreground)
        .fixed_pos(top_right)
        .pivot(Align2::RIGHT_TOP)
        .show(ctx, |ui| {
            Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let field = ui.add(TextEdit::singleline(&mut state.text).id(field_id).hint_text("Line or line:column").desired_width(FIELD_WIDTH));
                    if state.focus {
                        state.focus = false;
                        field.request_focus();
                    }
                    let parsed = parse(&state.text, line_count);
                    let (pressed_enter, pressed_escape) = ctx.input(|input| (input.key_pressed(Key::Enter), input.key_pressed(Key::Escape)));
                    if field.lost_focus() && pressed_enter {
                        match parsed {
                            Ok(Some(position)) => event = Some(GoToLineEvent::Go(position)),
                            _ => field.request_focus(),
                        }
                    }
                    if (field.has_focus() || field.lost_focus()) && pressed_escape {
                        event = Some(GoToLineEvent::Close);
                    }
                    let (text, color) = match parsed {
                        Err(error) => (error.to_string(), ui.visuals().error_fg_color),
                        Ok(_) => (format!("1 - {}", line_count.max(1)), ui.visuals().weak_text_color()),
                    };
                    ui.add_sized(egui::vec2(RANGE_WIDTH, ui.spacing().interact_size.y), egui::Label::new(RichText::new(text).color(color)).truncate());
                });
            });
        });
    event
}


#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;

    use super::*;


    fn at(line: usize, column: usize) -> Result<Option<Position>, GoToLineError> {
        Ok(Some(Position { line, column }))
    }

    fn bar_harness(text: &str) -> egui_kittest::Harness<'static, (GoToLineState, Option<GoToLineEvent>)> {
        let state = GoToLineState { open: true, text: text.to_string(), focus: true };
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(500.0, 100.0)).build_ui_state(
            |ui, (state, event): &mut (GoToLineState, Option<GoToLineEvent>)| {
                let shown = show_bar(ui.ctx(), 0, egui::pos2(480.0, 10.0), state, 40);
                *event = shown.or(event.take());
            },
            (state, None),
        );
        harness.run();
        harness
    }

    #[test]
    fn a_line_and_an_optional_column_count_from_one() {
        assert_eq!(parse("1", 10), at(0, 0));
        assert_eq!(parse(" 7 ", 10), at(6, 0));
        assert_eq!(parse("7:3", 10), at(6, 2));
        assert_eq!(parse("7, 3", 10), at(6, 2));
        assert_eq!(parse("7:", 10), at(6, 0));
        assert_eq!(parse("7:0", 10), at(6, 0));
    }

    #[test]
    fn a_line_past_the_end_is_the_last_line() {
        assert_eq!(parse("11", 10), at(9, 0));
        assert_eq!(parse("99999999999999999999999999", 10), at(9, 0));
        assert_eq!(parse("5", 0), at(0, 0));
    }

    #[test]
    fn blank_text_names_nothing_and_other_text_is_refused() {
        assert_eq!(parse("", 10), Ok(None));
        assert_eq!(parse("  ", 10), Ok(None));
        assert_eq!(parse("0", 10), Err(GoToLineError::Zero));
        for text in ["x", "-3", "+3", "3.5", "3:x", ":4", "3:4:5"] {
            assert_eq!(parse(text, 10), Err(GoToLineError::NotANumber), "{text}");
        }
    }

    #[test]
    fn the_bar_takes_the_keyboard_and_enter_goes_to_the_line() {
        let mut harness = bar_harness("12:4");
        assert!(harness.get_by_role(egui::Role::TextInput).is_focused());
        harness.get_by_label("1 - 40");
        harness.key_press(Key::Enter);
        harness.run();
        assert_eq!(harness.state().1, Some(GoToLineEvent::Go(Position { line: 11, column: 3 })));
    }

    #[test]
    fn enter_on_bad_text_stays_in_the_field_and_says_why() {
        let mut harness = bar_harness("abc");
        harness.get_by_label("Not a line number");
        harness.key_press(Key::Enter);
        harness.run();
        assert_eq!(harness.state().1, None);
        assert!(harness.get_by_role(egui::Role::TextInput).is_focused());
    }

    #[test]
    fn escape_closes_the_bar() {
        let mut harness = bar_harness("3");
        harness.key_press(Key::Escape);
        harness.run();
        assert_eq!(harness.state().1, Some(GoToLineEvent::Close));
    }
}
