use std::path::{Path, PathBuf};

use kdiff4_protocol::{Change, Document, DocumentText, EolStyle, OpenFiles, OutputDocument, OutputEdit, Revision, Snapshot};

use crate::error::CoreError;
use crate::text::{self, SourceText};
use crate::undo::{History, Step};


#[derive(Debug)]
struct Source {
    document: Document,
    path: PathBuf,
    label: Option<String>,
    text: SourceText,
}


impl Source {
    fn to_document_text(&self) -> DocumentText {
        DocumentText { document: self.document.clone(), path: self.path.clone(), label: self.label.clone(), eol: self.text.eol.clone(), text: self.text.text.clone(), lines: self.text.lines.clone() }
    }
}


/// D. Its text uses `\n` line ends.
#[derive(Debug)]
struct Output {
    path: Option<PathBuf>,
    text: String,
    history: History,
}


impl Output {
    /// Replaces `range`, which the caller has checked, and reports the change.
    fn replace(&mut self, range: std::ops::Range<usize>, text: &str) -> Change {
        self.text.replace_range(range.clone(), text);
        Change::OutputEdited { range, text: text.to_string(), modified: self.history.is_modified() }
    }
}


/// One open comparison: the source documents and, for a merge, D.
#[derive(Debug)]
pub struct Session {
    revision: u64,
    sources: Vec<Source>,
    output: Option<Output>,
}


impl Session {
    /// For a merge, D starts with the contents of an existing output file, or empty.
    pub fn open(files: &OpenFiles) -> Result<Session, CoreError> {
        if let Some(output) = files.output.as_deref()
            && output.is_dir()
        {
            return Err(CoreError::Directory { path: output.to_path_buf() });
        }
        let mut paths: Vec<(Document, &Path)> = vec![(Document::A, files.a.as_path()), (Document::B, files.b.as_path())];
        if let Some(c) = files.c.as_deref() {
            paths.push((Document::C, c));
        }
        let mut sources = Vec::with_capacity(paths.len());
        for (document, path) in paths {
            let text = SourceText::load(path)?;
            let label = files.labels.get(document.clone()).map(str::to_string);
            sources.push(Source { document, path: path.to_path_buf(), label, text });
        }
        let output = if files.is_merge() {
            let text = match files.output.as_deref() {
                Some(path) if path.is_file() => text::normalize_to_lf(&SourceText::load(path)?.text),
                _ => String::new(),
            };
            Some(Output { path: files.output.clone(), text, history: History::new_saved() })
        } else {
            None
        };
        Ok(Session { revision: 1, sources, output })
    }


    pub fn revision(&self) -> Revision {
        Revision(self.revision.clone())
    }


    pub fn snapshot(&self) -> Snapshot {
        let documents = self.sources
            .iter()
            .map(Source::to_document_text)
            .collect();
        let output = self.output
            .as_ref()
            .map(|output| OutputDocument { path: output.path.clone(), text: output.text.clone(), modified: output.history.is_modified() });
        Snapshot { revision: self.revision(), documents, output }
    }


    /// Replaces A, B or C with the file at `path`. D is kept.
    pub fn load_source(&mut self, document: Document, path: &Path) -> Result<Change, CoreError> {
        let index = self.sources
            .iter()
            .position(|source| source.document == document)
            .ok_or(CoreError::NoSuchDocument { document: document.clone() })?;
        let text = SourceText::load(path)?;
        let source = &mut self.sources[index];
        source.path = path.to_path_buf();
        source.text = text;
        let change = Change::SourceReplaced(source.to_document_text());
        self.revision += 1;
        Ok(change)
    }


    pub fn edit_output(&mut self, edit: &OutputEdit) -> Result<Change, CoreError> {
        let current = self.revision();
        let output = self.output.as_mut().ok_or(CoreError::NotAMerge)?;
        if edit.base != current {
            return Err(CoreError::StaleRevision { base: edit.base.clone(), current });
        }
        let range = edit.range.clone();
        let fits = range.start <= range.end && output.text.is_char_boundary(range.start) && output.text.is_char_boundary(range.end) && range.end <= output.text.len();
        if !fits {
            return Err(CoreError::InvalidRange { start: range.start, end: range.end });
        }
        let inserted = text::normalize_to_lf(&edit.text);
        let removed = output.text[range.clone()].to_string();
        output.history.record(Step { start: range.start, removed, inserted: inserted.clone() });
        let change = output.replace(range, &inserted);
        self.revision += 1;
        Ok(change)
    }


