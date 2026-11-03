use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use kdiff4_protocol::engine::{CompareInput, Comparison, ComparisonEngine, ComparisonOptions, EngineCapabilities, HunkChange, Job, MergeStatus, Progress, Region, RegionStatus, SourceLines, run_comparison};
use kdiff4_protocol::{Change, Choice, Document, DocumentText, EolStyle, OpenFiles, OutputDocument, OutputEdit, RegionMerge, Revision, Snapshot};

use crate::error::CoreError;
use crate::merge::{self, Merge, MergeStep, RegionState};
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


    fn lines(&self) -> Vec<&str> {
        self.text
            .lines
            .iter()
            .map(|range| self.text.text.get(range.clone()).unwrap_or(""))
            .collect()
    }
}


/// D. Its text uses `\n` line ends and is always the concatenation of the regions' texts.
#[derive(Debug)]
struct Output {
    path: Option<PathBuf>,
    text: String,
    history: History<MergeStep>,
}


/// One open comparison: the source documents, their regions and rows, and, for a merge, the merge state and D.
#[derive(Debug)]
pub struct Session {
    revision: u64,
    sources: Vec<Source>,
    output: Option<Output>,
    options: ComparisonOptions,
    merge: Merge,
    engine: EngineCapabilities,
}


impl Session {
    /// For a merge, every region is auto-resolved and D is built from the result.
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
        let options = ComparisonOptions::default();
        let comparison = compare(&sources, engine, &options)?;
        let (merge, output) = if files.is_merge() {
            let (merge, text) = Merge::with_output(comparison.regions, &source_lines(&sources));
            (merge, Some(Output { path: files.output.clone(), text, history: History::new_saved() }))
        } else {
            (Merge::without_output(comparison.regions), None)
        };
        Ok(Session { revision: 1, sources, output, options, merge, engine: engine.capabilities() })
    }


    pub fn revision(&self) -> Revision {
        Revision(self.revision.clone())
    }


    pub fn unresolved_count(&self) -> usize {
        self.merge.unresolved_count()
    }


    pub fn snapshot(&self) -> Snapshot {
        let documents = self.sources
            .iter()
            .map(Source::to_document_text)
            .collect();
        let output = self.output
            .as_ref()
            .map(|output| OutputDocument { path: output.path.clone(), text: output.text.clone(), modified: output.history.is_modified() });
        let comparison = Comparison { regions: self.merge.regions().to_vec() };
        Snapshot { revision: self.revision(), documents, output, comparison, engine: self.engine.clone(), merge: self.merge.region_merges() }
    }


    /// Replaces A, B or C with the file at `path` and recomputes the regions and rows. D is rebuilt from them, its choices and edits are dropped and its undo history starts over. On failure the session is unchanged.
    pub fn load_source(&mut self, document: Document, path: &Path, engine: &dyn ComparisonEngine) -> Result<(), CoreError> {
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
        self.sources[index].path = path.to_path_buf();
        self.engine = engine.capabilities();
        match self.output.as_mut() {
            Some(output) => {
                let (merge, text) = Merge::with_output(comparison.regions, &source_lines(&self.sources));
                self.merge = merge;
                output.text = text;
                output.history = History::new_unsaved();
            }
            None => self.merge = Merge::without_output(comparison.regions),
        }
        self.revision += 1;
        Ok(())
    }


    /// A hand edit of D. The regions it touches become one edited region; several are joined, and `engine` classifies the result.
    pub fn edit_output(&mut self, edit: &OutputEdit, engine: &dyn ComparisonEngine) -> Result<Change, CoreError> {
        self.check_base(&edit.base)?;
        let output = self.output.as_ref().ok_or(CoreError::NotAMerge)?;
        let range = edit.range.clone();
        let fits = range.start <= range.end && output.text.is_char_boundary(range.start) && output.text.is_char_boundary(range.end) && range.end <= output.text.len();
        if !fits {
            return Err(CoreError::InvalidRange { start: range.start, end: range.end });
        }
        let inserted = text::normalize_to_lf(&edit.text);
        if self.merge.regions().is_empty() {
            let region = self.empty_region();
            let state = RegionState { merge: RegionMerge { choice: Choice::Unresolved, edited: true, output_lines: merge::line_count(&inserted) }, text_len: inserted.len() };
            return Ok(self.apply(range, inserted, MergeStep { at: 0, before: Vec::new(), after: vec![(region, state)] }));
        }
        let touched = self.merge.touched(range.clone(), &inserted, &output.text);
        let offsets = self.merge.offsets();
        let span = offsets[touched.start]..offsets[touched.end];
        let mut region_text = output.text[span.start..range.start].to_string();
        region_text.push_str(&inserted);
        region_text.push_str(&output.text[range.end..span.end]);
        let before = self.merge.entries(touched.clone());
        let (region, choice) = match before.as_slice() {
            [(region, state)] => (region.clone(), state.merge.choice.clone()),
            _ => (merge::join(&self.merge.regions()[touched.clone()], &source_lines(&self.sources), engine)?, Choice::Unresolved),
        };
        let state = RegionState { merge: RegionMerge { choice, edited: true, output_lines: merge::line_count(&region_text) }, text_len: region_text.len() };
        Ok(self.apply(range, inserted, MergeStep { at: touched.start, before, after: vec![(region, state)] }))
    }


    /// Sets the choice of one region and drops its hand edits.
    pub fn set_choice(&mut self, base: &Revision, index: usize, choice: Choice) -> Result<Change, CoreError> {
        self.check_base(base)?;
        self.output.as_ref().ok_or(CoreError::NotAMerge)?;
        merge::validate_choice(&choice, self.sources.len())?;
        let count = self.merge.regions().len();
        if index >= count {
            return Err(CoreError::NoSuchRegion { index, count });
        }
        let offsets = self.merge.offsets();
        let region = self.merge.regions()[index].clone();
        let text = merge::render(&region, &choice, &source_lines(&self.sources));
        let state = RegionState { merge: RegionMerge { choice, edited: false, output_lines: merge::line_count(&text) }, text_len: text.len() };
        let step = MergeStep { at: index, before: self.merge.entries(index..index + 1), after: vec![(region, state)] };
        Ok(self.apply(offsets[index]..offsets[index + 1], text, step))
    }


    /// Chooses the hunk of `document` for every unresolved region, as one undo step. `Ok(None)` when no region is unresolved.
    pub fn choose_for_unresolved(&mut self, base: &Revision, document: Document) -> Result<Option<Change>, CoreError> {
        self.check_base(base)?;
        let output = self.output.as_ref().ok_or(CoreError::NotAMerge)?;
        let choice = Choice::Hunks(vec![document]);
        merge::validate_choice(&choice, self.sources.len())?;
        let unresolved: Vec<usize> = self.merge
            .states()
            .iter()
            .enumerate()
            .filter(|(_, state)| state.merge.is_unresolved())
            .map(|(index, _)| index)
            .collect();
        let (Some(first), Some(last)) = (unresolved.first().cloned(), unresolved.last().cloned()) else {
            return Ok(None);
        };
        let offsets = self.merge.offsets();
        let lines = source_lines(&self.sources);
        let before = self.merge.entries(first..last + 1);
        let mut text = String::new();
        let mut after = Vec::with_capacity(before.len());
        for (offset, (region, state)) in before.iter().enumerate() {
            let index = first + offset;
            if state.merge.is_unresolved() {
                let rendered = merge::render(region, &choice, &lines);
                after.push((region.clone(), RegionState { merge: RegionMerge { choice: choice.clone(), edited: false, output_lines: merge::line_count(&rendered) }, text_len: rendered.len() }));
                text.push_str(&rendered);
            } else {
                after.push((region.clone(), state.clone()));
                text.push_str(&output.text[offsets[index]..offsets[index + 1]]);
            }
        }
        Ok(Some(self.apply(offsets[first]..offsets[last + 1], text, MergeStep { at: first, before, after })))
    }


    /// `Ok(None)` when there is nothing to undo.
    pub fn undo(&mut self) -> Result<Option<Change>, CoreError> {
        let output = self.output.as_mut().ok_or(CoreError::NotAMerge)?;
        let Some(step) = output.history.undo().cloned() else {
            return Ok(None);
        };
        let (range, text) = step.backward();
        let text = text.to_string();
        let data = step.data;
        let added = data.before.len();
        let removed = data.after.len();
        self.merge.splice(data.at.clone(), removed.clone(), data.before);
        Ok(Some(self.replace_output(range, text, data.at, removed, added)))
    }


    /// `Ok(None)` when there is nothing to redo.
    pub fn redo(&mut self) -> Result<Option<Change>, CoreError> {
        let output = self.output.as_mut().ok_or(CoreError::NotAMerge)?;
        let Some(step) = output.history.redo().cloned() else {
            return Ok(None);
        };
        let (range, text) = step.forward();
        let text = text.to_string();
        let data = step.data;
        let added = data.after.len();
        let removed = data.before.len();
        self.merge.splice(data.at.clone(), removed.clone(), data.after);
        Ok(Some(self.replace_output(range, text, data.at, removed, added)))
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


    /// Writes D with the EOL style and BOM of A, the output reference. Refused while unresolved conflicts remain.
    fn write_output(&mut self, path: &Path) -> Result<Change, CoreError> {
        if path.is_dir() {
            return Err(CoreError::Directory { path: path.to_path_buf() });
        }
        self.output.as_ref().ok_or(CoreError::NotAMerge)?;
        let unresolved = self.merge.unresolved_count();
        if unresolved > 0 {
            return Err(CoreError::UnresolvedConflicts { count: unresolved });
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


    fn check_base(&self, base: &Revision) -> Result<(), CoreError> {
        let current = self.revision();
        if *base != current {
            return Err(CoreError::StaleRevision { base: base.clone(), current });
        }
        Ok(())
    }


    /// A region without lines, for typing into a merge of empty documents.
    fn empty_region(&self) -> Region {
        let status = match self.sources.len() {
            2 => RegionStatus::Diff(HunkChange::Unchanged),
            _ => RegionStatus::Merge(MergeStatus::Unchanged),
        };
        Region { hunks: vec![0..0; self.sources.len()], status, rows: Vec::new() }
    }


    /// Makes a merge op: replaces `range` of D with `inserted` and the regions of `step.before` with `step.after`, and records it for undo.
    fn apply(&mut self, range: Range<usize>, inserted: String, step: MergeStep) -> Change {
        let at = step.at.clone();
        let removed = step.before.len();
        let added = step.after.len();
        self.merge.splice(at.clone(), removed.clone(), step.after.clone());
        if let Some(output) = self.output.as_mut() {
            let removed_text = output.text[range.clone()].to_string();
            output.history.record(Step { start: range.start, removed: removed_text, inserted: inserted.clone(), data: step });
        }
        self.replace_output(range, inserted, at, removed, added)
    }


    /// Replaces `range` of D, whose regions are already updated, and reports the change.
    fn replace_output(&mut self, range: Range<usize>, inserted: String, at: usize, removed: usize, added: usize) -> Change {
        let modified = match self.output.as_mut() {
            Some(output) => {
                output.text.replace_range(range.clone(), &inserted);
                output.history.is_modified()
            }
            None => false,
        };
        self.revision += 1;
        Change::OutputEdited { range, text: inserted, modified, merge: self.merge.update(at, removed, added) }
    }
}


fn source_lines(sources: &[Source]) -> Vec<Vec<&str>> {
    sources
        .iter()
        .map(Source::lines)
        .collect()
}


/// Compares the source documents with `engine`. Runs to completion: cancelable worker jobs come later.
fn compare(sources: &[Source], engine: &dyn ComparisonEngine, options: &ComparisonOptions) -> Result<Comparison, CoreError> {
    let lines = source_lines(sources);
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


    /// A 3-way merge of three copies of `text` in `dir`, so D starts as `text` with no conflicts, with the output at `dir/out.txt` (not created).
    fn merge_session(dir: &Path, text: &str) -> Session {
        let a = write(dir, "a.txt", text);
        let b = write(dir, "b.txt", text);
        let c = write(dir, "c.txt", text);
        Session::open(&files(a, b, Some(c), Some(dir.join("out.txt"))), &BuiltinEngine).unwrap()
    }


    /// A 3-way merge of `a`, `b` and `c`, given as lines.
    fn merge_of(dir: &Path, a: &[&str], b: &[&str], c: &[&str]) -> Session {
        let text = |lines: &[&str]| lines.iter().map(|line| format!("{line}\n")).collect::<String>();
        let a = write(dir, "a.txt", &text(a));
        let b = write(dir, "b.txt", &text(b));
        let c = write(dir, "c.txt", &text(c));
        Session::open(&files(a, b, Some(c), Some(dir.join("out.txt"))), &BuiltinEngine).unwrap()
    }


    fn output_text(session: &Session) -> String {
        session.snapshot().output.unwrap().text
    }


    /// D's text is the regions' texts in order, and each region's line count matches its text.
    fn assert_consistent(session: &Session) {
        let snapshot = session.snapshot();
        let text = snapshot.output.unwrap().text;
        let offsets = session.merge.offsets();
        assert_eq!(offsets.last().cloned(), Some(text.len()), "regions cover D");
        for (index, state) in session.merge.states().iter().enumerate() {
            assert_eq!(merge::line_count(&text[offsets[index]..offsets[index + 1]]), state.merge.output_lines, "line count of region {index}");
        }
        assert_eq!(snapshot.merge.len(), snapshot.comparison.regions.len());
    }


    fn text_part(change: &Change) -> (std::ops::Range<usize>, String, bool) {
        match change {
            Change::OutputEdited { range, text, modified, .. } => (range.clone(), text.clone(), modified.clone()),
            other => panic!("expected an output change, got {other:?}"),
        }
    }


    fn choices(session: &Session) -> Vec<Choice> {
        session
            .snapshot()
            .merge
            .iter()
            .map(|merge| merge.choice.clone())
            .collect()
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
    fn load_source_recomputes_comparison_and_rebuilds_d() {
        let dir = temp_dir("load-recomputes");
        let mut session = merge_of(&dir, &["x"], &["x"], &["y"]);
        assert_eq!(statuses(&session.snapshot().comparison), [RegionStatus::Merge(MergeStatus::ChangedInC)]);
        session.edit_output(&edit(&session, 0..0, "typed\n"), &BuiltinEngine).unwrap();
        let new_c = write(&dir, "c2.txt", "x\n");
        session.load_source(Document::C, &new_c, &BuiltinEngine).unwrap();
        let snapshot = session.snapshot();
        assert_eq!(statuses(&snapshot.comparison), [RegionStatus::Merge(MergeStatus::Unchanged)]);
        assert_eq!(snapshot.documents[2].path, new_c);
        let output = snapshot.output.unwrap();
        assert_eq!(output.text, "x\n");
        assert!(output.modified);
        assert_eq!(session.undo().unwrap(), None);
        assert_consistent(&session);
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
    fn merge_without_output_has_unnamed_output() {
        let dir = temp_dir("in-memory");
        let a = write(&dir, "a.txt", "a");
        let b = write(&dir, "b.txt", "a");
        let c = write(&dir, "c.txt", "a");
        let output = Session::open(&files(a, b, Some(c), None), &BuiltinEngine).unwrap().snapshot().output.unwrap();
        assert_eq!(output, OutputDocument { path: None, text: "a\n".to_string(), modified: false });
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
    fn existing_output_file_is_ignored_and_d_is_the_merge() {
        let dir = temp_dir("existing-output");
        let a = write(&dir, "a.txt", "a\r\nb\r\n");
        let b = write(&dir, "b.txt", "a\r\nB\r\n");
        let c = write(&dir, "c.txt", "a\r\nb\r\n");
        let out = write(&dir, "out.txt", "old merge result\n");
        let output = Session::open(&files(a, b, Some(c), Some(out)), &BuiltinEngine).unwrap().snapshot().output.unwrap();
        assert_eq!(output.text, "a\nB\n");
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
        session.load_source(Document::B, &new_b, &BuiltinEngine).unwrap();
        assert_eq!(session.revision(), Revision(2));
        let text = &session.snapshot().documents[1];
        assert_eq!(text.document, Document::B);
        assert_eq!(text.path, new_b);
        assert_eq!(text.line(0), "new b");
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
        let mut session = merge_session(&dir, "");
        session.edit_output(&edit(&session, 0..0, "héllo\r\nworld"), &BuiltinEngine).unwrap();
        let change = session.edit_output(&edit(&session, 1..3, "e"), &BuiltinEngine).unwrap();
        assert_eq!(text_part(&change), (1..3, "e".to_string(), true));
        let output = session.snapshot().output.unwrap();
        assert_eq!(output.text, "hello\nworld");
        assert!(output.modified);
        assert_eq!(session.revision(), Revision(3));
        assert_consistent(&session);
    }


    #[test]
    fn edit_output_rejects_stale_revision_and_bad_ranges() {
        let dir = temp_dir("edit-invalid");
        let mut session = merge_session(&dir, "");
        session.edit_output(&edit(&session, 0..0, "é"), &BuiltinEngine).unwrap();
        let stale = OutputEdit { base: Revision(1), range: 0..0, text: "x".to_string() };
        assert!(matches!(session.edit_output(&stale, &BuiltinEngine), Err(CoreError::StaleRevision { .. })));
        assert!(matches!(session.edit_output(&edit(&session, 1..1, "x"), &BuiltinEngine), Err(CoreError::InvalidRange { .. })));
        assert!(matches!(session.edit_output(&edit(&session, 0..9, "x"), &BuiltinEngine), Err(CoreError::InvalidRange { .. })));
        let reversed = edit(&session, 2..0, "x");
        assert!(matches!(session.edit_output(&reversed, &BuiltinEngine), Err(CoreError::InvalidRange { .. })));
        assert_eq!(session.snapshot().output.unwrap().text, "é");
    }


    #[test]
    fn undo_and_redo_replay_edits_and_track_modified() {
        let dir = temp_dir("undo");
        let mut session = merge_session(&dir, "");
        session.edit_output(&edit(&session, 0..0, "a"), &BuiltinEngine).unwrap();
        session.edit_output(&edit(&session, 1..1, "b"), &BuiltinEngine).unwrap();
        session.edit_output(&edit(&session, 2..2, "\n"), &BuiltinEngine).unwrap();
        assert_eq!(text_part(&session.undo().unwrap().unwrap()), (2..3, String::new(), true));
        assert_eq!(text_part(&session.undo().unwrap().unwrap()), (0..2, String::new(), false));
        assert_eq!(session.undo().unwrap(), None);
        assert_eq!(session.snapshot().output.unwrap().text, "");
        assert!(!session.snapshot().output.unwrap().modified);
        assert!(session.snapshot().comparison.regions.is_empty());
        assert_eq!(text_part(&session.redo().unwrap().unwrap()), (0..0, "ab".to_string(), true));
        assert_consistent(&session);
        assert_eq!(session.snapshot().output.unwrap().text, "ab");
        assert_eq!(session.revision(), Revision(7));
    }


    #[test]
    fn undo_back_to_saved_text_is_unmodified() {
        let dir = temp_dir("undo-saved");
        let mut session = merge_session(&dir, "");
        session.edit_output(&edit(&session, 0..0, "x"), &BuiltinEngine).unwrap();
        session.save().unwrap();
        session.edit_output(&edit(&session, 1..1, "y"), &BuiltinEngine).unwrap();
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
        assert!(matches!(session.edit_output(&edit(&session, 0..0, "x"), &BuiltinEngine), Err(CoreError::NotAMerge)));
        assert!(matches!(session.save(), Err(CoreError::NotAMerge)));
        assert!(matches!(session.set_output_path(None), Err(CoreError::NotAMerge)));
        assert!(matches!(session.undo(), Err(CoreError::NotAMerge)));
    }


    #[test]
    fn save_writes_with_eol_and_bom_of_a() {
        let dir = temp_dir("save");
        let mut session = merge_session(&dir, "\u{FEFF}one\r\ntwo\r\n");
        session.edit_output(&edit(&session, 0..0, "x\ny\n"), &BuiltinEngine).unwrap();
        assert_eq!(session.save().unwrap(), Change::OutputSaved(dir.join("out.txt")));
        assert_eq!(fs::read(dir.join("out.txt")).unwrap(), "\u{FEFF}x\r\ny\r\none\r\ntwo\r\n".as_bytes());
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
        let mut session = merge_session(&dir, "");
        session.edit_output(&edit(&session, 0..0, "z\n"), &BuiltinEngine).unwrap();
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
        session.edit_output(&edit(&session, 0..0, "z"), &BuiltinEngine).unwrap();
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


    #[test]
    fn auto_resolve_builds_d_and_leaves_conflicts_unresolved() {
        let dir = temp_dir("auto-resolve");
        let session = merge_of(&dir, &["same", "base", "x", "keep", "a"], &["same", "left", "x", "keep", "b"], &["same", "base", "x", "keep", "c"]);
        assert_eq!(statuses(&session.snapshot().comparison), [RegionStatus::Merge(MergeStatus::Unchanged), RegionStatus::Merge(MergeStatus::ChangedInB), RegionStatus::Merge(MergeStatus::Unchanged), RegionStatus::Merge(MergeStatus::Conflict)]);
        assert_eq!(choices(&session), [Choice::Hunks(vec![Document::A]), Choice::Hunks(vec![Document::B]), Choice::Hunks(vec![Document::A]), Choice::Unresolved]);
        assert_eq!(output_text(&session), "same\nleft\nx\nkeep\n");
        assert_eq!(session.unresolved_count(), 1);
        assert_consistent(&session);
    }


    #[test]
    fn two_way_merge_leaves_differences_unresolved() {
        let dir = temp_dir("two-way-auto");
        let a = write(&dir, "a.txt", "x\ny\n");
        let b = write(&dir, "b.txt", "x\nz\n");
        let session = Session::open(&files(a, b, None, Some(dir.join("out.txt"))), &BuiltinEngine).unwrap();
        assert_eq!(choices(&session), [Choice::Hunks(vec![Document::A]), Choice::Unresolved]);
        assert_eq!(output_text(&session), "x\n");
    }


    #[test]
    fn set_choice_rebuilds_the_region_and_undoes() {
        let dir = temp_dir("set-choice");
        let mut session = merge_of(&dir, &["s", "a", "t"], &["s", "b", "t"], &["s", "c", "t"]);
        let change = session.set_choice(&session.revision(), 1, Choice::Hunks(vec![Document::C, Document::B])).unwrap();
        assert_eq!(text_part(&change), (2..2, "c\nb\n".to_string(), true));
        let Change::OutputEdited { merge: update, .. } = &change else {
            panic!("expected an output change");
        };
        assert_eq!((update.at, update.removed, update.regions.len()), (1, 1, 1));
        assert_eq!(update.merge[0], RegionMerge { choice: Choice::Hunks(vec![Document::C, Document::B]), edited: false, output_lines: 2 });
        assert_eq!(output_text(&session), "s\nc\nb\nt\n");
        assert_eq!(session.unresolved_count(), 0);
        assert_consistent(&session);
        session.undo().unwrap();
        assert_eq!(output_text(&session), "s\nt\n");
        assert_eq!(choices(&session)[1], Choice::Unresolved);
        session.redo().unwrap();
        assert_eq!(output_text(&session), "s\nc\nb\nt\n");
        assert_consistent(&session);
    }


    #[test]
    fn set_choice_can_choose_nothing_and_unresolve() {
        let dir = temp_dir("set-choice-empty");
        let mut session = merge_of(&dir, &["s", "a"], &["s", "b"], &["s", "c"]);
        session.set_choice(&session.revision(), 1, Choice::Hunks(Vec::new())).unwrap();
        assert_eq!(output_text(&session), "s\n");
        assert_eq!(session.unresolved_count(), 0);
        session.set_choice(&session.revision(), 1, Choice::Unresolved).unwrap();
        assert_eq!(session.unresolved_count(), 1);
    }


    #[test]
    fn set_choice_rejects_bad_requests() {
        let dir = temp_dir("set-choice-bad");
        let mut session = merge_of(&dir, &["a"], &["b"], &["c"]);
        let base = session.revision();
        assert!(matches!(session.set_choice(&Revision(9), 0, Choice::Unresolved), Err(CoreError::StaleRevision { .. })));
        assert!(matches!(session.set_choice(&base, 5, Choice::Unresolved), Err(CoreError::NoSuchRegion { index: 5, count: 1 })));
        assert!(matches!(session.set_choice(&base, 0, Choice::Hunks(vec![Document::D])), Err(CoreError::NoSuchDocument { .. })));
        assert!(matches!(session.set_choice(&base, 0, Choice::Hunks(vec![Document::A, Document::A])), Err(CoreError::InvalidChoice { .. })));
        assert_eq!(session.revision(), base);
    }


    #[test]
    fn choose_for_unresolved_is_one_undo_step() {
        let dir = temp_dir("choose-all");
        let mut session = merge_of(&dir, &["a1", "s", "x", "s2", "a3"], &["b1", "s", "X", "s2", "b3"], &["c1", "s", "x", "s2", "c3"]);
        assert_eq!(session.unresolved_count(), 2);
        let change = session.choose_for_unresolved(&session.revision(), Document::C).unwrap().unwrap();
        assert_eq!(text_part(&change).0, 0..7);
        assert_eq!(output_text(&session), "c1\ns\nX\ns2\nc3\n");
        assert_eq!(session.unresolved_count(), 0);
        assert_consistent(&session);
        assert_eq!(session.choose_for_unresolved(&session.revision(), Document::B).unwrap(), None);
        session.undo().unwrap();
        assert_eq!(session.unresolved_count(), 2);
        assert_eq!(output_text(&session), "s\nX\ns2\n");
        assert_consistent(&session);
    }


    #[test]
    fn edit_inside_a_region_marks_it_edited() {
        let dir = temp_dir("edit-region");
        let mut session = merge_of(&dir, &["s", "a", "t"], &["s", "b", "t"], &["s", "a", "t"]);
        assert_eq!(output_text(&session), "s\nb\nt\n");
        let change = session.edit_output(&edit(&session, 3..3, "!"), &BuiltinEngine).unwrap();
        let Change::OutputEdited { merge: update, .. } = &change else {
            panic!("expected an output change");
        };
        assert_eq!((update.at, update.removed), (1, 1));
        assert_eq!(update.merge[0], RegionMerge { choice: Choice::Hunks(vec![Document::B]), edited: true, output_lines: 1 });
        assert_eq!(output_text(&session), "s\nb!\nt\n");
        assert_consistent(&session);
    }


    #[test]
    fn editing_a_chosen_hunk_and_choosing_again_drops_the_edit() {
        let dir = temp_dir("edit-resolves");
        let mut session = merge_of(&dir, &["s", "a"], &["s", "b"], &["s", "c"]);
        session.set_choice(&session.revision(), 1, Choice::Hunks(vec![Document::B])).unwrap();
        session.edit_output(&edit(&session, 2..3, "mine"), &BuiltinEngine).unwrap();
        assert_eq!(output_text(&session), "s\nmine\n");
        assert!(session.snapshot().merge[1].edited);
        session.set_choice(&session.revision(), 1, Choice::Hunks(vec![Document::C])).unwrap();
        assert_eq!(output_text(&session), "s\nc\n");
        assert!(!session.snapshot().merge[1].edited);
        assert_consistent(&session);
    }


    #[test]
    fn typing_next_to_an_empty_conflict_stays_out_of_it() {
        let dir = temp_dir("edit-next-to-conflict");
        let mut session = merge_of(&dir, &["s", "a"], &["s", "b"], &["s", "c"]);
        session.edit_output(&edit(&session, 2..2, "more\n"), &BuiltinEngine).unwrap();
        assert_eq!(output_text(&session), "s\nmore\n");
        assert_eq!(session.unresolved_count(), 1);
        assert_consistent(&session);
    }


    #[test]
    fn edit_across_regions_joins_them() {
        let dir = temp_dir("edit-join");
        let mut session = merge_of(&dir, &["s", "a", "t"], &["s", "b", "t"], &["s", "a", "t"]);
        assert_eq!(session.snapshot().comparison.regions.len(), 3);
        session.edit_output(&edit(&session, 1..2, ""), &BuiltinEngine).unwrap();
        let snapshot = session.snapshot();
        assert_eq!(snapshot.output.as_ref().unwrap().text, "sb\nt\n");
        assert_eq!(snapshot.comparison.regions.len(), 2);
        assert_eq!(snapshot.comparison.regions[0].hunks, [0..2, 0..2, 0..2]);
        assert_eq!(snapshot.comparison.regions[0].status, RegionStatus::Merge(MergeStatus::ChangedInB));
        assert_eq!(snapshot.merge[0], RegionMerge { choice: Choice::Unresolved, edited: true, output_lines: 1 });
        assert_consistent(&session);
        session.undo().unwrap();
        assert_eq!(session.snapshot().comparison.regions.len(), 3);
        assert_eq!(output_text(&session), "s\nb\nt\n");
        assert_consistent(&session);
    }


    #[test]
    fn typing_in_one_region_is_one_undo_step() {
        let dir = temp_dir("typing-step");
        let mut session = merge_session(&dir, "s\n");
        for (offset, letter) in ["a", "b", "c"].iter().enumerate() {
            session.edit_output(&edit(&session, 1 + offset..1 + offset, letter), &BuiltinEngine).unwrap();
        }
        assert_eq!(output_text(&session), "sabc\n");
        session.undo().unwrap();
        assert_eq!(output_text(&session), "s\n");
        assert!(!session.snapshot().merge[0].edited);
    }


    #[test]
    fn save_is_refused_while_conflicts_remain() {
        let dir = temp_dir("save-unresolved");
        let mut session = merge_of(&dir, &["a"], &["b"], &["c"]);
        assert!(matches!(session.save(), Err(CoreError::UnresolvedConflicts { count: 1 })));
        assert!(matches!(session.save_as(&dir.join("other.txt")), Err(CoreError::UnresolvedConflicts { count: 1 })));
        assert!(!dir.join("out.txt").exists());
        session.set_choice(&session.revision(), 0, Choice::Hunks(vec![Document::B])).unwrap();
        session.save().unwrap();
        assert_eq!(fs::read_to_string(dir.join("out.txt")).unwrap(), "b\n");
    }


    #[derive(Debug, Clone)]
    enum Op {
        /// Replaces `len` bytes at `at` (both scaled into D) with `text`.
        Edit { at: usize, len: usize, text: &'static str },
        Choose { region: usize, documents: Vec<Document> },
        ChooseAll(Document),
        Undo,
        Redo,
    }


    fn op() -> impl proptest::strategy::Strategy<Value = Op> {
        use proptest::prelude::*;
        let documents = proptest::sample::subsequence(vec![Document::A, Document::B, Document::C], 0..=3).prop_shuffle();
        prop_oneof![
            (any::<usize>(), 0usize..6, proptest::sample::select(vec!["", "x", "\n", "y\n", "z\nw"])).prop_map(|(at, len, text)| Op::Edit { at, len, text }),
            (any::<usize>(), documents).prop_map(|(region, documents)| Op::Choose { region, documents }),
            proptest::sample::select(vec![Document::A, Document::B, Document::C]).prop_map(Op::ChooseAll),
            Just(Op::Undo),
            Just(Op::Redo),
        ]
    }


    fn apply_op(session: &mut Session, op: &Op) {
        let base = session.revision();
        match op {
            Op::Edit { at, len, text } => {
                let output = output_text(session);
                let mut start = at % (output.len() + 1);
                while !output.is_char_boundary(start) {
                    start -= 1;
                }
                let mut end = (start + len).min(output.len());
                while !output.is_char_boundary(end) {
                    end -= 1;
                }
                session.edit_output(&edit(session, start..end, text), &BuiltinEngine).unwrap();
            }
            Op::Choose { region, documents } => {
                let count = session.snapshot().comparison.regions.len();
                if count > 0 {
                    session.set_choice(&base, region % count, Choice::Hunks(documents.clone())).unwrap();
                }
            }
            Op::ChooseAll(document) => {
                session.choose_for_unresolved(&base, document.clone()).unwrap();
            }
            Op::Undo => {
                session.undo().unwrap();
            }
            Op::Redo => {
                session.redo().unwrap();
            }
        }
    }


    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]
        #[test]
        fn merge_ops_keep_d_consistent_and_undo_restores_the_start(
            lines in proptest::collection::vec(proptest::collection::vec(proptest::sample::select(vec!["a", "b", "c"]), 0..6), 3),
            ops in proptest::collection::vec(op(), 0..25),
        ) {
            let dir = temp_dir("merge-ops");
            let as_refs: Vec<Vec<&str>> = lines.iter().map(|document| document.clone()).collect();
            let mut session = merge_of(&dir, &as_refs[0], &as_refs[1], &as_refs[2]);
            let start = session.snapshot();
            for op in &ops {
                apply_op(&mut session, op);
                assert_consistent(&session);
            }
            while session.undo().unwrap().is_some() {
                assert_consistent(&session);
            }
            let end = session.snapshot();
            proptest::prop_assert_eq!(end.output, start.output);
            proptest::prop_assert_eq!(end.comparison, start.comparison);
            proptest::prop_assert_eq!(end.merge, start.merge);
        }
    }
}
