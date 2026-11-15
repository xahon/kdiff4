use egui::{Context, Key, KeyboardShortcut, Modifiers};
use kdiff4_protocol::{Document, Navigation};

pub const OPEN_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
pub const OPEN_TAB_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::T);
pub const RELOAD_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F5);
pub const SAVE_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
pub const SAVE_AS_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
pub const UNDO_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
pub const REDO_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
pub const MARK_LINES_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Space);
pub const MARK_FUNCTION_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Space);
pub const APPLY_ALIGNMENTS_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
pub const CLEAR_ALIGNMENTS_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Y);
pub const HEURISTICS_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::J);
pub const HEURISTICS_INSIDE_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::ALT), Key::J);
pub const EXIT_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Q);
pub const CLOSE_TAB_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::W);
pub const REOPEN_TAB_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::T);
pub const LEFT_TAB_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::ALT, Key::ArrowLeft);
pub const RIGHT_TAB_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::ALT, Key::ArrowRight);
pub const MOVE_TAB_LEFT_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::ALT.plus(Modifiers::SHIFT), Key::ArrowLeft);
pub const MOVE_TAB_RIGHT_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::ALT.plus(Modifiers::SHIFT), Key::ArrowRight);
pub const NEXT_TAB_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::Tab);
pub const PREVIOUS_TAB_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::Tab);
pub const SETTINGS_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma);
pub const FIND_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::F);
pub const FIND_NEXT_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F3);
pub const FIND_PREVIOUS_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::SHIFT, Key::F3);
pub const MINIMAP_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::M);
pub const IGNORE_WHITESPACE_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::I);
pub const INDEPENDENT_SCROLL_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::U);
/// egui can't tell the numeric keypad's minus and plus from the main ones, so both fold and unfold.
pub const FOLD_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Minus);
pub const UNFOLD_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Plus);
pub const FOLD_ALL_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Minus);
pub const UNFOLD_ALL_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Plus);
pub const KEYBOARD_SHORTCUTS_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F1);
pub const COMMAND_PALETTE_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::A);
/// Toggle A, B or C in the region with the caret (kdiff3: Ctrl+1, Ctrl+2, Ctrl+3).
pub const CHOOSE_SHORTCUTS: [(Document, KeyboardShortcut); 3] = [
    (Document::A, KeyboardShortcut::new(Modifiers::COMMAND, Key::Num1)),
    (Document::B, KeyboardShortcut::new(Modifiers::COMMAND, Key::Num2)),
    (Document::C, KeyboardShortcut::new(Modifiers::COMMAND, Key::Num3)),
];
pub const NAVIGATION_SHORTCUTS: [(Navigation, &str, KeyboardShortcut, KeyboardShortcut); 4] = [
    (
        Navigation::PreviousDifference,
        "Previous Difference",
        KeyboardShortcut::new(Modifiers::COMMAND, Key::ArrowUp),
        KeyboardShortcut::new(Modifiers::ALT, Key::ArrowUp),
    ),
    (
        Navigation::NextDifference,
        "Next Difference",
        KeyboardShortcut::new(Modifiers::COMMAND, Key::ArrowDown),
        KeyboardShortcut::new(Modifiers::ALT, Key::ArrowDown),
    ),
    (
        Navigation::PreviousConflict,
        "Previous Conflict",
        KeyboardShortcut::new(Modifiers::COMMAND, Key::PageUp),
        KeyboardShortcut::new(Modifiers::ALT, Key::PageUp),
    ),
    (
        Navigation::NextConflict,
        "Next Conflict",
        KeyboardShortcut::new(Modifiers::COMMAND, Key::PageDown),
        KeyboardShortcut::new(Modifiers::ALT, Key::PageDown),
    ),
];
pub const VOID_TOGGLE_LABEL: &str = "Void Space";
pub const VOID_TOGGLE_HINT: &str = "Toggle Void Space visibility in Output Pane";
pub const MINIMAP_TOGGLE_LABEL: &str = "Minimap";
pub const MINIMAP_TOGGLE_HINT: &str = "Toggle the Minimap: the whole document as a column right of the panes, to scroll by clicking or dragging";
pub const OVERVIEW_TOGGLE_LABEL: &str = "Overview Strip";
pub const OVERVIEW_TOGGLE_HINT: &str = "Toggle the Overview Strip: the current line of every document, with its character diff";
pub const IGNORE_WHITESPACE_TOGGLE_LABEL: &str = "Ignore Whitespace";
pub const IGNORE_WHITESPACE_TOGGLE_HINT: &str = "Toggle ignoring whitespace and line breaks in the character diff, so reflowed or reindented text shows only its real changes";
pub const INDEPENDENT_SCROLL_TOGGLE_LABEL: &str = "Independent Scroll";
pub const INDEPENDENT_SCROLL_TOGGLE_HINT: &str = "Toggle independent scrolling: every pane scrolls on its own. Turned off, the panes line up again with the one last clicked or scrolled";
pub const ADD_TO_START_MENU_LABEL: &str = "Add to Start Menu";
pub const ADD_TO_START_MENU_HINT: &str = "Create a Kdiff4 shortcut in your Start menu that starts this executable";
/// The keys the panes handle themselves, which no UI command stands for; listed in the Keyboard Shortcuts window.
pub const PANE_KEYS: [(&str, &[KeyboardShortcut]); 7] = [
    ("Copy", &[KeyboardShortcut::new(Modifiers::COMMAND, Key::C)]),
    ("Cut (in D)", &[KeyboardShortcut::new(Modifiers::COMMAND, Key::X)]),
    ("Paste (in D)", &[KeyboardShortcut::new(Modifiers::COMMAND, Key::V)]),
    ("Select All", &[KeyboardShortcut::new(Modifiers::COMMAND, Key::A)]),
    ("Previous / Next Word", &[KeyboardShortcut::new(Modifiers::COMMAND, Key::ArrowLeft), KeyboardShortcut::new(Modifiers::COMMAND, Key::ArrowRight)]),
    ("Start / End of Document", &[KeyboardShortcut::new(Modifiers::COMMAND, Key::Home), KeyboardShortcut::new(Modifiers::COMMAND, Key::End)]),
    ("Delete Previous / Next Word (in D)", &[KeyboardShortcut::new(Modifiers::COMMAND, Key::Backspace), KeyboardShortcut::new(Modifiers::COMMAND, Key::Delete)]),
];
pub const PANE_KEYS_GROUP: &str = "Panes";


