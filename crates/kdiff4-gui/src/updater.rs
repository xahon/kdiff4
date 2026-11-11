use std::path::{Path, PathBuf};

use egui_commonmark::CommonMarkCache;
use semver::Version;

use crate::about_dialog;
use crate::config::{self, UpdateConfig};
use crate::dialog_window::{DialogSpec, DialogWindows};
use crate::self_update::{self, UpdateInstall};
use crate::update_check::{self, CheckOutcome, ReleaseInfo, UpdateCheck, UpdateError};
use crate::update_dialog::{self, AvailableAction, InstallOffer};

const UPDATE_DIALOG_ID: &str = "update_dialog";


/// The About or update window in front of everything: build information, the offer, the download with its progress, or a notice.
pub(crate) enum UpdateWindow {
    About,
    Available(ReleaseInfo),
    Installing { release: ReleaseInfo, install: UpdateInstall },
    Message { title: String, text: String },
}


pub(crate) struct Updater {
    current_version: Version,
    update_check: Option<UpdateCheck>,
    pub(crate) update_window: Option<UpdateWindow>,
    update_cache: CommonMarkCache,
    pub(crate) restart: Option<PathBuf>,
}


impl Default for Updater {
    fn default() -> Self {
        Self {
            current_version: update_check::current_version(),
            update_check: None,
            update_window: None,
            update_cache: CommonMarkCache::default(),
            restart: None,
        }
    }
}


impl Updater {
    /// Removes what the last update left behind and, when due, looks for a newer release.
    pub(crate) fn begin_startup_update_tasks(&mut self, ctx: &egui::Context, updates: &UpdateConfig, url: String) {
        self_update::clean_up_previous_update();
        if updates.check_automatically && update_check::is_due(updates.last_check_unix, update_check::now_unix()) {
            self.begin_update_check_at(ctx, false, url);
        }
    }

    pub(crate) fn begin_update_check(&mut self, ctx: &egui::Context, manual: bool) {
        self.begin_update_check_at(ctx, manual, update_check::release_url());
    }


    fn begin_update_check_at(&mut self, ctx: &egui::Context, manual: bool, url: String) {
        if self.update_check.is_some() || matches!(self.update_window, Some(UpdateWindow::Installing { .. })) {
            return;
        }
        let repaint_ctx = ctx.clone();
        self.update_check = Some(UpdateCheck::start(url, manual, move || repaint_ctx.request_repaint()));
    }

    fn poll_update_check(&mut self, updates: &mut UpdateConfig) {
        let Some(check) = self.update_check.as_ref() else {
            return;
        };
        let Some(result) = check.poll() else {
            return;
        };
        let manual = check.manual;
        self.update_check = None;
        match result {
            Ok(release) => {
                updates.last_check_unix = update_check::now_unix();
                let outcome = update_check::evaluate(&self.current_version, release, updates.skipped_version.as_deref(), manual);
                match outcome {
                    CheckOutcome::Available(release) => self.update_window = Some(UpdateWindow::Available(release)),
                    CheckOutcome::UpToDate if manual => {
                        self.update_window = Some(UpdateWindow::Message {
                            title: "No Update Available".to_string(),
                            text: format!("Kdiff4 {} is the newest version.", self.current_version),
                        });
                    }
                    CheckOutcome::UpToDate | CheckOutcome::Skipped => {}
                }
            }
            Err(error) => {
                log::warn!("{error}");
                if manual {
                    self.update_window = Some(UpdateWindow::Message {
                        title: "Update Check Failed".to_string(),
                        text: error.to_string(),
                    });
                }
            }
        }
    }

    fn update_message(&mut self, title: &str, error: &UpdateError) {
        self.update_window = Some(UpdateWindow::Message {
            title: title.to_string(),
            text: error.to_string(),
        });
    }

    pub(crate) fn start_install(&mut self, ctx: &egui::Context, release: ReleaseInfo) {
        let Some(asset) = release.asset_for_this_platform().cloned() else {
            self.open_release_page(&release);
            return;
        };
        let overridden = std::env::var_os(update_check::URL_OVERRIDE_ENV).is_some();
        if !overridden && !asset.url.starts_with("https://") {
            self.update_message("Update Failed", &UpdateError::Download(format!("refusing to download over an insecure address: {}", asset.url)));
            return;
        }
        let repaint_ctx = ctx.clone();
        match UpdateInstall::start(asset.url, asset.size, move || repaint_ctx.request_repaint()) {
            Ok(install) => self.update_window = Some(UpdateWindow::Installing { release, install }),
            Err(error) => self.update_message("Update Failed", &error),
        }
    }