    /// `Ok(None)` when there is nothing to undo.
    pub fn undo(&mut self) -> Result<Option<Change>, CoreError> {
        let output = self.output.as_mut().ok_or(CoreError::NotAMerge)?;
        let (range, text) = match output.history.undo() {
            Some(step) => {
                let (range, text) = step.backward();
                (range, text.to_string())
            }
            None => return Ok(None),
        };
        let change = output.replace(range, &text);
        self.revision += 1;
        Ok(Some(change))
    }


    /// `Ok(None)` when there is nothing to redo.
    pub fn redo(&mut self) -> Result<Option<Change>, CoreError> {
        let output = self.output.as_mut().ok_or(CoreError::NotAMerge)?;
        let (range, text) = match output.history.redo() {
            Some(step) => {
                let (range, text) = step.forward();
                (range, text.to_string())
            }
            None => return Ok(None),
        };
        let change = output.replace(range, &text);
        self.revision += 1;
        Ok(Some(change))
    }


    pub fn set_output_path(&mut self, path: Option<PathBuf>) -> Result<Change, CoreError> {
        if let Some(path) = path.as_deref()
            && path.is_dir()
        {
            return Err(CoreError::Directory { path: path.to_path_buf() });
        }
        let output = self.output.as_mut().ok_or(CoreError::NotAMerge)?;
        output.path = path.clone();
        self.revision += 1;
        Ok(Change::OutputPathChanged(path))
    }


    pub fn save(&mut self) -> Result<Change, CoreError> {
        let path = self.output
            .as_ref()
            .ok_or(CoreError::NotAMerge)?
            .path
            .clone()
            .ok_or(CoreError::NoOutputPath)?;
        self.write_output(&path)
    }


    /// The output keeps its old path if the write fails.
    pub fn save_as(&mut self, path: &Path) -> Result<Change, CoreError> {
        self.write_output(path)
    }


    /// Writes D with the EOL style and BOM of A, the output reference.
    fn write_output(&mut self, path: &Path) -> Result<Change, CoreError> {
        if path.is_dir() {
            return Err(CoreError::Directory { path: path.to_path_buf() });
        }
        let (eol, bom) = self.sources
            .first()
            .map(|source| (source.text.eol.clone(), source.text.bom.clone()))
            .unwrap_or((EolStyle::Lf, false));
        let output = self.output.as_mut().ok_or(CoreError::NotAMerge)?;
        let bytes = text::encode_output(&output.text, eol, bom);
        text::write_atomic(path, &bytes).map_err(|source| CoreError::Write { path: path.to_path_buf(), source })?;
        output.path = Some(path.to_path_buf());
        output.history.mark_saved();
        self.revision += 1;
        Ok(Change::OutputSaved(path.to_path_buf()))
    }
}


#[cfg(test)]
mod tests {
    use std::fs;

    use kdiff4_protocol::Labels;

