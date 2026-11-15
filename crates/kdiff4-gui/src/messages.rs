use std::collections::VecDeque;
use std::time::{Duration, Instant};

use egui::{Align, Label, Layout, RichText, ScrollArea, Ui, vec2};

use crate::dialog_window::Resizable;
use crate::theme;

pub(crate) const DIALOG_WIDTH: f32 = 640.0;
pub(crate) const RESIZABLE: Resizable = Resizable { height: 400.0, min_size: vec2(360.0, 200.0) };
pub const CLEAR_LABEL: &str = "Clear";
pub const COPY_LABEL: &str = "Copy All";
pub const EMPTY_LABEL: &str = "No messages";
/// How many messages are kept; the oldest go first.
const LIMIT: usize = 500;
const AGE_WIDTH: f32 = 80.0;
const TAB_WIDTH: f32 = 130.0;


/// One line the status bar or an Open form showed: an error, or the outcome of something the user did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub at: Instant,
    /// The title of the tab it was shown in.
    pub tab: String,
    pub text: String,
    pub is_error: bool,
}


/// The messages of all tabs since the app started, oldest first, and how many errors the user has not seen in the Messages window.
#[derive(Debug, Clone, Default)]
pub struct Messages {
    entries: VecDeque<Message>,
    unseen_errors: usize,
}


impl Messages {
    pub fn push(&mut self, at: Instant, tab: &str, text: &str, is_error: bool) {
        if self.entries.len() >= LIMIT {
            self.entries.pop_front();
        }
        self.entries.push_back(Message { at, tab: tab.to_string(), text: text.to_string(), is_error });
        self.unseen_errors += usize::from(is_error);
    }

    pub fn entries(&self) -> impl DoubleEndedIterator<Item = &Message> {
        self.entries.iter()
    }

    pub fn unseen_errors(&self) -> usize {
        self.unseen_errors
    }

    pub fn mark_seen(&mut self) {
        self.unseen_errors = 0;
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.unseen_errors = 0;
    }

    /// Every message on a line of its own, oldest first, for the clipboard.
    pub fn text(&self, now: Instant) -> String {
        self.entries
            .iter()
            .map(|message| {
                let kind = if message.is_error { "error" } else { "info" };
                format!("[{}] {kind}: {}: {}\n", age(now.saturating_duration_since(message.at)), message.tab, message.text)
            })
            .collect()
    }
}


/// How long ago something happened, to the minute and then the hour.
pub fn age(elapsed: Duration) -> String {
    match elapsed.as_secs() / 60 {
        0 => "just now".to_string(),
        minutes if minutes < 60 => format!("{minutes} min ago"),
        minutes => format!("{} h ago", minutes / 60),
    }
}


/// The red text of the status bar for errors the user has not seen; `None` without any.
pub fn error_badge(unseen: usize) -> Option<String> {
    match unseen {
        0 => None,
        1 => Some("1 error".to_string()),
        count => Some(format!("{count} errors")),
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessagesEvent {
    Copy,
    Clear,
    Close,
}


/// The Messages window's content: the messages, newest first, each with its age and tab and its whole text, errors in the error color; under them Copy All, Clear and Close.
pub fn show(ui: &mut Ui, messages: &Messages, now: Instant) -> Option<MessagesEvent> {
    let mut event = None;
    // The buttons go in first, from the bottom, so the list gets whatever height the window leaves.
    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        ui.horizontal(|ui| {
            if ui.button("Close").clicked() {
                event = Some(MessagesEvent::Close);
            }
            let has_messages = messages.entries().next().is_some();
            if ui.add_enabled(has_messages, egui::Button::new(COPY_LABEL)).clicked() {
                event = Some(MessagesEvent::Copy);
            }
            if ui.add_enabled(has_messages, egui::Button::new(CLEAR_LABEL)).clicked() {
                event = Some(MessagesEvent::Clear);
            }
        });
        ui.add_space(4.0);
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let weak = theme::palette_of(ui).weak_text;
                if messages.entries.is_empty() {
                    ui.label(RichText::new(EMPTY_LABEL).color(weak));
                }
                for message in messages.entries().rev() {
                    ui.horizontal_top(|ui| {
                        let height = ui.spacing().interact_size.y;
                        ui.add_sized(vec2(AGE_WIDTH, height), Label::new(RichText::new(age(now.saturating_duration_since(message.at))).color(weak)).truncate());
                        ui.add_sized(vec2(TAB_WIDTH, height), Label::new(RichText::new(message.tab.as_str()).color(weak)).truncate());
                        let color = if message.is_error { ui.visuals().error_fg_color } else { ui.visuals().text_color() };
                        ui.add(Label::new(RichText::new(message.text.as_str()).color(color)).wrap());
                    });
                }
            });
        });
    });
    event
}