/// What the Keyboard Shortcuts window and the command palette file a UI command under: its menu, or the document tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    File,
    Edit,
    Merge,
    View,
    Settings,
    Help,
    Tabs,
}


impl Group {
    /// The groups the menu bar has a menu for, in its order.
    pub const MENUS: [Group; 6] = [Group::File, Group::Edit, Group::Merge, Group::View, Group::Settings, Group::Help];

    pub fn name(self) -> &'static str {
        match self {
            Group::File => "File",
            Group::Edit => "Edit",
            Group::Merge => "Merge",
            Group::View => "View",
            Group::Settings => "Settings",
            Group::Help => "Help",
            Group::Tabs => "Tabs",
        }
    }
}


/// Something the user can invoke from a menu, a shortcut or the command palette. Not a protocol command: running one may send none, one or several.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiCommand {
    Open,
    Reload,
    Save,
    SaveAs,
    CloseTab,
    ReopenTab,
    Exit,
    Undo,
    Redo,
    Find,
    FindNext,
    FindPrevious,
    ChooseHere(Document),
    ChooseEverywhere(Document),
    ChooseForUnresolved(Document),
    Navigate(Navigation),
    MarkLines,
    MarkFunction,
    MarkAnchor,
    ApplyAlignments,
    Heuristics,
    HeuristicsInside,
    ClearAlignments,
    ToggleVoidSpace,
    ToggleMinimap,
    ToggleOverviewStrip,
    ToggleIndependentScroll,
    Fold,
    Unfold,
    FoldAll,
    UnfoldAll,
    ToggleIgnoreWhitespace,
    Settings,
    AddToStartMenu,
    About,
    KeyboardShortcuts,
    CommandPalette,
    CheckForUpdates,
    ToggleAutomaticUpdates,
    PreviousTab,
    NextTab,
    MoveTabLeft,
    MoveTabRight,
}


