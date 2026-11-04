use std::ops::Range;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::engine::{CharRun, Comparison, ComparisonOptions, EngineCapabilities, Region};


#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Document {
    A,
    B,
    C,
    D,
}


impl Document {
    pub const ALL: [Document; 4] = [Document::A, Document::B, Document::C, Document::D];


    pub fn letter(&self) -> &'static str {
        match self {
            Document::A => "A",
            Document::B => "B",
            Document::C => "C",
            Document::D => "D",
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RequestId(pub u64);


#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Revision(pub u64);


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: RequestId,
    pub command: Command,
}


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    Open(OpenFiles),
    /// Reloads A, B or C from a (possibly new) path and recomputes the comparison. D is rebuilt from the new regions and its undo history starts over; the host answers with a new snapshot.
    LoadSource { document: Document, path: PathBuf },
    EditOutput(OutputEdit),
    /// Sets the choice of region `region`, dropping its hand edits. `base` is the revision the region index refers to.
    SetChoice { base: Revision, region: usize, choice: Choice },
    /// Chooses the hunk of `document` for every unresolved region. One undo step; does nothing when no region is unresolved.
    ChooseForUnresolved { base: Revision, document: Document },
    /// Changes where D is saved, without writing it. `None` makes D an unnamed in-memory document.
    SetOutputPath(Option<PathBuf>),
    Save,
    SaveAs(PathBuf),
    /// Undoes the last undo step of D. Does nothing when there is none.
    Undo,
    /// Redoes the last undone step of D. Does nothing when there is none.
    Redo,
    /// Asks the host for a fresh snapshot.
    Resync,
    /// Recomputes the comparison with these options. Like a reload, D is rebuilt from the new regions and its undo history starts over; the host answers with a new snapshot. On failure the session is unchanged.
    SetComparisonOptions(ComparisonOptions),
    /// Moves the current region from region `from` (the current region when `None`) to the next or previous difference or conflict. Nothing changes when there is none. `base` is the revision `from` refers to.
    Navigate { base: Revision, from: Option<usize>, to: Navigation },
    /// Makes region `region` current, or no region. `base` is the revision the index refers to.
    SetCurrentRegion { base: Revision, region: Option<usize> },
}


/// Where a navigate command moves the current region. A difference is a region whose hunks are not all equal; a conflict is one that needs a decision before auto-resolve can merge it, whether or not D has resolved it since.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Navigation {
    PreviousDifference,
    NextDifference,
    PreviousConflict,
    NextConflict,
}


/// A session with C or an output is a merge and has D; A and B alone are a 2-way diff.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenFiles {
    pub a: PathBuf,
    pub b: PathBuf,
    pub c: Option<PathBuf>,
    pub output: Option<PathBuf>,
    pub labels: Labels,
}


impl OpenFiles {
    pub fn is_merge(&self) -> bool {
        self.c.is_some() || self.output.is_some()
    }
}


/// Display labels of the source documents, shown instead of nothing next to their paths.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Labels {
    pub a: Option<String>,
    pub b: Option<String>,
    pub c: Option<String>,
}


impl Labels {
    pub fn get(&self, document: Document) -> Option<&str> {
        match document {
            Document::A => self.a.as_deref(),
            Document::B => self.b.as_deref(),
            Document::C => self.c.as_deref(),
            Document::D => None,
        }
    }
}


/// Replaces a byte range of D's text. `base` is the revision the edit was made on. `region`, when given, is a region without lines in D that the edit goes to: an insertion of whole lines at the region's place, which a position alone can't tell apart from its neighbors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputEdit {
    pub base: Revision,
    pub range: Range<usize>,
    pub text: String,
    #[serde(default)]
    pub region: Option<usize>,
}


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub id: RequestId,
    pub result: Result<(), ProtocolError>,
}


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    Snapshot(Snapshot),
    Delta(Delta),
    CurrentRegion(CurrentRegion),
}


/// The current region changed by navigation. It does not change the revision: `revision` is the one `region` refers to. Changes of D also make their region current, without this event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentRegion {
    pub revision: Revision,
    pub region: Option<usize>,
    pub cause: Option<RequestId>,
}


/// The change made by one revision. `cause` is the request that made it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delta {
    pub revision: Revision,
    pub cause: Option<RequestId>,
    pub change: Change,
}


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Change {
    /// D's text and the regions behind it changed: a hand edit, a choice, an undo or a redo. The first region of `merge` becomes the current region. `modified` says whether D now differs from what was loaded or last saved; undo can bring it back.
    OutputEdited { range: Range<usize>, text: String, modified: bool, merge: MergeUpdate },
    OutputPathChanged(Option<PathBuf>),
    OutputSaved(PathBuf),
}


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostMessage {
    Response(Response),
    Event(Event),
}


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub revision: Revision,
    pub documents: Vec<DocumentText>,
    /// D; present for merges.
    pub output: Option<OutputDocument>,
    pub comparison: Comparison,
    /// The comparison engine that computed `comparison`.
    pub engine: EngineCapabilities,
    /// Per region of `comparison`, its merge state; empty without D.
    pub merge: Vec<RegionMerge>,
    /// The options `comparison` was computed with.
    pub options: ComparisonOptions,
    /// The region navigation is at, if any.
    pub current_region: Option<usize>,
    /// Per source document, in `documents` order, the character-level diff of its lines. Rows depend only on the comparison, so D's edits never change them.
    pub char_diffs: Vec<LineCharRuns>,
}


