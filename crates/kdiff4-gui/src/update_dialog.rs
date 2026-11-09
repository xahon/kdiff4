use egui::Ui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use semver::Version;

use crate::update_check::ReleaseInfo;

pub(crate) const DIALOG_WIDTH: f32 = 480.0;
const NOTES_HEIGHT: f32 = 260.0;
const NO_NOTES_TEXT: &str = "The release has no notes.";
pub const INSTALL_LABEL: &str = "Update and Restart";
pub const OPEN_PAGE_LABEL: &str = "Open Release Page";
pub const REMIND_LABEL: &str = "Remind Me Later";
pub const SKIP_LABEL: &str = "Skip This Version";
pub const CANCEL_LABEL: &str = "Cancel";
pub const OK_LABEL: &str = "OK";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvailableAction {
    /// Download the new executable, replace the running one and start it.
    Install,
    OpenPage,
    RemindLater,
    SkipVersion,
}

/// What the user can be told before updating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallOffer {
    /// The release has a file for this platform.
    pub can_install: bool,
    /// Restarting drops edits that are not saved.
    pub has_unsaved_changes: bool,
}


/// The "Update Available" window: the versions, the release notes as Markdown and the choices.
pub fn show_available(ui: &mut Ui, release: &ReleaseInfo, current: &Version, offer: InstallOffer, cache: &mut CommonMarkCache) -> Option<AvailableAction> {
    let mut action = None;
    ui.set_width(DIALOG_WIDTH);
    ui.heading("Update Available");
    ui.add_space(4.0);
    ui.label(format!("Kdiff4 {} is available. You have {current}.", release.version));
    ui.add_space(6.0);
    ui.label("What's new:");
    egui::ScrollArea::vertical().max_height(NOTES_HEIGHT).auto_shrink([false, true]).show(ui, |ui| {
        let notes = if release.notes.trim().is_empty() { NO_NOTES_TEXT } else { release.notes.as_str() };
        CommonMarkViewer::new().show(ui, cache, notes);
    });
    ui.add_space(6.0);
    if !offer.can_install {
        ui.label("This release has no download for this platform. Get it from the release page.");
    } else if offer.has_unsaved_changes {
        ui.colored_label(ui.visuals().warn_fg_color, "Unsaved changes are lost: Kdiff4 restarts with the files it was started with.");
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if offer.can_install {
            if ui.button(INSTALL_LABEL).clicked() {
                action = Some(AvailableAction::Install);
            }
        } else if ui.button(OPEN_PAGE_LABEL).clicked() {
            action = Some(AvailableAction::OpenPage);
        }
        if ui.button(REMIND_LABEL).clicked() {
            action = Some(AvailableAction::RemindLater);
        }
        if ui.button(SKIP_LABEL).clicked() {
            action = Some(AvailableAction::SkipVersion);
        }
    });
    action
}


/// The window shown while the update downloads. Returns true when the user presses Cancel.
pub fn show_progress(ui: &mut Ui, version: &Version, fraction: Option<f32>, downloaded: u64, total: u64) -> bool {
    ui.set_width(DIALOG_WIDTH);
    ui.heading("Updating Kdiff4");
    ui.add_space(4.0);
    ui.label(format!("Downloading Kdiff4 {version}. Kdiff4 restarts when it is done."));
    ui.add_space(6.0);
    let bar = match fraction {
        Some(fraction) => egui::ProgressBar::new(fraction).text(format!("{} of {}", format_megabytes(downloaded), format_megabytes(total))),
        None => egui::ProgressBar::new(0.0).animate(true).text(format_megabytes(downloaded)),
    };
    ui.add(bar);
    ui.add_space(6.0);
    ui.button(CANCEL_LABEL).clicked()
}


/// A plain notice with an OK button, for "up to date" and for failures. Returns true when it is dismissed.
pub fn show_message(ui: &mut Ui, title: &str, text: &str) -> bool {
    ui.set_width(DIALOG_WIDTH);
    ui.heading(title);
    ui.add_space(4.0);
    ui.label(text);
    ui.add_space(6.0);
    ui.button(OK_LABEL).clicked()
}


pub fn format_megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}


