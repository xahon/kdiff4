use std::io;
use std::ops::Range;
use std::path::Path;

use kdiff4_protocol::EolStyle;

use crate::error::CoreError;

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Decoded content of one source document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceText {
    pub text: String,
    /// Byte ranges into `text`, without line terminators.
    pub lines: Vec<Range<usize>>,
    /// `None` when the text has no line terminator, so it has no style to tell.
    pub eol: Option<EolStyle>,
    pub bom: bool,
    /// The last line ends with a terminator; true for an empty text.
    pub final_eol: bool,
}

impl SourceText {
    /// Decodes as UTF-8, dropping a UTF-8 BOM; invalid bytes become U+FFFD. Encoding detection comes later.
    pub fn decode(bytes: &[u8]) -> SourceText {
        let bom = bytes.starts_with(UTF8_BOM);
        let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
        let text = String::from_utf8_lossy(bytes).into_owned();
        let lines = split_lines(&text);
        let eol = detect_eol(&text);
        let final_eol = text.is_empty() || text.ends_with(['\n', '\r']);
        SourceText { text, lines, eol, bom, final_eol }
    }

    pub fn load(path: &Path) -> Result<SourceText, CoreError> {
        if path.is_dir() {
            return Err(CoreError::Directory { path: path.to_path_buf() });
        }
        let bytes = std::fs::read(path).map_err(|source| CoreError::Read { path: path.to_path_buf(), source })?;
        Ok(SourceText::decode(&bytes))
    }
}

/// Splits at LF, CRLF and CR. A terminator ends a line, so a trailing terminator adds no empty line.
pub fn split_lines(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\n' => {
                lines.push(start..index);
                index += 1;
                start = index;
            }
            b'\r' => {
                lines.push(start..index);
                index += if bytes.get(index + 1) == Some(&b'\n') { 2 } else { 1 };
                start = index;
            }
            _ => index += 1,
        }
    }
    if start < bytes.len() {
        lines.push(start..bytes.len());
    }
    lines
}

/// The most frequent line terminator; LF on a tie with LF. `None` when there is no terminator at all.
pub fn detect_eol(text: &str) -> Option<EolStyle> {
    let bytes = text.as_bytes();
    let (mut lf, mut crlf, mut cr) = (0usize, 0usize, 0usize);
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\n' => lf += 1,
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                crlf += 1;
                index += 1;
            }
            b'\r' => cr += 1,
            _ => {}
        }
        index += 1;
    }
    if lf + crlf + cr == 0 {
        None
    } else if crlf > lf && crlf >= cr {
        Some(EolStyle::CrLf)
    } else if cr > lf && cr > crlf {
        Some(EolStyle::Cr)
    } else {
        Some(EolStyle::Lf)
    }
}

/// Turns CRLF and CR into LF, the form D is edited in.
pub fn normalize_to_lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// The bytes D is saved as: UTF-8, its `\n` line ends replaced by `eol`, with a BOM if `bom`. Without `final_eol` the last line gets no terminator.
pub fn encode_output(text: &str, eol: EolStyle, bom: bool, final_eol: bool) -> Vec<u8> {
    let text = if final_eol { text } else { text.strip_suffix('\n').unwrap_or(text) };
    let body = match eol {
        EolStyle::Lf => text.to_string(),
        EolStyle::CrLf | EolStyle::Cr => text.replace('\n', eol.terminator()),
    };
    let mut bytes = Vec::with_capacity(body.len() + UTF8_BOM.len());
    if bom {
        bytes.extend_from_slice(UTF8_BOM);
    }
    bytes.extend_from_slice(body.as_bytes());
    bytes
}