/// What every place that offers a UI command shows of it.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandInfo {
    pub group: Group,
    pub label: String,
    /// Every shortcut that runs the command, the main one first.
    pub shortcuts: Vec<KeyboardShortcut>,
    pub hint: Option<&'static str>,
    /// Whether the command switches something on and off, so a menu shows it as a checkbox.
    pub toggle: bool,
}


impl CommandInfo {
    fn new(group: Group, label: impl Into<String>, shortcuts: &[KeyboardShortcut]) -> Self {
        Self { group, label: label.into(), shortcuts: shortcuts.to_vec(), hint: None, toggle: false }
    }

    fn hint(mut self, hint: &'static str) -> Self {
        self.hint = Some(hint);
        self
    }

    fn toggle(mut self) -> Self {
        self.toggle = true;
        self
    }
}


impl UiCommand {
    pub fn info(self) -> CommandInfo {
        match self {
            UiCommand::Open => CommandInfo::new(Group::File, "Open...", &[OPEN_SHORTCUT, OPEN_TAB_SHORTCUT]),
            UiCommand::Reload => CommandInfo::new(Group::File, "Reload", &[RELOAD_SHORTCUT]),
            UiCommand::Save => CommandInfo::new(Group::File, "Save", &[SAVE_SHORTCUT]),
            UiCommand::SaveAs => CommandInfo::new(Group::File, "Save As...", &[SAVE_AS_SHORTCUT]),
            UiCommand::CloseTab => CommandInfo::new(Group::File, "Close Tab", &[CLOSE_TAB_SHORTCUT]),
            UiCommand::ReopenTab => CommandInfo::new(Group::File, "Reopen Closed Tab", &[REOPEN_TAB_SHORTCUT]),
            UiCommand::Exit => CommandInfo::new(Group::File, "Exit", &[EXIT_SHORTCUT]),
            UiCommand::Undo => CommandInfo::new(Group::Edit, "Undo", &[UNDO_SHORTCUT]),
            UiCommand::Redo => CommandInfo::new(Group::Edit, "Redo", &[REDO_SHORTCUT]),
            UiCommand::Find => CommandInfo::new(Group::Edit, "Find...", &[FIND_SHORTCUT]),
            UiCommand::FindNext => CommandInfo::new(Group::Edit, "Find Next", &[FIND_NEXT_SHORTCUT]),
            UiCommand::FindPrevious => CommandInfo::new(Group::Edit, "Find Previous", &[FIND_PREVIOUS_SHORTCUT]),
            UiCommand::ChooseHere(document) => CommandInfo::new(Group::Merge, format!("Choose {} Here", document.letter()), &choose_shortcut(document, false)),
            UiCommand::ChooseEverywhere(document) => CommandInfo::new(Group::Merge, format!("Choose {} Everywhere", document.letter()), &choose_shortcut(document, true)),
            UiCommand::ChooseForUnresolved(document) => CommandInfo::new(Group::Merge, format!("Choose {} for All Unresolved", document.letter()), &[]),
            UiCommand::Navigate(to) => navigation_info(to),
            UiCommand::MarkLines => CommandInfo::new(Group::Merge, "Mark Selected Lines for Alignment", &[MARK_LINES_SHORTCUT]),
            UiCommand::MarkFunction => CommandInfo::new(Group::Merge, "Mark Function for Alignment", &[MARK_FUNCTION_SHORTCUT]),
            UiCommand::MarkAnchor => CommandInfo::new(Group::Merge, "Mark Alignment Anchor Before Line", &[]),
            UiCommand::ApplyAlignments => CommandInfo::new(Group::Merge, "Apply Alignments", &[APPLY_ALIGNMENTS_SHORTCUT]),
            UiCommand::Heuristics => CommandInfo::new(Group::Merge, "Heuristics Alignment…", &[HEURISTICS_SHORTCUT]).hint("Find functions, methods and types in the documents and align them, also where a document moved them"),
            UiCommand::HeuristicsInside => CommandInfo::new(Group::Merge, "Heuristics Alignment Inside…", &[HEURISTICS_INSIDE_SHORTCUT]).hint("Find and align the entities inside the applied alignment around the caret"),
            UiCommand::ClearAlignments => CommandInfo::new(Group::Merge, "Clear All Alignments", &[CLEAR_ALIGNMENTS_SHORTCUT]),
            UiCommand::ToggleVoidSpace => CommandInfo::new(Group::Merge, VOID_TOGGLE_LABEL, &[]).hint(VOID_TOGGLE_HINT).toggle(),
            UiCommand::ToggleMinimap => CommandInfo::new(Group::View, MINIMAP_TOGGLE_LABEL, &[MINIMAP_SHORTCUT]).hint(MINIMAP_TOGGLE_HINT).toggle(),
            UiCommand::ToggleOverviewStrip => CommandInfo::new(Group::View, OVERVIEW_TOGGLE_LABEL, &[]).hint(OVERVIEW_TOGGLE_HINT).toggle(),
            UiCommand::ToggleIndependentScroll => CommandInfo::new(Group::View, INDEPENDENT_SCROLL_TOGGLE_LABEL, &[INDEPENDENT_SCROLL_SHORTCUT]).hint(INDEPENDENT_SCROLL_TOGGLE_HINT).toggle(),
            UiCommand::Fold => CommandInfo::new(Group::View, "Fold", &[FOLD_SHORTCUT]).hint("Fold the innermost function, type or block around the caret"),
            UiCommand::Unfold => CommandInfo::new(Group::View, "Unfold", &[UNFOLD_SHORTCUT]).hint("Unfold the fold at the caret"),
            UiCommand::FoldAll => CommandInfo::new(Group::View, "Fold All", &[FOLD_ALL_SHORTCUT]).hint("Fold the outermost functions, types and blocks of the pane with the caret"),
            UiCommand::UnfoldAll => CommandInfo::new(Group::View, "Unfold All", &[UNFOLD_ALL_SHORTCUT]).hint("Unfold everything in the pane with the caret"),
            UiCommand::ToggleIgnoreWhitespace => CommandInfo::new(Group::View, IGNORE_WHITESPACE_TOGGLE_LABEL, &[IGNORE_WHITESPACE_SHORTCUT]).hint(IGNORE_WHITESPACE_TOGGLE_HINT).toggle(),
            UiCommand::Settings => CommandInfo::new(Group::Settings, "Configure Kdiff4...", &[SETTINGS_SHORTCUT]),
            UiCommand::AddToStartMenu => CommandInfo::new(Group::Settings, ADD_TO_START_MENU_LABEL, &[]).hint(ADD_TO_START_MENU_HINT),
            UiCommand::About => CommandInfo::new(Group::Help, "About Kdiff4...", &[]),
            UiCommand::KeyboardShortcuts => CommandInfo::new(Group::Help, "Keyboard Shortcuts...", &[KEYBOARD_SHORTCUTS_SHORTCUT]),
            UiCommand::CommandPalette => CommandInfo::new(Group::Help, "Command Palette...", &[COMMAND_PALETTE_SHORTCUT]),
            UiCommand::CheckForUpdates => CommandInfo::new(Group::Help, "Check for Updates...", &[]),
            UiCommand::ToggleAutomaticUpdates => CommandInfo::new(Group::Help, "Check for Updates Automatically", &[]).toggle(),
            UiCommand::PreviousTab => CommandInfo::new(Group::Tabs, "Previous Tab", &[LEFT_TAB_SHORTCUT, PREVIOUS_TAB_SHORTCUT]),
            UiCommand::NextTab => CommandInfo::new(Group::Tabs, "Next Tab", &[RIGHT_TAB_SHORTCUT, NEXT_TAB_SHORTCUT]),
            UiCommand::MoveTabLeft => CommandInfo::new(Group::Tabs, "Move Tab Left", &[MOVE_TAB_LEFT_SHORTCUT]),
            UiCommand::MoveTabRight => CommandInfo::new(Group::Tabs, "Move Tab Right", &[MOVE_TAB_RIGHT_SHORTCUT]),
        }
    }
}


