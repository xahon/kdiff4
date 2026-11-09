use std::collections::{HashMap, HashSet};

use eframe::egui_wgpu::wgpu::rwh::RawWindowHandle;
use egui::{Context, Key, Margin, Modifiers, Pos2, Rect, Ui, UiBuilder, Vec2, ViewportBuilder, ViewportId, vec2};

/// Height a dialog window opens with, before it is fitted to its content.
const INITIAL_HEIGHT: f32 = 200.0;
/// Size changes smaller than this many points don't refit a window, so rounding can't make it resize every frame.
const FIT_TOLERANCE: f32 = 1.0;
/// Room the content is laid out in while it is measured; scroll areas cap their own height well below it.
const MEASURE_HEIGHT: f32 = 10_000.0;
const MARGIN: i8 = 12;


/// What a dialog window looks like. `width` is the width its content is laid out in.
pub struct DialogSpec<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub width: f32,
    /// Whether the window has a close button and closes on Escape.
    pub closable: bool,
}


/// Where a dialog window is and how big it is, kept while the dialog stays open.
#[derive(Debug, Clone, PartialEq)]
struct Placement {
    size: Vec2,
    position: Option<Pos2>,
    fitted: bool,
}


impl Placement {
    fn new(width: f32, parent: Option<Rect>) -> Self {
        let size = vec2(width, INITIAL_HEIGHT) + margins();
        Self {
            size: size.clone(),
            position: centered(parent, size),
            fitted: false,
        }
    }


    /// Sizes the window to `content`, and centers it over `parent` the first time. Returns whether anything changed.
    fn fit(&mut self, content: Vec2, parent: Option<Rect>) -> bool {
        let size = (content + margins()).ceil();
        let resized = (size - self.size.clone()).abs().max_elem() > FIT_TOLERANCE;
        if resized {
            self.size = size.clone();
        }
        if !self.fitted {
            self.fitted = true;
            let position = centered(parent, size);
            if position.is_some() && position != self.position {
                self.position = position;
                return true;
            }
        }
        resized
    }
}


/// Shows modal dialogs as separate native windows. Call `show` every frame a dialog is open, and `end_frame` once per frame after all of them.
#[derive(Default)]
pub struct DialogWindows {
    placements: HashMap<ViewportId, Placement>,
    /// The title of each dialog shown this frame.
    shown: HashMap<ViewportId, String>,
    /// Dialogs whose window the main window owns already.
    owned: HashSet<ViewportId>,
}