/// Writes a temp file next to `path`, then renames it over `path`, so a failed write never leaves a half-written file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut temp_name = std::ffi::OsString::from(".");
    temp_name.push(path.file_name().unwrap_or_default());
    temp_name.push(".kdiff4-tmp");
    let temp_path = path.with_file_name(temp_name);
    std::fs::write(&temp_path, bytes)?;
    if let Err(error) = std::fs::rename(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn line_texts(text: &str) -> Vec<&str> {
        split_lines(text).into_iter().map(|range| &text[range]).collect()
    }

    #[test]
    fn empty_text_has_no_lines() {
        assert!(split_lines("").is_empty());
    }

    #[test]
    fn last_line_without_terminator_is_kept() {
        assert_eq!(line_texts("one\ntwo"), ["one", "two"]);
    }

    #[test]
    fn trailing_terminator_adds_no_line() {
        assert_eq!(line_texts("one\ntwo\n"), ["one", "two"]);
    }

    #[test]
    fn all_eol_styles_split() {
        assert_eq!(line_texts("lf\ncrlf\r\ncr\rlast"), ["lf", "crlf", "cr", "last"]);
    }

    #[test]
    fn empty_lines_are_kept() {
        assert_eq!(line_texts("\n\r\n\r"), ["", "", ""]);
    }

    #[test]
    fn multibyte_characters_stay_intact() {
        assert_eq!(line_texts("héllo\r\nwörld"), ["héllo", "wörld"]);
    }

    #[test]
    fn decode_strips_utf8_bom() {
        let source = SourceText::decode(b"\xEF\xBB\xBFone\ntwo");
        assert_eq!(source.text, "one\ntwo");
        assert_eq!(source.lines, [0..3, 4..7]);
        assert!(source.bom);
        assert!(!SourceText::decode(b"one").bom);
    }

    #[test]
    fn decode_replaces_invalid_bytes() {
        let source = SourceText::decode(b"a\xFFb");
        assert_eq!(source.text, "a\u{FFFD}b");
        assert_eq!(source.lines.len(), 1);
    }

    #[test]
    fn eol_is_the_majority_style() {
        assert_eq!(detect_eol("a\r\nb\r\nc\n"), Some(EolStyle::CrLf));
        assert_eq!(detect_eol("a\rb\rc\n"), Some(EolStyle::Cr));
        assert_eq!(detect_eol("a\nb\r\n"), Some(EolStyle::Lf));
        assert_eq!(detect_eol("a\r\n\r\n"), Some(EolStyle::CrLf));
    }

    #[test]
    fn text_without_terminator_has_no_eol_style() {
        assert_eq!(detect_eol(""), None);
        assert_eq!(detect_eol("no terminator"), None);
        assert_eq!(SourceText::decode(b"one").eol, None);
        assert_eq!(SourceText::decode(b"one\r\ntwo").eol, Some(EolStyle::CrLf));
    }

    #[test]
    fn final_eol_is_whether_the_last_line_is_terminated() {
        assert!(SourceText::decode(b"").final_eol);
        assert!(SourceText::decode(b"one\n").final_eol);
        assert!(SourceText::decode(b"one\r\n").final_eol);
        assert!(SourceText::decode(b"one\r").final_eol);
        assert!(!SourceText::decode(b"one").final_eol);
        assert!(!SourceText::decode(b"one\r\ntwo").final_eol);
    }

    #[test]
    fn normalize_turns_every_terminator_into_lf() {
        assert_eq!(normalize_to_lf("a\r\nb\rc\nd"), "a\nb\nc\nd");
        assert_eq!(normalize_to_lf("\r\r\n"), "\n\n");
    }

    #[test]
    fn encode_output_applies_eol_and_bom() {
        assert_eq!(encode_output("a\nb\n", EolStyle::Lf, false, true), b"a\nb\n");
        assert_eq!(encode_output("a\nb\n", EolStyle::CrLf, false, true), b"a\r\nb\r\n");
        assert_eq!(encode_output("a\nb", EolStyle::Cr, false, true), b"a\rb");
        assert_eq!(encode_output("é", EolStyle::Lf, true, true), "\u{FEFF}é".as_bytes());
    }

    #[test]
    fn encode_output_leaves_the_last_line_unterminated_without_final_eol() {
        assert_eq!(encode_output("a\nb\n", EolStyle::CrLf, false, false), b"a\r\nb");
        assert_eq!(encode_output("a\nb\n\n", EolStyle::Lf, false, false), b"a\nb\n");
        assert_eq!(encode_output("a\nb", EolStyle::Lf, false, false), b"a\nb");
        assert_eq!(encode_output("", EolStyle::Lf, false, false), b"");
    }

    #[test]
    fn write_atomic_replaces_file_and_leaves_no_temp() {
        let dir = std::env::temp_dir().join(format!("kdiff4-core-text-atomic-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.txt");
        fs::write(&path, "old").unwrap();
        write_atomic(&path, b"new").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        let entries: Vec<_> = fs::read_dir(&dir).unwrap().collect();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn write_atomic_into_missing_directory_fails() {
        let path = std::env::temp_dir().join(format!("kdiff4-missing-dir-{}", std::process::id())).join("out.txt");
        assert!(write_atomic(&path, b"x").is_err());
    }
}