    use super::*;


    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kdiff4-core-session-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }


    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, text).unwrap();
        path
    }


    fn files(a: PathBuf, b: PathBuf, c: Option<PathBuf>, output: Option<PathBuf>) -> OpenFiles {
        OpenFiles { a, b, c, output, labels: Labels::default() }
    }


    /// A 3-way merge of small files in `dir`, with the output at `dir/out.txt` (not created).
    fn merge_session(dir: &Path, a_text: &str) -> Session {
        let a = write(dir, "a.txt", a_text);
        let b = write(dir, "b.txt", "b\n");
        let c = write(dir, "c.txt", "c\n");
        Session::open(&files(a, b, Some(c), Some(dir.join("out.txt")))).unwrap()
    }


    fn edit(session: &Session, range: std::ops::Range<usize>, text: &str) -> OutputEdit {
        OutputEdit { base: session.revision(), range, text: text.to_string() }
    }


    #[test]
    fn open_two_files_gives_a_and_b_without_output() {
        let dir = temp_dir("two");
        let a = write(&dir, "a.txt", "one\ntwo\n");
        let b = write(&dir, "b.txt", "one\n");
        let session = Session::open(&files(a.clone(), b.clone(), None, None)).unwrap();
        let snapshot = session.snapshot();
        assert_eq!(snapshot.revision, Revision(1));
        assert_eq!(snapshot.output, None);
        assert_eq!(snapshot.documents.len(), 2);
        assert_eq!(snapshot.documents[0].document, Document::A);
        assert_eq!(snapshot.documents[0].path, a);
        assert_eq!(snapshot.documents[0].line(1), "two");
        assert_eq!(snapshot.documents[1].document, Document::B);
        assert_eq!(snapshot.documents[1].path, b);
    }


    #[test]
    fn labels_reach_the_snapshot() {
        let dir = temp_dir("labels");
        let a = write(&dir, "a.txt", "a");
        let b = write(&dir, "b.txt", "b");
        let labels = Labels { a: Some("Base".to_string()), b: Some("Local".to_string()), c: None };
        let session = Session::open(&OpenFiles { a, b, c: None, output: None, labels }).unwrap();
        let snapshot = session.snapshot();
        assert_eq!(snapshot.documents[0].label.as_deref(), Some("Base"));
        assert_eq!(snapshot.documents[1].label.as_deref(), Some("Local"));
    }


    #[test]
    fn merge_without_output_has_empty_unnamed_output() {
        let dir = temp_dir("in-memory");
        let a = write(&dir, "a.txt", "a");
        let b = write(&dir, "b.txt", "b");
        let c = write(&dir, "c.txt", "c");
        let output = Session::open(&files(a, b, Some(c), None)).unwrap().snapshot().output.unwrap();
        assert_eq!(output, OutputDocument { path: None, text: String::new(), modified: false });
    }


    #[test]
    fn two_way_merge_has_output() {
        let dir = temp_dir("two-way-merge");
        let a = write(&dir, "a.txt", "a");
        let b = write(&dir, "b.txt", "b");
        let snapshot = Session::open(&files(a, b, None, Some(dir.join("out.txt")))).unwrap().snapshot();
        assert_eq!(snapshot.documents.len(), 2);
        assert_eq!(snapshot.output.unwrap().path, Some(dir.join("out.txt")));
    }


    #[test]
    fn existing_output_file_is_loaded_with_lf_line_ends() {
        let dir = temp_dir("existing-output");
        let a = write(&dir, "a.txt", "a");
        let b = write(&dir, "b.txt", "b");
        let c = write(&dir, "c.txt", "c");
        let out = write(&dir, "out.txt", "\u{FEFF}x\r\ny\r\n");
        let output = Session::open(&files(a, b, Some(c), Some(out))).unwrap().snapshot().output.unwrap();
        assert_eq!(output.text, "x\ny\n");
        assert!(!output.modified);
    }


    #[test]
    fn missing_file_is_a_read_error() {
        let dir = temp_dir("missing");
        let a = write(&dir, "a.txt", "a");
        let error = Session::open(&files(a, dir.join("missing.txt"), None, None)).unwrap_err();
        assert!(matches!(error, CoreError::Read { ref path, .. } if path.ends_with("missing.txt")));
    }


    #[test]
    fn directory_as_source_is_rejected() {
        let dir = temp_dir("dir-source");
        let a = write(&dir, "a.txt", "a");
        let error = Session::open(&files(a, dir.clone(), None, None)).unwrap_err();
        assert!(matches!(error, CoreError::Directory { ref path } if *path == dir));
    }


    #[test]
    fn directory_as_output_is_rejected() {
        let dir = temp_dir("dir-output");
        let a = write(&dir, "a.txt", "a");
        let b = write(&dir, "b.txt", "b");
        let c = write(&dir, "c.txt", "c");
        let error = Session::open(&files(a, b, Some(c), Some(dir.clone()))).unwrap_err();
        assert!(matches!(error, CoreError::Directory { ref path } if *path == dir));
    }


    #[test]
    fn load_source_replaces_document_and_bumps_revision() {
        let dir = temp_dir("load-source");
        let mut session = merge_session(&dir, "a\n");
        let new_b = write(&dir, "b2.txt", "new b\n");
        let change = session.load_source(Document::B, &new_b).unwrap();
        let Change::SourceReplaced(text) = change else {
            panic!("expected a replaced source");
        };
        assert_eq!(text.document, Document::B);
        assert_eq!(text.path, new_b);
        assert_eq!(text.line(0), "new b");
        assert_eq!(session.revision(), Revision(2));
        assert_eq!(session.snapshot().documents[1].line(0), "new b");
    }


    #[test]
    fn load_source_keeps_output_edits() {
        let dir = temp_dir("load-keeps-output");
        let mut session = merge_session(&dir, "a\n");
        session.edit_output(&edit(&session, 0..0, "typed")).unwrap();
        let new_a = write(&dir, "a2.txt", "a2\n");
        session.load_source(Document::A, &new_a).unwrap();
        assert_eq!(session.snapshot().output.unwrap().text, "typed");
    }


    #[test]
    fn load_source_rejects_missing_documents() {
        let dir = temp_dir("load-missing-doc");
        let a = write(&dir, "a.txt", "a");
        let b = write(&dir, "b.txt", "b");
        let mut session = Session::open(&files(a.clone(), b, None, None)).unwrap();
        assert!(matches!(session.load_source(Document::C, &a), Err(CoreError::NoSuchDocument { document: Document::C })));
        assert!(matches!(session.load_source(Document::D, &a), Err(CoreError::NoSuchDocument { document: Document::D })));
        assert!(matches!(session.load_source(Document::B, &dir.join("gone.txt")), Err(CoreError::Read { .. })));
        assert_eq!(session.revision(), Revision(1));
    }


    #[test]
    fn edit_output_replaces_range_and_marks_modified() {
        let dir = temp_dir("edit");
        let mut session = merge_session(&dir, "a\n");
        session.edit_output(&edit(&session, 0..0, "héllo\r\nworld")).unwrap();
        let change = session.edit_output(&edit(&session, 1..3, "e")).unwrap();
        assert_eq!(change, Change::OutputEdited { range: 1..3, text: "e".to_string(), modified: true });
        let output = session.snapshot().output.unwrap();
        assert_eq!(output.text, "hello\nworld");
        assert!(output.modified);
        assert_eq!(session.revision(), Revision(3));
    }


    #[test]
    fn edit_output_rejects_stale_revision_and_bad_ranges() {
        let dir = temp_dir("edit-invalid");
        let mut session = merge_session(&dir, "a\n");
        session.edit_output(&edit(&session, 0..0, "é")).unwrap();
        let stale = OutputEdit { base: Revision(1), range: 0..0, text: "x".to_string() };
        assert!(matches!(session.edit_output(&stale), Err(CoreError::StaleRevision { .. })));
        assert!(matches!(session.edit_output(&edit(&session, 1..1, "x")), Err(CoreError::InvalidRange { .. })));
        assert!(matches!(session.edit_output(&edit(&session, 0..9, "x")), Err(CoreError::InvalidRange { .. })));
        let reversed = edit(&session, 2..0, "x");
        assert!(matches!(session.edit_output(&reversed), Err(CoreError::InvalidRange { .. })));
        assert_eq!(session.snapshot().output.unwrap().text, "é");
    }


    #[test]
    fn undo_and_redo_replay_edits_and_track_modified() {
        let dir = temp_dir("undo");
        let mut session = merge_session(&dir, "a\n");
        session.edit_output(&edit(&session, 0..0, "a")).unwrap();
        session.edit_output(&edit(&session, 1..1, "b")).unwrap();
        session.edit_output(&edit(&session, 2..2, "\n")).unwrap();
        assert_eq!(session.undo().unwrap(), Some(Change::OutputEdited { range: 2..3, text: String::new(), modified: true }));
        assert_eq!(session.undo().unwrap(), Some(Change::OutputEdited { range: 0..2, text: String::new(), modified: false }));
        assert_eq!(session.undo().unwrap(), None);
        assert_eq!(session.snapshot().output.unwrap().text, "");
        assert!(!session.snapshot().output.unwrap().modified);
        assert_eq!(session.redo().unwrap(), Some(Change::OutputEdited { range: 0..0, text: "ab".to_string(), modified: true }));
        assert_eq!(session.snapshot().output.unwrap().text, "ab");
        assert_eq!(session.revision(), Revision(7));
    }


    #[test]
    fn undo_back_to_saved_text_is_unmodified() {
        let dir = temp_dir("undo-saved");
        let mut session = merge_session(&dir, "a\n");
        session.edit_output(&edit(&session, 0..0, "x")).unwrap();
        session.save().unwrap();
        session.edit_output(&edit(&session, 1..1, "y")).unwrap();
        assert!(session.snapshot().output.unwrap().modified);
        session.undo().unwrap();
        assert!(!session.snapshot().output.unwrap().modified);
        session.undo().unwrap();
        assert!(session.snapshot().output.unwrap().modified);
    }


    #[test]
    fn diff_session_rejects_output_commands() {
        let dir = temp_dir("diff-output");
        let a = write(&dir, "a.txt", "a");
        let b = write(&dir, "b.txt", "b");
        let mut session = Session::open(&files(a, b, None, None)).unwrap();
        assert!(matches!(session.edit_output(&edit(&session, 0..0, "x")), Err(CoreError::NotAMerge)));
        assert!(matches!(session.save(), Err(CoreError::NotAMerge)));
        assert!(matches!(session.set_output_path(None), Err(CoreError::NotAMerge)));
        assert!(matches!(session.undo(), Err(CoreError::NotAMerge)));
    }


    #[test]
    fn save_writes_with_eol_and_bom_of_a() {
        let dir = temp_dir("save");
        let mut session = merge_session(&dir, "\u{FEFF}one\r\ntwo\r\n");
        session.edit_output(&edit(&session, 0..0, "x\ny\n")).unwrap();
        assert_eq!(session.save().unwrap(), Change::OutputSaved(dir.join("out.txt")));
        assert_eq!(fs::read(dir.join("out.txt")).unwrap(), "\u{FEFF}x\r\ny\r\n".as_bytes());
        assert!(!session.snapshot().output.unwrap().modified);
    }


    #[test]
    fn save_without_path_fails() {
        let dir = temp_dir("save-no-path");
        let a = write(&dir, "a.txt", "a");
        let b = write(&dir, "b.txt", "b");
        let c = write(&dir, "c.txt", "c");
        let mut session = Session::open(&files(a, b, Some(c), None)).unwrap();
        assert!(matches!(session.save(), Err(CoreError::NoOutputPath)));
    }


    #[test]
    fn save_as_moves_the_output_path() {
        let dir = temp_dir("save-as");
        let mut session = merge_session(&dir, "a\n");
        session.edit_output(&edit(&session, 0..0, "z\n")).unwrap();
        let target = dir.join("other.txt");
        session.save_as(&target).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "z\n");
        assert_eq!(session.snapshot().output.unwrap().path, Some(target));
        assert!(!dir.join("out.txt").exists());
    }


    #[test]
    fn failed_save_as_keeps_old_path_and_modified() {
        let dir = temp_dir("save-as-fail");
        let mut session = merge_session(&dir, "a\n");
        session.edit_output(&edit(&session, 0..0, "z")).unwrap();
        let revision = session.revision();
        assert!(matches!(session.save_as(&dir.join("missing-dir").join("x.txt")), Err(CoreError::Write { .. })));
        assert!(matches!(session.save_as(&dir), Err(CoreError::Directory { .. })));
        let output = session.snapshot().output.unwrap();
        assert_eq!(output.path, Some(dir.join("out.txt")));
        assert!(output.modified);
        assert_eq!(session.revision(), revision);
    }


    #[test]
    fn set_output_path_changes_location_only() {
        let dir = temp_dir("set-path");
        let mut session = merge_session(&dir, "a\n");
        let target = dir.join("elsewhere.txt");
        assert_eq!(session.set_output_path(Some(target.clone())).unwrap(), Change::OutputPathChanged(Some(target.clone())));
        assert!(!target.exists());
        assert_eq!(session.snapshot().output.unwrap().path, Some(target));
        session.set_output_path(None).unwrap();
        assert_eq!(session.snapshot().output.unwrap().path, None);
        assert!(matches!(session.set_output_path(Some(dir.clone())), Err(CoreError::Directory { .. })));
    }
}