fn choose_shortcut(document: Document, everywhere: bool) -> Vec<KeyboardShortcut> {
    CHOOSE_SHORTCUTS
        .iter()
        .filter(|(chosen, _)| *chosen == document)
        .map(|(_, shortcut)| if everywhere { everywhere_shortcut(shortcut) } else { shortcut.clone() })
        .collect()
}


/// The shortcut that chooses a document everywhere: the one that chooses it here, with Shift.
pub fn everywhere_shortcut(here: &KeyboardShortcut) -> KeyboardShortcut {
    KeyboardShortcut::new(here.modifiers.plus(Modifiers::SHIFT), here.logical_key)
}


fn navigation_info(to: Navigation) -> CommandInfo {
    let found = NAVIGATION_SHORTCUTS
        .iter()
        .find(|(navigation, _, _, _)| *navigation == to);
    match found {
        Some((_, label, shortcut, alternate)) => CommandInfo::new(Group::Merge, *label, &[shortcut.clone(), alternate.clone()]),
        None => CommandInfo::new(Group::Merge, "Navigate", &[]),
    }
}


/// The items of a menu, top to bottom; `None` is a separator. Choose C Everywhere is left out while C can't be chosen, and Add to Start Menu where there is no Start menu.
pub fn menu_items(menu: Group, c_choosable: bool, has_start_menu: bool) -> Vec<Option<UiCommand>> {
    let documents = [Document::A, Document::B, Document::C];
    match menu {
        Group::File => vec![
            Some(UiCommand::Open),
            Some(UiCommand::Reload),
            Some(UiCommand::Save),
            Some(UiCommand::SaveAs),
            None,
            Some(UiCommand::CloseTab),
            Some(UiCommand::ReopenTab),
            Some(UiCommand::Exit),
        ],
        Group::Edit => vec![Some(UiCommand::Undo), Some(UiCommand::Redo), None, Some(UiCommand::Find), Some(UiCommand::FindNext), Some(UiCommand::FindPrevious)],
        Group::Merge => {
            let mut items: Vec<Option<UiCommand>> = documents.iter().map(|document| Some(UiCommand::ChooseHere(document.clone()))).collect();
            items.push(None);
            items.extend(
                documents
                    .iter()
                    .filter(|document| c_choosable || **document != Document::C)
                    .map(|document| Some(UiCommand::ChooseEverywhere(document.clone()))),
            );
            items.push(None);
            items.extend(documents.iter().map(|document| Some(UiCommand::ChooseForUnresolved(document.clone()))));
            items.push(None);
            items.extend(NAVIGATION_SHORTCUTS.iter().map(|(to, _, _, _)| Some(UiCommand::Navigate(to.clone()))));
            items.push(None);
            items.extend([
                Some(UiCommand::MarkLines),
                Some(UiCommand::MarkFunction),
                Some(UiCommand::MarkAnchor),
                Some(UiCommand::ApplyAlignments),
                Some(UiCommand::Heuristics),
                Some(UiCommand::HeuristicsInside),
                Some(UiCommand::ClearAlignments),
                None,
                Some(UiCommand::ToggleVoidSpace),
            ]);
            items
        }
        Group::View => vec![
            Some(UiCommand::ToggleMinimap),
            Some(UiCommand::ToggleIndependentScroll),
            None,
            Some(UiCommand::Fold),
            Some(UiCommand::Unfold),
            Some(UiCommand::FoldAll),
            Some(UiCommand::UnfoldAll),
            None,
            Some(UiCommand::ToggleIgnoreWhitespace),
        ],
        Group::Settings if has_start_menu => vec![Some(UiCommand::Settings), None, Some(UiCommand::AddToStartMenu)],
        Group::Settings => vec![Some(UiCommand::Settings)],
        Group::Help => vec![
            Some(UiCommand::About),
            None,
            Some(UiCommand::KeyboardShortcuts),
            Some(UiCommand::CommandPalette),
            None,
            Some(UiCommand::CheckForUpdates),
            Some(UiCommand::ToggleAutomaticUpdates),
        ],
        Group::Tabs => Vec::new(),
    }
}


