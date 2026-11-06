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
