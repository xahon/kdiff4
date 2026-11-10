use egui::Ui;

pub(crate) const DIALOG_WIDTH: f32 = 480.0;
pub(crate) const UPDATE_LABEL: &str = "Check for Updates...";
pub(crate) const REPOSITORY_URL: &str = "https://github.com/xahon/kdiff4";
const BUILD_FIELDS: &[(&str, &str)] = &[
    ("Version", env!("CARGO_PKG_VERSION")),
    ("Commit", env!("KDIFF4_BUILD_COMMIT")),
    ("Target", env!("KDIFF4_BUILD_TARGET")),
    ("Profile", env!("KDIFF4_BUILD_PROFILE")),
    ("Cargo features", env!("KDIFF4_BUILD_FEATURES")),
];


fn build_info() -> String {
    let mut text = String::from("Kdiff4\nFile comparison and merge tool\nLicense: GPL-2.0-or-later\n");
    for (label, value) in BUILD_FIELDS {
        text.push_str(&format!("{label}: {value}\n"));
    }
    text.push_str(&format!("Repository: {REPOSITORY_URL}\n"));
    text
}


pub(crate) fn show(ui: &mut Ui, checking: bool) -> bool {
    ui.set_width(DIALOG_WIDTH);
    ui.add(egui::Label::new(egui::RichText::new("Kdiff4").heading()).selectable(true));
    ui.add(egui::Label::new("File comparison and merge tool").selectable(true));
    ui.add(egui::Label::new("GPL-2.0-or-later").selectable(true));
    ui.add_space(8.0);
    egui::Grid::new("about_build_info").spacing([12.0, 4.0]).show(ui, |ui| {
        for (label, value) in BUILD_FIELDS {
            ui.add(egui::Label::new(*label).selectable(true));
            ui.add(egui::Label::new(egui::RichText::new(*value).monospace()).selectable(true));
            ui.end_row();
        }
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add(egui::Label::new("Repository").selectable(true));
        ui.hyperlink(REPOSITORY_URL);
    });
    ui.add_space(8.0);
    let mut check_for_updates = false;
    ui.horizontal(|ui| {
        let label = if checking { "Checking for Updates..." } else { UPDATE_LABEL };
        check_for_updates = ui.add_enabled(!checking, egui::Button::new(label)).clicked();
        if ui.button("Copy to Clipboard").clicked() {
            ui.ctx().copy_text(build_info());
        }
    });
    check_for_updates
}


#[cfg(test)]
mod tests {
    use egui_kittest::kittest::{NodeT, Queryable};

    use super::*;


    fn harness(checking: bool) -> egui_kittest::Harness<'static, (bool, Vec<egui::OutputCommand>)> {
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(600.0, 600.0)).build_ui_state(
            move |ui, state: &mut (bool, Vec<egui::OutputCommand>)| {
                state.0 |= show(ui, checking);
                state.1.extend(ui.ctx().output(|output| output.commands.clone()));
            },
            (false, Vec::new()),
        );
        harness.run_steps(3);
        harness
    }


    fn opened_urls(commands: &[egui::OutputCommand]) -> Vec<&str> {
        commands
            .iter()
            .filter_map(|command| match command {
                egui::OutputCommand::OpenUrl(url) => Some(url.url.as_str()),
                _ => None,
            })
            .collect()
    }


    #[test]
    fn shows_build_information_with_only_cargo_features() {
        let harness = harness(false);
        for value in [env!("CARGO_PKG_VERSION"), env!("KDIFF4_BUILD_COMMIT"), env!("KDIFF4_BUILD_TARGET"), env!("KDIFF4_BUILD_PROFILE"), env!("KDIFF4_BUILD_FEATURES")] {
            harness.get_by_label(value);
        }
        harness.get_by_label("Cargo features");
        assert!(harness.query_by_label("Features").is_none());
        assert!(harness.query_by_label_contains("2-way and 3-way").is_none());
    }


    #[test]
    fn update_button_requests_a_check() {
        let mut harness = harness(false);
        harness.get_by_label(UPDATE_LABEL).click();
        harness.run_steps(2);
        assert!(harness.state().0);
    }


    #[test]
    fn repository_is_a_full_url_link_in_its_own_row() {
        let harness = harness(false);
        let link = harness.get_by_label(REPOSITORY_URL);
        assert_eq!(link.accesskit_node().role(), egui::Role::Link);
        let prefix = harness.get_by_label("Repository").rect();
        assert!(prefix.right() < link.rect().left());
        assert_eq!(prefix.center().y, link.rect().center().y);
        for label in [UPDATE_LABEL, "Copy to Clipboard"] {
            assert!(link.rect().bottom() < harness.get_by_label(label).rect().top());
        }
        assert!(harness.query_by_label("GitHub").is_none());
        assert!(harness.query_by_label("Close").is_none());
        assert!(opened_urls(&harness.state().1).is_empty());
    }


    #[test]
    fn repository_link_opens_the_exact_url_on_pointer_release_across_its_text() {
        for checking in [false, true] {
            for fraction in [0.05, 0.5, 0.95] {
                let mut harness = harness(checking);
                let rect = harness.get_by_label(REPOSITORY_URL).rect();
                let pos = egui::pos2(rect.left() + rect.width() * fraction, rect.center().y);
                harness.event(egui::Event::PointerMoved(pos));
                harness.run_steps(1);
                for pressed in [true, false] {
                    harness.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
                    harness.run_steps(1);
                    if pressed {
                        assert!(opened_urls(&harness.state().1).is_empty());
                    }
                }
                harness.run_steps(2);
                assert_eq!(opened_urls(&harness.state().1), [REPOSITORY_URL]);
                assert!(!harness.state().0);
            }
        }
    }


    #[test]
    fn repository_link_opens_the_exact_url_from_the_keyboard() {
        let mut harness = harness(false);
        harness.get_by_label(REPOSITORY_URL).focus();
        harness.run_steps(1);
        assert!(harness.get_by_label(REPOSITORY_URL).is_focused());
        harness.key_press(egui::Key::Enter);
        harness.run_steps(2);
        assert_eq!(opened_urls(&harness.state().1), [REPOSITORY_URL]);
        assert!(!harness.state().0);
    }


    #[test]
    fn update_button_is_disabled_during_a_check() {
        let mut harness = harness(true);
        harness.get_by_label("Checking for Updates...").click();
        harness.run_steps(2);
        assert!(!harness.state().0);
    }


    #[test]
    fn copy_button_includes_build_information_with_only_cargo_features() {
        let mut harness = harness(false);
        harness.get_by_label("Copy to Clipboard").click();
        harness.run_steps(2);
        let text = harness.state().1.iter().find_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text),
            _ => None,
        }).expect("About test: no clipboard command");
        for (label, value) in BUILD_FIELDS {
            assert!(text.contains(&format!("{label}: {value}\n")));
        }
        assert!(text.contains("Repository: https://github.com/xahon/kdiff4\n"));
        assert!(!text.lines().any(|line| line == "Features:"));
        assert!(text.contains("License: GPL-2.0-or-later"));
        assert!(opened_urls(&harness.state().1).is_empty());
        harness.get_by_label("Kdiff4");
    }
}
