use std::path::{Path, PathBuf};

/// Path text as typed, pasted or passed on a command line, without surrounding whitespace, line breaks and other control characters. On Windows, surrounding quotes go too ("Copy as path" adds them, and no Windows file name contains one). Returns `None` when nothing is left.
pub fn clean_path_text(text: &str) -> Option<&str> {
    let cleaned = text.trim_matches(|c: char| c.is_whitespace() || c.is_control() || (cfg!(windows) && c == '"'));
    if cleaned.is_empty() { None } else { Some(cleaned) }
}

/// `clean_path_text` for a path; a path that isn't valid Unicode is returned unchanged.
pub fn clean_path(path: &Path) -> Option<PathBuf> {
    match path.to_str() {
        Some(text) => clean_path_text(text).map(PathBuf::from),
        None => Some(path.to_path_buf()),
    }
}


const ELLIPSIS: char = '…';
pub const REVEAL_LABEL: &str = if cfg!(windows) {
    "Reveal in Explorer"
} else if cfg!(target_os = "macos") {
    "Reveal in Finder"
} else {
    "Open Containing Folder"
};


/// The end of `text` that fits `max_width`, after an ellipsis for the start that was cut, so a long path still shows its file name. `width_of` gives the width of a character. Text that fits is returned whole.
pub fn elide_start(text: &str, max_width: f32, mut width_of: impl FnMut(char) -> f32) -> String {
    let total: f32 = text.chars().map(&mut width_of).sum();
    if total <= max_width {
        return text.to_string();
    }
    let mut room = max_width - width_of(ELLIPSIS);
    let mut start = text.len();
    for (index, c) in text.char_indices().rev() {
        room -= width_of(c);
        if room.is_nan() || room < 0.0 {
            break;
        }
        start = index;
    }
    format!("{ELLIPSIS}{}", &text[start..])
}


/// Command that shows `path` in the system's file manager: selected in its folder on Windows and macOS, and the folder alone elsewhere.
pub fn reveal_command(path: &Path) -> std::process::Command {
    if cfg!(windows) {
        // Explorer selects nothing when the path has forward slashes.
        let native = path.to_string_lossy().replace('/', "\\");
        let mut command = std::process::Command::new("explorer");
        command.arg("/select,").arg(native);
        command
    } else if cfg!(target_os = "macos") {
        let mut command = std::process::Command::new("open");
        command.arg("-R").arg(path);
        command
    } else {
        let folder = path.parent().filter(|folder| !folder.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let mut command = std::process::Command::new("xdg-open");
        command.arg(folder);
        command
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_breaks_and_whitespace_are_removed_from_both_ends() {
        assert_eq!(clean_path_text("C:\\dir\\file.txt\n"), Some("C:\\dir\\file.txt"));
        assert_eq!(clean_path_text("\r\n\t file.txt \r\n"), Some("file.txt"));
        assert_eq!(clean_path_text("my file.txt"), Some("my file.txt"));
    }

    #[test]
    fn a_long_path_keeps_its_end_after_an_ellipsis() {
        let one = |_: char| 1.0;
        assert_eq!(elide_start("dir/file.txt", 12.0, one), "dir/file.txt");
        assert_eq!(elide_start("some/dir/file.txt", 9.0, one), "…file.txt");
        assert_eq!(elide_start("dir/файл.txt", 9.0, one), "…файл.txt");
        assert_eq!(elide_start("", 0.0, one), "");
    }

    #[test]
    fn eliding_survives_no_room_and_a_broken_width() {
        let one = |_: char| 1.0;
        assert_eq!(elide_start("file.txt", 0.0, one), "…");
        assert_eq!(elide_start("file.txt", -5.0, one), "…");
        assert_eq!(elide_start("file.txt", f32::NAN, one), "…");
    }

    #[test]
    fn revealing_passes_the_file_or_its_folder_to_the_file_manager() {
        let command = reveal_command(Path::new("some dir/file.txt"));
        let arguments: Vec<String> = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        if cfg!(windows) {
            assert_eq!(command.get_program(), "explorer");
            assert_eq!(arguments, ["/select,", "some dir\\file.txt"]);
        } else if cfg!(target_os = "macos") {
            assert_eq!(arguments, ["-R", "some dir/file.txt"]);
        } else {
            assert_eq!(arguments, ["some dir"]);
        }
    }

    #[test]
    fn blank_text_is_no_path() {
        assert_eq!(clean_path_text(""), None);
        assert_eq!(clean_path_text(" \n\t\r"), None);
        assert_eq!(clean_path(Path::new("\n")), None);
    }

    #[cfg(windows)]
    #[test]
    fn surrounding_quotes_are_removed_on_windows() {
        assert_eq!(clean_path_text("\"C:\\dir\\file.txt\"\n"), Some("C:\\dir\\file.txt"));
        assert_eq!(clean_path_text("C:\\dir\\file.txt\n\""), Some("C:\\dir\\file.txt"));
    }

    #[test]
    fn clean_path_keeps_inner_text() {
        assert_eq!(clean_path(Path::new("a b/c.txt\n")), Some(PathBuf::from("a b/c.txt")));
    }
}