impl DialogWindows {
    /// Shows `content` in its own window over the main one, which owns it and takes no pointer input meanwhile; when the main window gets focus, the dialog takes it back. Where the backend has no native windows, the dialog is an in-window modal instead.
    ///
    /// Returns whether the user closed the window with its close button or Escape. The window stays open until the caller stops showing it.
    pub fn show(&mut self, ctx: &Context, spec: &DialogSpec, mut content: impl FnMut(&mut Ui)) -> bool {
        if ctx.embed_viewports() {
            let modal = egui::Modal::new(egui::Id::unique(spec.id)).show(ctx, |ui| content(ui));
            return spec.closable && modal.should_close();
        }
        let id = ViewportId::from_hash_of(spec.id);
        self.shown.insert(id.clone(), spec.title.to_owned());
        let parent = ctx.input(|input| input.viewport().outer_rect);
        let placement = self.placements.entry(id.clone()).or_insert_with(|| Placement::new(spec.width.clone(), parent.clone()));
        egui::Modal::new(egui::Id::unique((spec.id, "backdrop"))).frame(egui::Frame::NONE).show(ctx, |_| ());
        if ctx.input(|input| input.viewport().focused) == Some(true) {
            ctx.send_viewport_cmd_to(id.clone(), egui::ViewportCommand::Focus);
        }
        let mut builder = ViewportBuilder::default()
            .with_title(spec.title)
            .with_inner_size(placement.size.clone())
            .with_resizable(false)
            .with_minimize_button(false)
            .with_maximize_button(false)
            .with_close_button(spec.closable)
            .with_taskbar(false);
        if let Some(position) = placement.position.clone() {
            builder = builder.with_position(position);
        }
        let width = spec.width.clone();
        let closable = spec.closable;
        ctx.show_viewport_immediate(id, builder, |ui, _| {
            let frame = egui::Frame::central_panel(ui.style()).inner_margin(Margin::same(MARGIN));
            let mut content_size = Vec2::ZERO;
            egui::CentralPanel::default().frame(frame).show(ui, |ui| {
                let room = Rect::from_min_size(ui.cursor().min, vec2(width.clone(), MEASURE_HEIGHT));
                content_size = ui.scope_builder(UiBuilder::new().max_rect(room), |ui| content(ui)).response.rect.size();
            });
            if !placement.fitted {
                let theme = title_bar_theme(ui.ctx().options(|options| options.theme_preference));
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::SetTheme(theme));
            }
            if placement.fit(content_size, parent.clone()) {
                ui.ctx().request_repaint_of(ViewportId::ROOT);
            }
            let ctx = ui.ctx();
            let escape = closable && !egui::Popup::is_any_open(ctx) && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape));
            let closed = closable && ctx.input(|input| input.viewport().close_requested());
            escape || closed
        })
    }


    /// Forgets the placement of every dialog not shown this frame, so it opens centered again next time, and makes `owner`, the main window, own the windows of the dialogs that are shown, so they always stay above it.
    pub fn end_frame(&mut self, owner: Option<RawWindowHandle>) {
        let shown = std::mem::take(&mut self.shown);
        self.placements.retain(|id, _| shown.contains_key(id));
        self.owned.retain(|id| shown.contains_key(id));
        if let Some(owner) = owner {
            self.attach(&shown, |title| platform::set_owner(owner.clone(), title));
        }
    }


    /// Calls `set_owner` with the title of each shown dialog whose window has no owner yet; it returns false while that window doesn't exist, so it is tried again next frame.
    fn attach(&mut self, shown: &HashMap<ViewportId, String>, mut set_owner: impl FnMut(&str) -> bool) {
        for (id, title) in shown {
            if !self.owned.contains(id) && set_owner(title) {
                self.owned.insert(id.clone());
            }
        }
    }
}


/// The theme of a native title bar that matches the app's theme.
pub(crate) fn title_bar_theme(preference: egui::ThemePreference) -> egui::SystemTheme {
    match preference {
        egui::ThemePreference::Light => egui::SystemTheme::Light,
        egui::ThemePreference::Dark => egui::SystemTheme::Dark,
        egui::ThemePreference::System => egui::SystemTheme::SystemDefault,
    }
}


fn margins() -> Vec2 {
    Margin::same(MARGIN).sum()
}


/// The position that centers a window of `size` over `parent`, if its place on screen is known. A window larger than `parent` starts at its top left corner, so its title bar can't end up above the screen.
fn centered(parent: Option<Rect>, size: Vec2) -> Option<Pos2> {
    let parent = parent?;
    Some((parent.center() - size / 2.0).max(parent.min).round())
}


/// Windows keeps an owned window above its owner, so a click on the main window can't hide the dialog behind it. egui can't set an owner, so this finds the dialog's window among this thread's windows by its title.
#[cfg(windows)]
mod platform {
    use std::ffi::c_void;

    use eframe::egui_wgpu::wgpu::rwh::RawWindowHandle;

