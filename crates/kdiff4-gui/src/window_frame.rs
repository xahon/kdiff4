use eframe::egui_wgpu::wgpu::rwh::HasWindowHandle;


/// How Windows 11 rounds the corners of the native window frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CornerPreference {
    Square,
    Small,
    Round,
}


/// Windows offers only three roundings, so the configured radius picks the closest.
pub fn corner_preference(radius: u8) -> CornerPreference {
    match radius {
        0 => CornerPreference::Square,
        1..=4 => CornerPreference::Small,
        _ => CornerPreference::Round,
    }
}


/// Applies `preference` to the native frame of `window`. A no-op outside Windows; on Windows 10, which can't round frames, the call fails and is only logged.
pub fn apply(window: &impl HasWindowHandle, preference: CornerPreference) {
    let handle = match window.window_handle() {
        Ok(handle) => handle,
        Err(error) => {
            log::debug!("window frame: no window handle: {error}");
            return;
        }
    };
    platform::set_corner_preference(handle.as_raw(), preference);
}


#[cfg(windows)]
mod platform {
    use std::ffi::c_void;

    use eframe::egui_wgpu::wgpu::rwh::RawWindowHandle;

    use super::CornerPreference;


    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWCP_DONOTROUND: u32 = 1;
    const DWMWCP_ROUND: u32 = 2;
    const DWMWCP_ROUNDSMALL: u32 = 3;


    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(hwnd: *mut c_void, attribute: u32, value: *const c_void, size: u32) -> i32;
    }


    pub fn set_corner_preference(handle: RawWindowHandle, preference: CornerPreference) {
        let RawWindowHandle::Win32(handle) = handle else {
            return;
        };
        let value: u32 = match preference {
            CornerPreference::Square => DWMWCP_DONOTROUND,
            CornerPreference::Small => DWMWCP_ROUNDSMALL,
            CornerPreference::Round => DWMWCP_ROUND,
        };
        let hwnd = handle.hwnd.get() as *mut c_void;
        // SAFETY: `hwnd` comes from eframe's live main window, which outlives this call. `value` is a `u32` on the stack that stays alive for the call, and the size passed is exactly its size, as DWMWA_WINDOW_CORNER_PREFERENCE requires. The function only reads `value` and has no other preconditions.
        let result = unsafe { DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, (&raw const value).cast(), size_of::<u32>() as u32) };
        if result != 0 {
            log::debug!("window frame: DwmSetWindowAttribute failed with HRESULT {result:#010x}");
        }
    }
}


#[cfg(not(windows))]
mod platform {
    use eframe::egui_wgpu::wgpu::rwh::RawWindowHandle;

    use super::CornerPreference;


    pub fn set_corner_preference(_handle: RawWindowHandle, _preference: CornerPreference) {}
}


#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn radius_maps_to_closest_windows_rounding() {
        assert_eq!(corner_preference(0), CornerPreference::Square);
        assert_eq!(corner_preference(1), CornerPreference::Small);
        assert_eq!(corner_preference(4), CornerPreference::Small);
        assert_eq!(corner_preference(5), CornerPreference::Round);
        assert_eq!(corner_preference(12), CornerPreference::Round);
    }
}
