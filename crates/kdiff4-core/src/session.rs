use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use kdiff4_protocol::engine::{CompareInput, Comparison, ComparisonEngine, ComparisonOptions, EngineCapabilities, Job, Progress, SourceLines, run_comparison};
use kdiff4_protocol::{Change, Document, DocumentText, EolStyle, OpenFiles, OutputDocument, OutputEdit, Revision, Snapshot};

use crate::error::CoreError;
use crate::normalizer;
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


/// One open comparison: the source documents, their regions and rows, and, for a merge, D.
#[derive(Debug)]
pub struct Session {
    revision: u64,
    sources: Vec<Source>,
    output: Option<Output>,
    options: ComparisonOptions,
    comparison: Comparison,
    engine: EngineCapabilities,
}


impl Session {
    /// For a merge, D starts with the contents of an existing output file, or empty.
    pub fn open(files: &OpenFiles, engine: &dyn ComparisonEngine) -> Result<Session, CoreError> {
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
        let options = ComparisonOptions::default();
        let comparison = compare(&sources, engine, &options)?;
        Ok(Session { revision: 1, sources, output, options, comparison, engine: engine.capabilities() })
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
        Snapshot { revision: self.revision(), documents, output, comparison: self.comparison.clone(), engine: self.engine.clone() }
    }


