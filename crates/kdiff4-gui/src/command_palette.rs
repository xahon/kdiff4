use egui::{vec2, Align2, Area, Context, Frame, Id, Key, Modifiers, Order, Pos2, Response, Role, ScrollArea, Sense, TextEdit, TextStyle, Ui, WidgetInfo};

use crate::theme;
use crate::ui_commands::UiCommand;

const WIDTH: f32 = 520.0;
/// Rows the list shows before it scrolls.
const MAX_ROWS: f32 = 12.0;
const FIELD_ID: &str = "command_palette_field";


/// The command palette while it is open: what was typed and which match is selected.
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteState {
    pub query: String,
    selected: usize,
    /// Whether the field still has to take the keyboard.
    focus: bool,
    /// Whether the list has to scroll to the selected row.
    reveal: bool,
}


impl Default for PaletteState {
    fn default() -> Self {
        Self { query: String::new(), selected: 0, focus: true, reveal: false }
    }
}


/// One UI command as the palette lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteEntry {
    pub command: UiCommand,
    pub group: &'static str,
    pub label: String,
    pub shortcut: String,
    pub enabled: bool,
    /// Whether a toggle is on; `None` for a command that is not a toggle.
    pub checked: Option<bool>,
}


#[derive(Debug, Clone, PartialEq)]
pub enum PaletteEvent {
    Run(UiCommand),
    Close,
}


/// How well an entry matches the lowercase `query`, lower being better: the label starts with it, a word of the label does, the label holds it, the group with the label does, or its characters appear in the label in order.
fn rank(entry: &PaletteEntry, query: &str) -> Option<u8> {
    let label = entry.label.to_lowercase();
    if label.starts_with(query) {
        return Some(0);
    }
    if label.split(|character: char| !character.is_alphanumeric()).any(|word| word.starts_with(query)) {
        return Some(1);
    }
    if label.contains(query) {
        return Some(2);
    }
    if format!("{} {label}", entry.group.to_lowercase()).contains(query) {
        return Some(3);
    }
    let mut wanted = query.chars().filter(|character| !character.is_whitespace()).peekable();
    for character in label.chars() {
        if wanted.peek() == Some(&character) {
            wanted.next();
        }
    }
    wanted.peek().is_none().then_some(4)
}


/// The entries that match `query`, best first and otherwise in their order; all of them for an empty query.
pub fn matches(entries: &[PaletteEntry], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return (0..entries.len()).collect();
    }
    let mut found: Vec<(u8, usize)> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| rank(entry, &query).map(|rank| (rank, index)))
        .collect();
    found.sort();
    found.into_iter().map(|(_, index)| index).collect()
}