/// Every UI command, grouped as the Keyboard Shortcuts window and the command palette list them: the menus top to bottom, then what only the toolbar or the tabs offer.
pub fn all(has_start_menu: bool) -> Vec<UiCommand> {
    let mut commands: Vec<UiCommand> = Vec::new();
    for menu in Group::MENUS {
        commands.extend(menu_items(menu, true, has_start_menu).into_iter().flatten());
        if menu == Group::View {
            commands.push(UiCommand::ToggleOverviewStrip);
        }
    }
    commands.extend([UiCommand::PreviousTab, UiCommand::NextTab, UiCommand::MoveTabLeft, UiCommand::MoveTabRight]);
    commands
}


/// The shortcuts as a menu shows them, such as `Ctrl+O / Ctrl+T`; empty without any.
pub fn shortcut_text(ctx: &Context, shortcuts: &[KeyboardShortcut]) -> String {
    shortcuts
        .iter()
        .map(|shortcut| ctx.format_shortcut(shortcut))
        .collect::<Vec<_>>()
        .join(" / ")
}


/// A toggle's hint followed by its shortcut, for the tooltips of the View menu and the toolbar.
pub fn hint_with_shortcut(hint: &str, shortcut: &str) -> String {
    format!("{hint} ({shortcut})")
}