/// Per line of one source document, the char runs of the line in its row; empty for a line whose row has no differences.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineCharRuns {
    pub lines: Vec<Vec<CharRun>>,
}


impl LineCharRuns {
    /// Empty for a line out of range.
    pub fn line(&self, line: usize) -> &[CharRun] {
        self.lines
            .get(line)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}


/// Which hunks of a region D takes its lines from, in this order; or none yet. An empty list is a deliberate choice of no lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Choice {
    Unresolved,
    Hunks(Vec<Document>),
}


/// The merge state of one region.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionMerge {
    pub choice: Choice,
    /// D's lines of this region were changed by hand; they replace the lines of the choice.
    pub edited: bool,
    /// How many lines of D this region contributes.
    pub output_lines: usize,
}


impl RegionMerge {
    /// An unresolved conflict: no choice and no hand edits. D can't be saved while one exists.
    pub fn is_unresolved(&self) -> bool {
        self.choice == Choice::Unresolved && !self.edited
    }
}


/// Regions `at..at + removed` were replaced by `regions`, with their merge states in `merge`. Usually the same number of regions; fewer when an edit joined them, more when an undo split them again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeUpdate {
    pub at: usize,
    pub removed: usize,
    pub regions: Vec<Region>,
    pub merge: Vec<RegionMerge>,
}


/// D's text uses `\n` line ends; the EOL style is applied when it is saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputDocument {
    pub path: Option<PathBuf>,
    pub text: String,
    /// Changed since it was loaded or last saved.
    pub modified: bool,
}


#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EolStyle {
    #[default]
    Lf,
    CrLf,
    Cr,
}


impl EolStyle {
    pub fn terminator(&self) -> &'static str {
        match self {
            EolStyle::Lf => "\n",
            EolStyle::CrLf => "\r\n",
            EolStyle::Cr => "\r",
        }
    }
}


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentText {
    pub document: Document,
    pub path: PathBuf,
    pub label: Option<String>,
    pub eol: EolStyle,
    pub text: String,
    /// Byte ranges into `text`, without line terminators.
    pub lines: Vec<Range<usize>>,
}


impl DocumentText {
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }


    /// Returns an empty string for an index or range that doesn't fit the text.
    pub fn line(&self, index: usize) -> &str {
        self.lines
            .get(index)
            .and_then(|range| self.text.get(range.clone()))
            .unwrap_or("")
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorKind {
    Io,
    InvalidRequest,
    Canceled,
    /// A comparison engine failed or returned a result that broke the rules.
    Engine,
}


#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[error("{message}")]
pub struct ProtocolError {
    pub kind: ErrorKind,
    pub message: String,
}


#[cfg(test)]
mod tests {
    use super::*;


    fn document_text(text: &str, lines: Vec<Range<usize>>) -> DocumentText {
        DocumentText { document: Document::A, path: PathBuf::from("a.txt"), label: None, eol: EolStyle::Lf, text: text.to_string(), lines }
    }


    #[test]
    fn line_returns_text_of_range() {
        let text = document_text("one\ntwo", vec![0..3, 4..7]);
        assert_eq!(text.line_count(), 2);
        assert_eq!(text.line(0), "one");
        assert_eq!(text.line(1), "two");
    }


    #[test]
    fn line_out_of_bounds_is_empty() {
        let text = document_text("one", vec![0..3, 2..10]);
        assert_eq!(text.line(1), "");
        assert_eq!(text.line(5), "");
    }


    #[test]
    fn merge_needs_c_or_output() {
        let mut files = OpenFiles { a: PathBuf::from("a"), b: PathBuf::from("b"), ..OpenFiles::default() };
        assert!(!files.is_merge());
        files.output = Some(PathBuf::from("d"));
        assert!(files.is_merge());
        files.output = None;
        files.c = Some(PathBuf::from("c"));
        assert!(files.is_merge());
    }


    #[test]
    fn labels_by_document() {
        let labels = Labels { a: Some("Base".to_string()), b: None, c: Some("Remote".to_string()) };
        assert_eq!(labels.get(Document::A), Some("Base"));
        assert_eq!(labels.get(Document::B), None);
        assert_eq!(labels.get(Document::C), Some("Remote"));
        assert_eq!(labels.get(Document::D), None);
    }


    #[test]
    fn eol_terminators() {
        assert_eq!(EolStyle::Lf.terminator(), "\n");
        assert_eq!(EolStyle::CrLf.terminator(), "\r\n");
        assert_eq!(EolStyle::Cr.terminator(), "\r");
    }


    #[test]
    fn unresolved_means_no_choice_and_no_edits() {
        let state = |choice, edited| RegionMerge { choice, edited, output_lines: 0 };
        assert!(state(Choice::Unresolved, false).is_unresolved());
        assert!(!state(Choice::Unresolved, true).is_unresolved());
        assert!(!state(Choice::Hunks(Vec::new()), false).is_unresolved());
        assert!(!state(Choice::Hunks(vec![Document::B]), false).is_unresolved());
    }


    #[test]
    fn document_letters() {
        let letters: Vec<&str> = Document::ALL
            .iter()
            .map(|document| document.letter())
            .collect();
        assert_eq!(letters, ["A", "B", "C", "D"]);
    }
}