/// The palette, floating below `top_center`: a field and the matching commands. Up and Down move the selection, Enter or a click runs an enabled command, Escape or a click elsewhere closes.
pub fn show(ctx: &Context, top_center: Pos2, state: &mut PaletteState, entries: &[PaletteEntry]) -> Option<PaletteEvent> {
    let field_id = Id::unique(FIELD_ID);
    let (up, down, enter, escape) = ctx.input_mut(|input| {
        (
            input.consume_key(Modifiers::NONE, Key::ArrowUp),
            input.consume_key(Modifiers::NONE, Key::ArrowDown),
            input.consume_key(Modifiers::NONE, Key::Enter),
            input.consume_key(Modifiers::NONE, Key::Escape),
        )
    });
    let mut event = None;
    let area = Area::new(Id::unique("command_palette"))
        .order(Order::Foreground)
        .fixed_pos(top_center)
        .pivot(Align2::CENTER_TOP)
        .show(ctx, |ui| {
            Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(WIDTH);
                let field = ui.add(TextEdit::singleline(&mut state.query).id(field_id).hint_text("Type a command").desired_width(f32::INFINITY));
                if state.focus {
                    state.focus = false;
                    field.request_focus();
                }
                if field.changed() {
                    state.selected = 0;
                    state.reveal = true;
                }
                let found = matches(entries, &state.query);
                if !found.is_empty() {
                    state.selected = state.selected.min(found.len() - 1);
                    if down {
                        state.selected = (state.selected + 1) % found.len();
                    }
                    if up {
                        state.selected = (state.selected + found.len() - 1) % found.len();
                    }
                    state.reveal |= up || down;
                }
                ui.separator();
                let row_height = ui.spacing().interact_size.y;
                ScrollArea::vertical().max_height((row_height + ui.spacing().item_spacing.y) * MAX_ROWS).show(ui, |ui| {
                    if found.is_empty() {
                        ui.label(egui::RichText::new("No matching commands").color(theme::palette_of(ui).weak_text));
                    }
                    for (position, index) in found.iter().enumerate() {
                        let entry = &entries[*index];
                        let selected = position == state.selected;
                        let response = row(ui, entry, selected, row_height);
                        if selected && state.reveal {
                            response.scroll_to_me(None);
                        }
                        if response.clicked() && entry.enabled {
                            event = Some(PaletteEvent::Run(entry.command.clone()));
                        }
                    }
                });
                state.reveal = false;
                let chosen = found.get(state.selected).map(|index| &entries[*index]);
                if let Some(entry) = chosen.filter(|entry| enter && entry.enabled) {
                    event = Some(PaletteEvent::Run(entry.command.clone()));
                }
            });
        });
    if event.is_none() && (escape || area.response.clicked_elsewhere()) {
        event = Some(PaletteEvent::Close);
    }
    if event.is_some() {
        ctx.memory_mut(|memory| memory.surrender_focus(field_id));
    }
    event
}


/// One command: its group, its label (with whether a toggle is on) and, at the right, its shortcut. A disabled command is drawn weak.
fn row(ui: &mut Ui, entry: &PaletteEntry, selected: bool, height: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, entry.enabled, &entry.label));
    let visuals = ui.visuals();
    if selected {
        ui.painter().rect_filled(rect, visuals.widgets.hovered.corner_radius, visuals.selection.bg_fill);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, visuals.widgets.hovered.corner_radius, visuals.widgets.hovered.weak_bg_fill);
    }
    let weak = theme::palette_of(ui).weak_text;
    let color = if entry.enabled { visuals.text_color() } else { weak.clone() };
    let font = TextStyle::Button.resolve(ui.style());
    let padding = ui.spacing().button_padding.x;
    let label = match entry.checked {
        Some(true) => format!("{} (on)", entry.label),
        Some(false) => format!("{} (off)", entry.label),
        None => entry.label.clone(),
    };
    let group = ui.painter().text(rect.left_center() + vec2(padding, 0.0), Align2::LEFT_CENTER, format!("{}: ", entry.group), font.clone(), weak.clone());
    ui.painter().text(Pos2::new(group.right(), rect.center().y), Align2::LEFT_CENTER, label, font.clone(), color);
    ui.painter().text(rect.right_center() - vec2(padding, 0.0), Align2::RIGHT_CENTER, &entry.shortcut, font, weak);
    response
}