#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn every_menu_item_is_a_command_of_that_menu() {
        for menu in Group::MENUS {
            for command in menu_items(menu, true, true).into_iter().flatten() {
                assert_eq!(command.info().group, menu, "{command:?}");
            }
        }
    }

    #[test]
    fn all_lists_every_command_once_with_its_own_label() {
        let commands = all(true);
        for (index, command) in commands.iter().enumerate() {
            assert!(!commands[..index].contains(command), "{command:?} is listed twice");
            let label = command.info().label;
            assert!(!label.is_empty());
            assert!(commands[..index].iter().all(|other| other.info().label != label), "{label} names two commands");
        }
        for command in [UiCommand::ToggleOverviewStrip, UiCommand::NextTab, UiCommand::ChooseEverywhere(Document::C), UiCommand::AddToStartMenu, UiCommand::CommandPalette] {
            assert!(commands.contains(&command), "{command:?}");
        }
        assert!(!all(false).contains(&UiCommand::AddToStartMenu));
    }

    #[test]
    fn no_shortcut_runs_two_commands() {
        let mut seen: Vec<(KeyboardShortcut, UiCommand)> = Vec::new();
        for command in all(true) {
            for shortcut in command.info().shortcuts {
                let taken = seen.iter().find(|(other, _)| *other == shortcut);
                assert!(taken.is_none(), "{command:?} and {:?} share a shortcut", taken.map(|(_, other)| other));
                seen.push((shortcut, command));
            }
        }
    }

    #[test]
    fn heuristics_and_the_minimap_left_the_keys_of_replace_and_go_to_line() {
        let taken: Vec<KeyboardShortcut> = all(true)
            .into_iter()
            .flat_map(|command| command.info().shortcuts)
            .collect();
        for key in [Key::H, Key::G] {
            assert!(!taken.contains(&KeyboardShortcut::new(Modifiers::COMMAND, key)), "{key:?}");
        }
        assert_eq!(UiCommand::Heuristics.info().shortcuts, [KeyboardShortcut::new(Modifiers::COMMAND, Key::J)]);
        assert_eq!(UiCommand::ToggleMinimap.info().shortcuts, [KeyboardShortcut::new(Modifiers::COMMAND, Key::M)]);
    }

    #[test]
    fn the_merge_menu_leaves_choose_c_everywhere_out_while_c_cannot_be_chosen() {
        let everywhere_c = Some(UiCommand::ChooseEverywhere(Document::C));
        assert!(menu_items(Group::Merge, true, true).contains(&everywhere_c));
        let without = menu_items(Group::Merge, false, true);
        assert!(!without.contains(&everywhere_c));
        assert!(without.contains(&Some(UiCommand::ChooseHere(Document::C))));
        assert!(without.contains(&Some(UiCommand::ChooseForUnresolved(Document::C))));
    }

    #[test]
    fn a_command_with_two_shortcuts_shows_both() {
        let ctx = Context::default();
        let text = shortcut_text(&ctx, &UiCommand::Open.info().shortcuts);
        assert_eq!(text.split(" / ").count(), 2, "{text}");
        assert!(shortcut_text(&ctx, &UiCommand::About.info().shortcuts).is_empty());
    }

    #[test]
    fn a_toggle_hint_ends_with_its_shortcut() {
        let ctx = Context::default();
        let hint = hint_with_shortcut(MINIMAP_TOGGLE_HINT, &ctx.format_shortcut(&MINIMAP_SHORTCUT));
        assert!(hint.starts_with(MINIMAP_TOGGLE_HINT));
        assert!(hint.ends_with("+M)"), "{hint}");
    }
}
