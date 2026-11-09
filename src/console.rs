/// On Windows the binary is a GUI-subsystem app, so it never opens a console window. When launched from a terminal, this attaches to that terminal's console so stdout and stderr appear there. Handles that already point somewhere, such as a pipe or a file, are kept. A no-op elsewhere.
pub fn attach_to_parent() {
    platform::attach_to_parent();
}


#[cfg(windows)]
mod platform {
    use std::ffi::c_void;
    use std::ptr;

    type Handle = *mut c_void;

    const STD_OUTPUT_HANDLE: u32 = -11_i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12_i32 as u32;
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_WRITE: u32 = 0x2;
    const OPEN_EXISTING: u32 = 3;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(id: u32) -> Handle;
        fn SetStdHandle(id: u32, handle: Handle) -> i32;
        fn AttachConsole(process_id: u32) -> i32;
        fn CreateFileW(name: *const u16, access: u32, share_mode: u32, security_attributes: *mut c_void, creation_disposition: u32, flags_and_attributes: u32, template_file: Handle) -> Handle;
    }


    /// Null means no handle was inherited; all bits set is `INVALID_HANDLE_VALUE`.
    pub(super) fn is_usable(handle: Handle) -> bool {
        !handle.is_null() && handle.addr() != usize::MAX
    }


    pub(super) fn std_handle(id: u32) -> Handle {
        // SAFETY: GetStdHandle takes the handle id by value, touches no caller memory and has no preconditions.
        unsafe { GetStdHandle(id) }
    }


    pub fn attach_to_parent() {
        let missing: Vec<u32> = [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE]
            .into_iter()
            .filter(|id| !is_usable(std_handle(id.clone())))
            .collect();
        if missing.is_empty() {
            return;
        }
        // SAFETY: AttachConsole takes a process id by value and touches no caller memory. It fails harmlessly when the parent has no console, e.g. when started from Explorer.
        let attached = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } != 0;
        if !attached {
            return;
        }
        let name: Vec<u16> = "CONOUT$".encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: `name` is a NUL-terminated UTF-16 string that outlives the call. Null security attributes and a null template handle are documented as allowed.
        let console = unsafe { CreateFileW(name.as_ptr(), GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, ptr::null_mut(), OPEN_EXISTING, 0, ptr::null_mut()) };
        if !is_usable(console) {
            return;
        }
        for id in missing {
            // SAFETY: `console` is a valid handle to the attached console's output buffer. It is never closed, so it stays valid for as long as std may write to it.
            unsafe { SetStdHandle(id, console) };
        }
    }


    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn null_and_invalid_handles_are_unusable() {
            assert!(!is_usable(ptr::null_mut()));
            assert!(!is_usable(ptr::without_provenance_mut(usize::MAX)));
            assert!(is_usable(ptr::without_provenance_mut(0x40)));
        }


        #[test]
        fn usable_handles_are_kept() {
            let before = [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE].map(std_handle);
            attach_to_parent();
            let after = [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE].map(std_handle);
            for (before, after) in before.into_iter().zip(after) {
                if is_usable(before) {
                    assert_eq!(before, after);
                }
            }
        }
    }
}


#[cfg(not(windows))]
mod platform {
    pub fn attach_to_parent() {}
}
