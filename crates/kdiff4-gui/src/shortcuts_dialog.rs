use egui::{Context, Grid, RichText, ScrollArea, TextEdit, Ui};

use crate::theme;
use crate::ui_commands::{self, PANE_KEYS, PANE_KEYS_GROUP};

pub(crate) const DIALOG_WIDTH: f32 = 520.0;
const LIST_HEIGHT: f32 = 440.0;
const LABEL_WIDTH: f32 = 290.0;


/// The Keyboard Shortcuts window while it is open.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShortcutsDialog {
    filter: String,
    focused: bool,
}


/// One line of the window: what a key does, under the group it belongs to.
#[derive(Debug, Clone, PartialEq)]
pub struct ShortcutRow {
    pub group: &'static str,
    pub label: String,
    pub shortcut: String,
}


/// Every UI command that has a shortcut, in the order of the menus, then the keys of the panes.
pub fn rows(ctx: &Context, has_start_menu: bool) -> Vec<ShortcutRow> {
    let commands = ui_commands::all(has_start_menu)
        .into_iter()
        .map(|command| command.info())
        .filter(|info| !info.shortcuts.is_empty())
        .map(|info| ShortcutRow { group: info.group.name(), label: info.label, shortcut: ui_commands::shortcut_text(ctx, &info.shortcuts) });
    let pane_keys = PANE_KEYS
        .iter()
        .map(|(label, shortcuts)| ShortcutRow { group: PANE_KEYS_GROUP, label: label.to_string(), shortcut: ui_commands::shortcut_text(ctx, shortcuts) });
    commands.chain(pane_keys).collect()
}


/// The rows whose group, label or shortcut holds `filter`, whatever its case; all of them for an empty filter.
pub fn filtered<'a>(rows: &'a [ShortcutRow], filter: &str) -> Vec<&'a ShortcutRow> {
    let filter = filter.trim().to_lowercase();
    rows.iter()
        .filter(|row| filter.is_empty() || [row.group, row.label.as_str(), row.shortcut.as_str()].iter().any(|text| text.to_lowercase().contains(&filter)))
        .collect()
}


/// The window's content: a filter field, the rows under their groups, and Close. Returns whether Close was clicked.
pub fn show(ui: &mut Ui, dialog: &mut ShortcutsDialog, rows: &[ShortcutRow]) -> bool {
    ui.set_width(DIALOG_WIDTH);
    let field = ui.add(TextEdit::singleline(&mut dialog.filter).hint_text("Filter by command or key").desired_width(f32::INFINITY));
    if !dialog.focused {
        dialog.focused = true;
        field.request_focus();
    }
    ui.add_space(4.0);
    let shown = filtered(rows, &dialog.filter);
    ScrollArea::vertical().max_height(LIST_HEIGHT).auto_shrink([false, false]).show(ui, |ui| {
        if shown.is_empty() {
            ui.label(RichText::new("No matching shortcuts").color(theme::palette_of(ui).weak_text));
        }
        let mut start = 0;
        while start < shown.len() {
            let group = shown[start].group;
            let end = shown[start..].iter().position(|row| row.group != group).map_or(shown.len(), |length| start + length);
            ui.label(RichText::new(group).strong());
            Grid::new(("shortcuts_group", group)).num_columns(2).min_col_width(LABEL_WIDTH).spacing([12.0, 4.0]).show(ui, |ui| {
                for row in &shown[start..end] {
                    ui.label(row.label.as_str());
                    ui.label(RichText::new(row.shortcut.as_str()).color(theme::palette_of(ui).weak_text));
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            start = end;
        }
    });
    ui.add_space(8.0);
    ui.button("Close").clicked()
}


#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::ui_commands::UiCommand;


    fn all_rows() -> Vec<ShortcutRow> {
        rows(&Context::default(), true)
    }

    fn harness() -> egui_kittest::Harness<'static, (ShortcutsDialog, bool)> {
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(700.0, 700.0)).build_ui_state(
            |ui, state: &mut (ShortcutsDialog, bool)| {
                let rows = rows(ui.ctx(), true);
                state.1 |= show(ui, &mut state.0, &rows);
            },
            (ShortcutsDialog::default(), false),
        );
        harness.run_steps(3);
        harness
    }


    #[test]
    fn lists_every_command_with_a_shortcut_and_the_pane_keys() {
        let rows = all_rows();
        let with_shortcut = ui_commands::all(true)
            .into_iter()
            .filter(|command| !command.info().shortcuts.is_empty())
            .count();
        assert_eq!(rows.len(), with_shortcut + PANE_KEYS.len());
        assert!(rows.iter().all(|row| !row.shortcut.is_empty()));
        assert!(rows.iter().all(|row| row.label != UiCommand::About.info().label), "a command without a shortcut is left out");
        assert_eq!(rows.last().map(|row| row.group), Some(PANE_KEYS_GROUP));
    }

    #[test]
    fn the_filter_matches_labels_groups_and_keys_in_any_case() {
        let rows = all_rows();
        let labels = |filter: &str| -> Vec<String> {
            filtered(&rows, filter)
                .into_iter()
                .map(|row| row.label.clone())
                .collect()
        };
        assert_eq!(filtered(&rows, " ").len(), rows.len());
        assert_eq!(labels("MINIMAP"), ["Minimap"]);
        assert!(labels("tabs").contains(&"Next Tab".to_string()));
        assert!(labels("f5").contains(&"Reload".to_string()));
        assert!(labels("no such key").is_empty());
    }

    #[test]
    fn typing_filters_the_rows_and_close_reports_it() {
        let mut harness = harness();
        harness.get_by_label("Reload");
        harness.event(egui::Event::Text("minimap".to_string()));
        harness.run_steps(2);
        assert!(harness.query_by_label("Reload").is_none());
        harness.get_by_label("Minimap");
        assert!(!harness.state().1);
        harness.get_by_label("Close").click();
        harness.run_steps(2);
        assert!(harness.state().1);
    }
}