    const GWLP_HWNDPARENT: i32 = -8;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThreadId() -> u32;
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumThreadWindows(thread: u32, callback: unsafe extern "system" fn(*mut c_void, isize) -> i32, param: isize) -> i32;
        fn GetWindowTextW(hwnd: *mut c_void, text: *mut u16, capacity: i32) -> i32;
        #[cfg(target_pointer_width = "64")]
        fn SetWindowLongPtrW(hwnd: *mut c_void, index: i32, value: isize) -> isize;
        #[cfg(target_pointer_width = "32")]
        #[link_name = "SetWindowLongW"]
        fn SetWindowLongPtrW(hwnd: *mut c_void, index: i32, value: isize) -> isize;
    }


    struct Search {
        owner: *mut c_void,
        title: Vec<u16>,
        found: Option<*mut c_void>,
    }


    /// Makes `owner` own the window of this thread titled `title`. Returns false while there is no such window; true once it is owned, or when `owner` isn't a Win32 window.
    pub fn set_owner(owner: RawWindowHandle, title: &str) -> bool {
        let RawWindowHandle::Win32(owner) = owner else {
            return true;
        };
        let owner = owner.hwnd.get() as *mut c_void;
        let mut search = Search { owner, title: title.encode_utf16().collect(), found: None };
        // SAFETY: GetCurrentThreadId has no preconditions. EnumThreadWindows calls `visit` synchronously, only during this call, with the pointer to `search`, which stays alive on the stack and isn't used elsewhere until the call returns.
        unsafe { EnumThreadWindows(GetCurrentThreadId(), visit, (&raw mut search) as isize) };
        let Some(dialog) = search.found else {
            return false;
        };
        // SAFETY: `dialog` is a live top-level window of this thread that EnumThreadWindows just listed, and `owner` is eframe's live main window. On a top-level window GWLP_HWNDPARENT sets the owner, which only changes how Windows stacks, minimizes and activates it.
        unsafe { SetWindowLongPtrW(dialog, GWLP_HWNDPARENT, owner as isize) };
        true
    }


    unsafe extern "system" fn visit(hwnd: *mut c_void, param: isize) -> i32 {
        // SAFETY: `param` is the pointer to the `Search` that `set_owner` passed to EnumThreadWindows; it is alive and used by nothing else during the enumeration.
        let search = unsafe { &mut *(param as *mut Search) };
        if hwnd == search.owner {
            return 1;
        }
        let mut text = vec![0u16; search.title.len() + 2];
        let capacity = i32::try_from(text.len()).unwrap_or(i32::MAX);
        // SAFETY: `hwnd` was just listed by EnumThreadWindows, and `text` has room for at least `capacity` UTF-16 units, which is all GetWindowTextW writes.
        let length = unsafe { GetWindowTextW(hwnd, text.as_mut_ptr(), capacity) };
        if !title_matches(&text, length, &search.title) {
            return 1;
        }
        search.found = Some(hwnd);
        0
    }


    /// Whether the first `length` units of `text`, as GetWindowTextW returned them, are exactly `title`.
    pub(super) fn title_matches(text: &[u16], length: i32, title: &[u16]) -> bool {
        let text = usize::try_from(length)
            .ok()
            .and_then(|length| text.get(..length));
        text == Some(title)
    }
}


#[cfg(not(windows))]
mod platform {
    use eframe::egui_wgpu::wgpu::rwh::RawWindowHandle;