    /// Replaces A, B or C with the file at `path` and recomputes the regions and rows. D is kept. On failure the session is unchanged.
    pub fn load_source(&mut self, document: Document, path: &Path, engine: &dyn ComparisonEngine) -> Result<Change, CoreError> {
        let index = self.sources
            .iter()
            .position(|source| source.document == document)
            .ok_or(CoreError::NoSuchDocument { document: document.clone() })?;
        let text = SourceText::load(path)?;
        let old_text = std::mem::replace(&mut self.sources[index].text, text);
        let comparison = match compare(&self.sources, engine, &self.options) {
            Ok(comparison) => comparison,
            Err(error) => {
                self.sources[index].text = old_text;
                return Err(error);
            }
        };
        let source = &mut self.sources[index];
        source.path = path.to_path_buf();
        self.comparison = comparison.clone();
        self.engine = engine.capabilities();
        let change = Change::SourceReplaced { text: source.to_document_text(), comparison };
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


/// Compares the source documents with `engine`. Runs to completion: cancelable worker jobs come later.
fn compare(sources: &[Source], engine: &dyn ComparisonEngine, options: &ComparisonOptions) -> Result<Comparison, CoreError> {
    let lines: Vec<Vec<&str>> = sources
        .iter()
        .map(|source| source.text.lines.iter().map(|range| source.text.text.get(range.clone()).unwrap_or("")).collect())
        .collect();
    let tokens = normalizer::tokenize(&lines);
    let input = CompareInput {
        sources: lines.iter().zip(&tokens).map(|(lines, tokens)| SourceLines { lines, tokens }).collect(),
        alignments: &[],
        options,
    };
    let cancel = AtomicBool::new(false);
    let progress = |progress: Progress| log::debug!("compare: step {} of {}", progress.done, progress.total);
    let comparison = run_comparison(engine, &input, &Job::new(&cancel, &progress))?;
    log::info!("compare: {} regions with {}", comparison.regions.len(), engine.capabilities().id);
    Ok(comparison)
}


#[cfg(test)]
mod tests {
    use std::fs;

    use kdiff4_protocol::engine::{CharDiff, EngineError, HunkChange, MergeStatus, RegionStatus, Row};
    use kdiff4_protocol::{ErrorKind, Labels, ProtocolError};

    use super::*;
    use crate::builtin_engine::{BUILTIN_ENGINE_ID, BuiltinEngine};


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
        Session::open(&files(a, b, Some(c), Some(dir.join("out.txt"))), &BuiltinEngine).unwrap()
    }


    fn edit(session: &Session, range: std::ops::Range<usize>, text: &str) -> OutputEdit {
        OutputEdit { base: session.revision(), range, text: text.to_string() }
    }


    /// Fails every comparison, so a reload with it must leave the session as it was.
    struct FailingEngine;


    impl ComparisonEngine for FailingEngine {
        fn capabilities(&self) -> EngineCapabilities {
            EngineCapabilities { id: "failing".to_string(), ..BuiltinEngine.capabilities() }
        }


        fn compare(&self, _input: &CompareInput<'_>, _job: &Job<'_>) -> Result<Comparison, EngineError> {
            Err(EngineError::Failed("broken on purpose".to_string()))
        }


        fn classify(&self, hunks: &[SourceLines<'_>]) -> Result<RegionStatus, EngineError> {
            BuiltinEngine.classify(hunks)
        }


        fn char_diff(&self, row: &[Option<&str>], options: &ComparisonOptions) -> Result<CharDiff, EngineError> {
            BuiltinEngine.char_diff(row, options)
        }
    }


    fn statuses(comparison: &Comparison) -> Vec<RegionStatus> {
        comparison
            .regions
            .iter()
            .map(|region| region.status.clone())
            .collect()
    }


    #[test]
    fn open_compares_sources() {
        let dir = temp_dir("open-compares");
        let a = write(&dir, "a.txt", "same\nbase\n");
        let b = write(&dir, "b.txt", "same\nleft\n");
        let c = write(&dir, "c.txt", "same\nbase\n");
        let snapshot = Session::open(&files(a, b, Some(c), None), &BuiltinEngine).unwrap().snapshot();
        assert_eq!(statuses(&snapshot.comparison), [RegionStatus::Merge(MergeStatus::Unchanged), RegionStatus::Merge(MergeStatus::ChangedInB)]);
        assert_eq!(snapshot.comparison.regions[1].rows, [Row { lines: vec![Some(1), Some(1), Some(1)] }]);
        assert_eq!(snapshot.engine.id, BUILTIN_ENGINE_ID);
    }


    #[test]
    fn two_way_open_compares_a_and_b() {
        let dir = temp_dir("open-two-way");
        let a = write(&dir, "a.txt", "x\n");
        let b = write(&dir, "b.txt", "x\ny\n");
        let snapshot = Session::open(&files(a, b, None, None), &BuiltinEngine).unwrap().snapshot();
        assert_eq!(statuses(&snapshot.comparison), [RegionStatus::Diff(HunkChange::Unchanged), RegionStatus::Diff(HunkChange::Added)]);
    }


    #[test]
    fn open_fails_when_the_engine_fails() {
        let dir = temp_dir("open-engine-fails");
        let a = write(&dir, "a.txt", "a\n");
        let b = write(&dir, "b.txt", "b\n");
        let error = Session::open(&files(a, b, None, None), &FailingEngine).unwrap_err();
        assert!(matches!(error, CoreError::Engine(EngineError::Failed(_))));
        assert_eq!(ProtocolError::from(error).kind, ErrorKind::Engine);
    }


    #[test]
    fn load_source_recomputes_comparison() {
        let dir = temp_dir("load-recomputes");
        let mut session = merge_session(&dir, "b\n");
        assert_eq!(statuses(&session.snapshot().comparison), [RegionStatus::Merge(MergeStatus::ChangedInC)]);
        let new_c = write(&dir, "c2.txt", "b\n");
        let Change::SourceReplaced { comparison, .. } = session.load_source(Document::C, &new_c, &BuiltinEngine).unwrap() else {
            panic!("expected a replaced source");
        };
        assert_eq!(statuses(&comparison), [RegionStatus::Merge(MergeStatus::Unchanged)]);
        assert_eq!(session.snapshot().comparison, comparison);
    }


    #[test]
    fn failed_recompute_keeps_the_session() {
        let dir = temp_dir("load-engine-fails");
        let mut session = merge_session(&dir, "a\n");
        let before = session.snapshot();
        let new_b = write(&dir, "b2.txt", "other\n");
        let error = session.load_source(Document::B, &new_b, &FailingEngine).unwrap_err();
        assert!(matches!(error, CoreError::Engine(_)));
        assert_eq!(session.snapshot(), before);
    }


    #[test]
    fn canceled_comparison_keeps_its_error_kind() {
        assert_eq!(ProtocolError::from(CoreError::Engine(EngineError::Canceled)).kind, ErrorKind::Canceled);
    }


    #[test]
    fn open_two_files_gives_a_and_b_without_output() {
        let dir = temp_dir("two");
        let a = write(&dir, "a.txt", "one\ntwo\n");
        let b = write(&dir, "b.txt", "one\n");
        let session = Session::open(&files(a.clone(), b.clone(), None, None), &BuiltinEngine).unwrap();
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
        let session = Session::open(&OpenFiles { a, b, c: None, output: None, labels }, &BuiltinEngine).unwrap();
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
        let output = Session::open(&files(a, b, Some(c), None), &BuiltinEngine).unwrap().snapshot().output.unwrap();
        assert_eq!(output, OutputDocument { path: None, text: String::new(), modified: false });
    }


    #[test]
    fn two_way_merge_has_output() {
        let dir = temp_dir("two-way-merge");
        let a = write(&dir, "a.txt", "a");
        let b = write(&dir, "b.txt", "b");
        let snapshot = Session::open(&files(a, b, None, Some(dir.join("out.txt"))), &BuiltinEngine).unwrap().snapshot();
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
        let output = Session::open(&files(a, b, Some(c), Some(out)), &BuiltinEngine).unwrap().snapshot().output.unwrap();
        assert_eq!(output.text, "x\ny\n");
        assert!(!output.modified);
    }


    #[test]
    fn missing_file_is_a_read_error() {
        let dir = temp_dir("missing");
        let a = write(&dir, "a.txt", "a");
        let error = Session::open(&files(a, dir.join("missing.txt"), None, None), &BuiltinEngine).unwrap_err();
        assert!(matches!(error, CoreError::Read { ref path, .. } if path.ends_with("missing.txt")));
    }


    #[test]
    fn directory_as_source_is_rejected() {
        let dir = temp_dir("dir-source");
        let a = write(&dir, "a.txt", "a");
        let error = Session::open(&files(a, dir.clone(), None, None), &BuiltinEngine).unwrap_err();
        assert!(matches!(error, CoreError::Directory { ref path } if *path == dir));
    }


    #[test]
    fn directory_as_output_is_rejected() {
        let dir = temp_dir("dir-output");
        let a = write(&dir, "a.txt", "a");
        let b = write(&dir, "b.txt", "b");
        let c = write(&dir, "c.txt", "c");
        let error = Session::open(&files(a, b, Some(c), Some(dir.clone())), &BuiltinEngine).unwrap_err();
        assert!(matches!(error, CoreError::Directory { ref path } if *path == dir));
    }


    #[test]
    fn load_source_replaces_document_and_bumps_revision() {
        let dir = temp_dir("load-source");
        let mut session = merge_session(&dir, "a\n");
        let new_b = write(&dir, "b2.txt", "new b\n");
        let change = session.load_source(Document::B, &new_b, &BuiltinEngine).unwrap();
        let Change::SourceReplaced { text, .. } = change else {
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
        session.load_source(Document::A, &new_a, &BuiltinEngine).unwrap();
        assert_eq!(session.snapshot().output.unwrap().text, "typed");
    }


    #[test]
    fn load_source_rejects_missing_documents() {
        let dir = temp_dir("load-missing-doc");
        let a = write(&dir, "a.txt", "a");
        let b = write(&dir, "b.txt", "b");
        let mut session = Session::open(&files(a.clone(), b, None, None), &BuiltinEngine).unwrap();
        assert!(matches!(session.load_source(Document::C, &a, &BuiltinEngine), Err(CoreError::NoSuchDocument { document: Document::C })));
        assert!(matches!(session.load_source(Document::D, &a, &BuiltinEngine), Err(CoreError::NoSuchDocument { document: Document::D })));
        assert!(matches!(session.load_source(Document::B, &dir.join("gone.txt"), &BuiltinEngine), Err(CoreError::Read { .. })));
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
        let mut session = Session::open(&files(a, b, None, None), &BuiltinEngine).unwrap();
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
        let mut session = Session::open(&files(a, b, Some(c), None), &BuiltinEngine).unwrap();
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