    fn open_release_page(&mut self, release: &ReleaseInfo) {
        if !release.page_url.starts_with("https://") && !release.page_url.starts_with("http://") {
            self.update_window = Some(UpdateWindow::Message {
                title: "Cannot Open Release Page".to_string(),
                text: format!("{:?} is not a web address.", release.page_url),
            });
            return;
        }
        if let Err(error) = config::open_command(Path::new(&release.page_url)).spawn() {
            self.update_window = Some(UpdateWindow::Message {
                title: "Cannot Open Release Page".to_string(),
                text: format!("Cannot open {}: {error}", release.page_url),
            });
        }
    }

    pub(crate) fn finish_install(&mut self, exe: PathBuf, result: Result<(), UpdateError>) {
        match result {
            Ok(()) => {
                self.restart = Some(exe);
            }
            Err(UpdateError::Canceled) => {}
            Err(error) => self.update_message("Update Failed", &error),
        }
    }

    pub(crate) fn show(&mut self, ctx: &egui::Context, dialogs: &mut DialogWindows, updates: &mut UpdateConfig, has_unsaved_changes: bool) {
        self.poll_update_check(updates);
        let mut choice = None;
        let mut finished = None;
        let mut dismissed = false;
        match self.update_window.as_ref() {
            None => return,
            Some(UpdateWindow::About) => {
                let spec = DialogSpec { id: "about_dialog", title: "About Kdiff4", width: about_dialog::DIALOG_WIDTH, closable: true };
                let mut check_for_updates = false;
                let closed = dialogs.show(ctx, &spec, |ui| check_for_updates |= about_dialog::show(ui, self.update_check.is_some()));
                if check_for_updates {
                    self.update_window = None;
                    self.begin_update_check(ctx, true);
                } else {
                    dismissed = closed;
                }
            }
            Some(UpdateWindow::Available(release)) => {
                let offer = InstallOffer {
                    can_install: release.asset_for_this_platform().is_some(),
                    has_unsaved_changes,
                };
                let spec = DialogSpec { id: UPDATE_DIALOG_ID, title: "Update Available", width: update_dialog::DIALOG_WIDTH, closable: true };
                let closed = dialogs.show(ctx, &spec, |ui| {
                    choice = update_dialog::show_available(ui, release, &self.current_version, offer.clone(), &mut self.update_cache);
                });
                if choice.is_none() && closed {
                    choice = Some(AvailableAction::RemindLater);
                }
            }
            Some(UpdateWindow::Installing { release, install }) => match install.poll() {
                Some(result) => finished = Some(result),
                None => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                    let downloaded = install.downloaded.load(std::sync::atomic::Ordering::Relaxed);
                    let total = install.total.load(std::sync::atomic::Ordering::Relaxed);
                    let spec = DialogSpec { id: UPDATE_DIALOG_ID, title: "Updating Kdiff4", width: update_dialog::DIALOG_WIDTH, closable: false };
                    dialogs.show(ctx, &spec, |ui| {
                        if update_dialog::show_progress(ui, &release.version, install.fraction(), downloaded, total) {
                            install.cancel();
                        }
                    });
                }
            },
            Some(UpdateWindow::Message { title, text }) => {
                let spec = DialogSpec { id: UPDATE_DIALOG_ID, title, width: update_dialog::DIALOG_WIDTH, closable: true };
                let closed = dialogs.show(ctx, &spec, |ui| dismissed = update_dialog::show_message(ui, title, text));
                dismissed |= closed;
            }
        }
        if dismissed {
            self.update_window = None;
        }
        if let Some(result) = finished {
            if let Some(UpdateWindow::Installing { install, .. }) = self.update_window.take() {
                self.finish_install(install.exe.clone(), result);
            }
        }
        if let Some(choice) = choice {
            if let Some(UpdateWindow::Available(release)) = self.update_window.take() {
                self.handle_update_choice(ctx, updates, choice, release);
            }
        }
    }

    pub(crate) fn handle_update_choice(&mut self, ctx: &egui::Context, updates: &mut UpdateConfig, choice: AvailableAction, release: ReleaseInfo) {
        match choice {
            AvailableAction::Install => self.start_install(ctx, release),
            AvailableAction::OpenPage => self.open_release_page(&release),
            AvailableAction::RemindLater => {}
            AvailableAction::SkipVersion => {
                updates.skipped_version = Some(release.version.to_string());
            }
        }
    }
}