    pub fn set_owner(_owner: RawWindowHandle, _title: &str) -> bool {
        true
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    fn parent() -> Option<Rect> {
        Some(Rect::from_min_size(Pos2::new(100.0, 50.0), vec2(1000.0, 800.0)))
    }

    #[test]
    fn a_new_dialog_is_centered_over_its_parent() {
        let placement = Placement::new(400.0, parent());
        assert_eq!(placement.size, vec2(400.0, INITIAL_HEIGHT) + margins());
        let center = placement.position.expect("placed") + placement.size / 2.0;
        assert_eq!(center, Pos2::new(600.0, 450.0));
    }

    #[test]
    fn a_dialog_without_a_known_parent_is_left_to_the_system() {
        assert_eq!(Placement::new(400.0, None).position, None);
    }

    #[test]
    fn the_first_fit_sizes_to_the_content_and_centers_again() {
        let mut placement = Placement::new(400.0, parent());
        assert!(placement.fit(vec2(400.0, 300.0), parent()));
        assert_eq!(placement.size, vec2(400.0, 300.0) + margins());
        let center = placement.position.expect("placed") + placement.size / 2.0;
        assert_eq!(center, Pos2::new(600.0, 450.0));
        assert!(placement.fitted);
    }

    #[test]
    fn later_fits_resize_without_moving_the_window() {
        let mut placement = Placement::new(400.0, parent());
        placement.fit(vec2(400.0, 300.0), parent());
        let position = placement.position.clone();
        assert!(placement.fit(vec2(400.0, 500.0), parent()));
        assert_eq!(placement.size, vec2(400.0, 500.0) + margins());
        assert_eq!(placement.position, position);
    }

    #[test]
    fn tiny_size_changes_do_not_refit() {
        let mut placement = Placement::new(400.0, parent());
        placement.fit(vec2(400.0, 300.0), parent());
        let before = placement.clone();
        assert!(!placement.fit(vec2(400.4, 300.6), parent()));
        assert_eq!(placement, before);
    }

    #[test]
    fn content_sizes_are_rounded_up_to_whole_points() {
        let mut placement = Placement::new(400.0, None);
        placement.fit(vec2(400.2, 300.2), None);
        assert_eq!(placement.size, (vec2(400.2, 300.2) + margins()).ceil());
    }

    #[test]
    fn a_dialog_larger_than_its_parent_stays_on_screen() {
        let small = Some(Rect::from_min_size(Pos2::ZERO, vec2(200.0, 100.0)));
        assert_eq!(centered(small, vec2(400.0, 300.0)), Some(Pos2::ZERO));
    }

    #[test]
    fn title_bars_follow_the_app_theme() {
        assert_eq!(title_bar_theme(egui::ThemePreference::Light), egui::SystemTheme::Light);
        assert_eq!(title_bar_theme(egui::ThemePreference::Dark), egui::SystemTheme::Dark);
        assert_eq!(title_bar_theme(egui::ThemePreference::System), egui::SystemTheme::SystemDefault);
    }

    #[test]
    fn dialogs_not_shown_in_a_frame_are_forgotten() {
        let mut windows = DialogWindows::default();
        let kept = ViewportId::from_hash_of("kept");
        let dropped = ViewportId::from_hash_of("dropped");
        windows.placements.insert(kept.clone(), Placement::new(100.0, None));
        windows.placements.insert(dropped.clone(), Placement::new(100.0, None));
        windows.owned.insert(kept.clone());
        windows.owned.insert(dropped.clone());
        windows.shown.insert(kept.clone(), "Kept".to_owned());
        windows.end_frame(None);
        assert!(windows.placements.contains_key(&kept));
        assert!(!windows.placements.contains_key(&dropped));
        assert!(windows.owned.contains(&kept));
        assert!(!windows.owned.contains(&dropped));
        assert!(windows.shown.is_empty());
    }

    #[test]
    fn each_dialog_window_gets_its_owner_once() {
        let mut windows = DialogWindows::default();
        let shown = HashMap::from([(ViewportId::from_hash_of("a"), "A".to_owned()), (ViewportId::from_hash_of("b"), "B".to_owned())]);
        let mut calls = Vec::new();
        windows.attach(&shown, |title| {
            calls.push(title.to_owned());
            true
        });
        windows.attach(&shown, |title| {
            calls.push(title.to_owned());
            true
        });
        calls.sort();
        assert_eq!(calls, ["A", "B"]);
    }

    #[test]
    fn a_dialog_window_not_created_yet_gets_its_owner_later() {
        let mut windows = DialogWindows::default();
        let id = ViewportId::from_hash_of("a");
        let shown = HashMap::from([(id.clone(), "A".to_owned())]);
        windows.attach(&shown, |_| false);
        assert!(!windows.owned.contains(&id));
        let mut tried = false;
        windows.attach(&shown, |_| {
            tried = true;
            true
        });
        assert!(tried);
        assert!(windows.owned.contains(&id));
    }

    #[cfg(windows)]
    #[test]
    fn window_titles_match_exactly() {
        let title: Vec<u16> = "Settings".encode_utf16().collect();
        let mut text = title.clone();
        text.extend([0, 0]);
        assert!(platform::title_matches(&text, 8, &title));
        assert!(!platform::title_matches(&text, 7, &title));
        assert!(!platform::title_matches(&text, 9, &title));
        assert!(!platform::title_matches(&text, 0, &title));
        assert!(!platform::title_matches(&text, -1, &title));
        assert!(!platform::title_matches(&text, 100, &title));
    }

    #[cfg(windows)]
    #[test]
    fn without_a_dialog_window_no_owner_is_set() {
        let owner = eframe::egui_wgpu::wgpu::rwh::Win32WindowHandle::new(std::num::NonZeroIsize::new(1).expect("nonzero"));
        assert!(!platform::set_owner(RawWindowHandle::Win32(owner), "No such window in this test thread"));
    }

    #[test]
    fn without_native_windows_a_dialog_is_shown_in_the_main_window() {
        let mut windows = DialogWindows::default();
        let mut harness = egui_kittest::Harness::builder().with_size(vec2(600.0, 400.0)).build_ui(move |ui| {
            let spec = DialogSpec { id: "test_dialog", title: "Test", width: 200.0, closable: true };
            windows.show(ui.ctx(), &spec, |ui| {
                ui.label("dialog body");
            });
            windows.end_frame(None);
        });
        harness.run();
        harness.get_by_label("dialog body");
    }
}