#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;

    use super::*;


    fn three(now: Instant) -> Messages {
        let mut messages = Messages::default();
        messages.push(now - Duration::from_secs(7300), "a.txt", "Saved a.txt", false);
        messages.push(now - Duration::from_secs(130), "b.txt", "Cannot save: disk full", true);
        messages.push(now, "b.txt", "Reloaded", false);
        messages
    }

    fn harness(messages: Messages) -> egui_kittest::Harness<'static, (Messages, Option<MessagesEvent>)> {
        let now = Instant::now();
        let mut harness = egui_kittest::Harness::builder().with_size(vec2(640.0, 300.0)).build_ui_state(
            move |ui, state: &mut (Messages, Option<MessagesEvent>)| {
                let event = show(ui, &state.0, now);
                state.1 = event.or(state.1);
            },
            (messages, None),
        );
        harness.run_steps(3);
        harness
    }

    #[test]
    fn ages_round_down_to_minutes_and_hours() {
        assert_eq!(age(Duration::from_secs(59)), "just now");
        assert_eq!(age(Duration::from_secs(60)), "1 min ago");
        assert_eq!(age(Duration::from_secs(3599)), "59 min ago");
        assert_eq!(age(Duration::from_secs(7300)), "2 h ago");
    }

    #[test]
    fn errors_are_counted_until_they_are_seen() {
        let mut messages = three(Instant::now());
        assert_eq!(messages.unseen_errors(), 1);
        assert_eq!(error_badge(messages.unseen_errors()).as_deref(), Some("1 error"));
        messages.push(Instant::now(), "c.txt", "Cannot reload: gone", true);
        assert_eq!(error_badge(messages.unseen_errors()).as_deref(), Some("2 errors"));
        messages.mark_seen();
        assert_eq!(error_badge(messages.unseen_errors()), None);
        assert_eq!(messages.entries().count(), 4, "seen messages stay in the list");
        messages.push(Instant::now(), "c.txt", "Cannot reload: gone", true);
        messages.clear();
        assert_eq!((messages.entries().count(), messages.unseen_errors()), (0, 0));
    }

    #[test]
    fn only_the_newest_messages_are_kept() {
        let mut messages = Messages::default();
        let now = Instant::now();
        for index in 0..LIMIT + 2 {
            messages.push(now, "tab", &index.to_string(), false);
        }
        assert_eq!(messages.entries().count(), LIMIT);
        assert_eq!(messages.entries().next().map(|message| message.text.as_str()), Some("2"));
    }

    #[test]
    fn the_text_for_the_clipboard_lists_the_messages_oldest_first() {
        let now = Instant::now();
        assert_eq!(three(now).text(now), "[2 h ago] info: a.txt: Saved a.txt\n[2 min ago] error: b.txt: Cannot save: disk full\n[just now] info: b.txt: Reloaded\n");
    }

    #[test]
    fn the_window_lists_the_newest_message_first() {
        let harness = harness(three(Instant::now()));
        let top = |label: &str| harness.get_by_label(label).rect().top();
        assert!(top("Reloaded") < top("Cannot save: disk full"));
        assert!(top("Cannot save: disk full") < top("Saved a.txt"));
        harness.get_by_label("2 h ago");
        assert_eq!(harness.get_all_by_label("b.txt").count(), 2);
    }

    #[test]
    fn the_buttons_copy_clear_and_close() {
        for (label, event) in [(COPY_LABEL, MessagesEvent::Copy), (CLEAR_LABEL, MessagesEvent::Clear), ("Close", MessagesEvent::Close)] {
            let mut harness = harness(three(Instant::now()));
            harness.get_by_label(label).click();
            harness.run_steps(2);
            assert_eq!(harness.state().1, Some(event), "{label}");
        }
    }

    #[test]
    fn an_empty_list_says_so_and_has_nothing_to_copy_or_clear() {
        let mut harness = harness(Messages::default());
        harness.get_by_label(EMPTY_LABEL);
        harness.get_by_label(COPY_LABEL).click();
        harness.get_by_label(CLEAR_LABEL).click();
        harness.run_steps(2);
        assert_eq!(harness.state().1, None);
    }
}