#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;

    use super::*;


    fn entry(command: UiCommand, enabled: bool) -> PaletteEntry {
        let info = command.info();
        PaletteEntry { command, group: info.group.name(), label: info.label, shortcut: String::new(), enabled, checked: info.toggle.then_some(false) }
    }

    fn entries() -> Vec<PaletteEntry> {
        vec![
            entry(UiCommand::Open, true),
            entry(UiCommand::Save, false),
            entry(UiCommand::SaveAs, true),
            entry(UiCommand::FindNext, true),
            entry(UiCommand::ToggleMinimap, true),
            entry(UiCommand::UnfoldAll, true),
        ]
    }

    fn found(query: &str) -> Vec<UiCommand> {
        let entries = entries();
        matches(&entries, query)
            .into_iter()
            .map(|index| entries[index].command.clone())
            .collect()
    }

    fn harness() -> egui_kittest::Harness<'static, (PaletteState, Vec<PaletteEvent>)> {
        let mut harness = egui_kittest::Harness::builder().with_size(vec2(800.0, 600.0)).build_ui_state(
            |ui, state: &mut (PaletteState, Vec<PaletteEvent>)| {
                let event = show(ui.ctx(), Pos2::new(400.0, 20.0), &mut state.0, &entries());
                state.1.extend(event);
            },
            (PaletteState::default(), Vec::new()),
        );
        harness.run_steps(3);
        harness
    }

    fn type_text(harness: &mut egui_kittest::Harness<'static, (PaletteState, Vec<PaletteEvent>)>, text: &str) {
        harness.event(egui::Event::Text(text.to_string()));
        harness.run_steps(2);
    }


    #[test]
    fn an_empty_query_lists_every_command_in_order() {
        assert_eq!(found("").len(), entries().len());
        assert_eq!(found("  ")[0], UiCommand::Open);
    }

    #[test]
    fn a_label_that_starts_with_the_query_comes_before_one_that_holds_it() {
        assert_eq!(found("fold"), [UiCommand::UnfoldAll]);
        assert_eq!(found("save"), [UiCommand::Save, UiCommand::SaveAs]);
        assert_eq!(found("next"), [UiCommand::FindNext]);
        assert_eq!(found("a"), [UiCommand::SaveAs, UiCommand::UnfoldAll, UiCommand::Save, UiCommand::ToggleMinimap]);
    }

    #[test]
    fn the_query_ignores_case_and_matches_the_group_and_scattered_characters() {
        assert_eq!(found("MINI"), [UiCommand::ToggleMinimap]);
        assert_eq!(found("view"), [UiCommand::ToggleMinimap, UiCommand::UnfoldAll]);
        assert_eq!(found("fnx"), [UiCommand::FindNext]);
        assert!(found("zzz").is_empty());
    }

    #[test]
    fn typing_filters_the_list_and_enter_runs_the_selected_command() {
        let mut harness = harness();
        harness.get_by_label("Open...");
        type_text(&mut harness, "mini");
        assert!(harness.query_by_label("Open...").is_none());
        harness.get_by_label("Minimap");
        harness.key_press(Key::Enter);
        harness.run_steps(2);
        assert_eq!(harness.state().1, [PaletteEvent::Run(UiCommand::ToggleMinimap)]);
    }

    #[test]
    fn down_and_up_move_the_selection_around_the_matches() {
        let mut harness = harness();
        type_text(&mut harness, "save");
        harness.key_press(Key::ArrowDown);
        harness.run_steps(2);
        harness.key_press(Key::Enter);
        harness.run_steps(2);
        assert_eq!(harness.state().1, [PaletteEvent::Run(UiCommand::SaveAs)]);
        harness.key_press(Key::ArrowDown);
        harness.run_steps(2);
        assert_eq!(harness.state().0.selected, 0, "Down wraps to the first match");
        harness.key_press(Key::ArrowUp);
        harness.run_steps(2);
        assert_eq!(harness.state().0.selected, 1, "Up wraps to the last match");
    }

    #[test]
    fn a_disabled_command_does_not_run() {
        let mut harness = harness();
        type_text(&mut harness, "save");
        harness.key_press(Key::Enter);
        harness.run_steps(2);
        assert!(harness.state().1.is_empty());
        harness.get_by_label("Save").click();
        harness.run_steps(2);
        assert!(harness.state().1.is_empty());
    }

    #[test]
    fn a_click_runs_a_command_and_escape_closes() {
        let mut harness = harness();
        harness.get_by_label("Find Next").click();
        harness.run_steps(2);
        assert_eq!(harness.state().1, [PaletteEvent::Run(UiCommand::FindNext)]);
        harness.key_press(Key::Escape);
        harness.run_steps(2);
        assert_eq!(harness.state().1.last(), Some(&PaletteEvent::Close));
    }
}