#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;

    use super::*;

    struct AvailableTest {
        release: ReleaseInfo,
        offer: InstallOffer,
        cache: CommonMarkCache,
        action: Option<AvailableAction>,
    }

    fn release(notes: &str) -> ReleaseInfo {
        ReleaseInfo {
            version: Version::new(0, 2, 0),
            tag: "v0.2.0".to_string(),
            notes: notes.to_string(),
            page_url: "https://example.com".to_string(),
            assets: Vec::new(),
        }
    }

    fn available(notes: &str, offer: InstallOffer) -> egui_kittest::Harness<'static, AvailableTest> {
        let test = AvailableTest {
            release: release(notes),
            offer,
            cache: CommonMarkCache::default(),
            action: None,
        };
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(600.0, 600.0)).build_ui_state(
            |ui, test: &mut AvailableTest| {
                if let Some(action) = show_available(ui, &test.release, &Version::new(0, 1, 0), test.offer, &mut test.cache) {
                    test.action = Some(action);
                }
            },
            test,
        );
        harness.run_steps(3);
        harness
    }

    const INSTALLABLE: InstallOffer = InstallOffer { can_install: true, has_unsaved_changes: false };

    #[test]
    fn available_window_names_both_versions_and_shows_the_notes() {
        let harness = available("- Faster diff", INSTALLABLE);
        harness.get_by_label("Kdiff4 0.2.0 is available. You have 0.1.0.");
        harness.get_by_label("Faster diff");
    }

    #[test]
    fn available_window_says_when_there_are_no_notes() {
        let harness = available("  ", INSTALLABLE);
        harness.get_by_label(NO_NOTES_TEXT);
    }

    #[test]
    fn each_choice_reports_its_action() {
        for (label, expected) in [
            (INSTALL_LABEL, AvailableAction::Install),
            (REMIND_LABEL, AvailableAction::RemindLater),
            (SKIP_LABEL, AvailableAction::SkipVersion),
        ] {
            let mut harness = available("notes", INSTALLABLE);
            harness.get_by_label(label).click();
            harness.run_steps(2);
            assert_eq!(harness.state().action, Some(expected));
        }
    }

    #[test]
    fn without_a_download_for_this_platform_the_release_page_is_offered() {
        let mut harness = available("notes", InstallOffer { can_install: false, has_unsaved_changes: false });
        assert!(harness.query_by_label(INSTALL_LABEL).is_none());
        harness.get_by_label(OPEN_PAGE_LABEL).click();
        harness.run_steps(2);
        assert_eq!(harness.state().action, Some(AvailableAction::OpenPage));
    }

    #[test]
    fn unsaved_changes_are_warned_about_before_installing() {
        let warned = available("notes", InstallOffer { can_install: true, has_unsaved_changes: true });
        warned.get_by_label_contains("Unsaved changes are lost");
        let quiet = available("notes", INSTALLABLE);
        assert!(quiet.query_by_label_contains("Unsaved changes are lost").is_none());
    }

    #[test]
    fn progress_window_cancels() {
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(600.0, 300.0)).build_ui_state(
            |ui, canceled: &mut bool| {
                if show_progress(ui, &Version::new(0, 2, 0), Some(0.5), 1024 * 1024, 2 * 1024 * 1024) {
                    *canceled = true;
                }
            },
            false,
        );
        harness.run_steps(3);
        harness.get_by_label("1.0 MB of 2.0 MB");
        harness.get_by_label(CANCEL_LABEL).click();
        harness.run_steps(2);
        assert!(*harness.state());
    }

    #[test]
    fn message_window_is_dismissed_with_ok() {
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(600.0, 300.0)).build_ui_state(
            |ui, dismissed: &mut bool| {
                if show_message(ui, "No Update", "Kdiff4 is up to date.") {
                    *dismissed = true;
                }
            },
            false,
        );
        harness.run_steps(3);
        harness.get_by_label("Kdiff4 is up to date.");
        harness.get_by_label(OK_LABEL).click();
        harness.run_steps(2);
        assert!(*harness.state());
    }

    #[test]
    fn megabytes_are_shown_with_one_decimal() {
        assert_eq!(format_megabytes(0), "0.0 MB");
        assert_eq!(format_megabytes(1572864), "1.5 MB");
    }
}
