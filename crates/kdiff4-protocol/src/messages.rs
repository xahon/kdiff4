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
    LoadSource {
        document: Document,
        path: PathBuf,
    },
    EditOutput(OutputEdit),
    /// Sets the choice of region `region`, dropping its hand edits. `base` is the revision the region index refers to.
    SetChoice {
        base: Revision,
        region: usize,
        choice: Choice,
    },
    /// Chooses the hunk of `document` for every unresolved region. One undo step; does nothing when no region is unresolved.
    ChooseForUnresolved {
        base: Revision,
        document: Document,
    },
    /// Changes where D is saved, without writing it. `None` makes D an unnamed in-memory document.
    SetOutputPath(Option<PathBuf>),
    Save,
    SaveAs(PathBuf),
    /// Chooses the line ending style D is saved with, replacing what the sources gave. Stays until a source is loaded again or the session is reopened.
    SetOutputEol(EolStyle),
    /// Undoes the last undo step of D. Does nothing when there is none.
    Undo,
    /// Redoes the last undone step of D. Does nothing when there is none.
    Redo,
    /// Asks the host for a fresh snapshot.
    Resync,
    /// Recomputes the comparison with these options. Like a reload, D is rebuilt from the new regions and its undo history starts over; the host answers with a new snapshot. On failure the session is unchanged.
    SetComparisonOptions(ComparisonOptions),
    /// Moves the current region from region `from` (the current region when `None`) to the next or previous difference or conflict. Nothing changes when there is none. `base` is the revision `from` refers to. With `include_alignments`, a difference move also stops at the first region of every applied alignment, whether or not its hunks differ.
    Navigate {
        base: Revision,
        from: Option<usize>,
        to: Navigation,
        #[serde(default)]
        include_alignments: bool,
    },
    /// Makes region `region` current, or no region. `base` is the revision the index refers to.
    SetCurrentRegion {
        base: Revision,
        region: Option<usize>,
    },
    /// Marks the lines `lines` of a source document for alignment, under the lowest number that document has not used yet. An empty range is an anchor: the position before that line, which may be the document's line count. Nothing is compared again until `ApplyAlignments`. Rejected when it overlaps another mark of that document, or is an anchor strictly inside one.
    MarkAlignment {
        document: Document,
        lines: Range<usize>,
    },
    /// Removes the mark `number` of a source document.
    RemoveAlignmentMark {
        document: Document,
        number: u32,
    },
    /// Gives the mark `number` of a source document the number `to`. When that document already has a mark `to`, the two swap numbers.
    RenumberAlignmentMark {
        document: Document,
        number: u32,
        to: u32,
    },
    /// Shows the complete alignment `number` at `slot` of the display order of the other alignments, counting from 0 at the top. Where the documents disagree on the order, the documents that don't agree with that place exchange the hunk of the alignment with the one that has its place there. Nothing is compared again until `ApplyAlignments`. Rejected when `number` is not a complete alignment, when `slot` is past the last alignment, or when no document keeps its order at that slot.
    PlaceAlignment {
        number: u32,
        slot: usize,
    },
    /// Removes every mark. The comparison keeps the alignments it was computed with until `ApplyAlignments`.
    ClearAlignmentMarks,
    /// Compares again with the alignments the marks give: every number marked in all source documents. Like a reload, D is rebuilt from the new regions, its choices, edits and undo history are dropped, and the host answers with a new snapshot. Does nothing when the marks give the alignments already applied. Fails with `ErrorKind::NeedsConfirmation` when D holds choices or edits and `confirmed` is not set.
    ApplyAlignments {
        confirmed: bool,
    },
    /// Finds the functions, methods, types and (when the options ask for them, and always with `within`) blocks of the source documents of `language` (a name the language selector shows) and pairs them across the documents; the entities inside a paired entity are paired in turn. With `within`, the number of an applied alignment, only what lies inside that alignment is paired. The host answers with `Event::HeuristicProposal` and changes nothing. Rejected for a language the extractor can't parse, and for a `within` that is not an applied alignment.
    ProposeHeuristicAlignments {
        language: String,
        options: HeuristicOptions,
        #[serde(default)]
        within: Option<u32>,
    },
    /// Replaces the marks of the previous heuristic alignments with `alignments`, one part per source document each (an empty range is an anchor), and applies all marks like `ApplyAlignments`. An alignment that collides with a mark the user made is left out. With `within`, only the heuristic marks nested inside that alignment are replaced and the others stay. On failure the marks are unchanged.
    ApplyHeuristicAlignments {
        alignments: Vec<Vec<Range<usize>>>,
        confirmed: bool,
        #[serde(default)]
        within: Option<u32>,
    },
}


/// The parser that finds the entities Heuristics Alignment pairs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Extractor {
    #[default]
    TreeSitter,
}


/// What Heuristics Alignment looks for and how it pairs what it found. Similarities are percentages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HeuristicOptions {
    pub extractor: Extractor,
    pub functions: bool,
    pub methods: bool,
    pub types: bool,
    /// Look for the blocks inside functions and methods: conditions, loops, switches with their cases, try statements and lambdas.
    pub blocks: bool,
    /// Pair entities with the same name when the signature changed.
    pub match_names: bool,
    /// Pair entities whose signature and body are similar enough.
    pub match_similar: bool,
    pub signature_similarity: u8,
    pub body_similarity: u8,
    /// Pair the one overload of a name left over after the exact signatures were paired. Otherwise overloads only pair by exact signature.
    pub overload_leftovers: bool,
    /// Align an entity that one document lacks, at an anchor between the entities that surround it in the others, when those are neighbors in that document too.
    pub missing_by_neighbors: bool,
}

impl Default for HeuristicOptions {
    fn default() -> Self {
        HeuristicOptions {
            extractor: Extractor::TreeSitter,
            functions: true,
            methods: true,
            types: false,
            blocks: false,
            match_names: true,
            match_similar: true,
            signature_similarity: 80,
            body_similarity: 50,
            overload_leftovers: false,
            missing_by_neighbors: true,
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EntityKind {
    Function,
    Method,
    Type,
    /// A statement with a body inside a function: condition, loop, switch, case, try, lambda or nested function.
    Block,
}


/// How the parts of a heuristic alignment were paired: the weakest pairing among its documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchStage {
    Signature,
    Name,
    /// Signature and body similarity, in percent.
    Similar { signature: u8, body: u8 },
}


/// One alignment Heuristics Alignment proposes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeuristicMatch {
    pub kind: EntityKind,
    /// Per source document, the qualified name of its entity; `None` where the document lacks it and has an anchor.
    pub names: Vec<Option<String>>,
    /// Per source document, the lines of its entity, or an anchor (empty range).
    pub parts: Vec<Range<usize>>,
    pub stage: MatchStage,
    /// The index in `HeuristicProposal::matches` of the match whose entities hold the entities of this one. Parents come before their children.
    #[serde(default)]
    pub parent: Option<usize>,
    /// Some document has the entity away from where the display order shows it, so its hunk is exchanged.
    pub moved: bool,
}


/// The answer to `ProposeHeuristicAlignments`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeuristicProposal {
    pub cause: Option<RequestId>,
    /// Ordered by position in A.
    pub matches: Vec<HeuristicMatch>,
    /// Per source document, how many entities it has, at any depth.
    pub found: Vec<usize>,
    /// Matches left out because they collide with a mark the user made.
    pub blocked: usize,
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
    HeuristicProposal(HeuristicProposal),
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
    OutputEdited {
        range: Range<usize>,
        text: String,
        modified: bool,
        merge: MergeUpdate,
    },
    OutputPathChanged(Option<PathBuf>),
    OutputSaved(PathBuf),
    OutputEolChanged(EolChoice),
    /// The alignment marks changed. The comparison did not, so regions and rows stay as they were.
    AlignmentsChanged(Alignments),
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
    /// The alignment marks and whether the comparison is up to date with them.
    pub alignments: Alignments,
}

/// Where a mark stands after the last Apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarkStatus {
    /// Part of the alignments the current comparison was computed with.
    Applied,
    /// Part of a complete alignment that Apply would put into effect.
    Pending,
    /// Its number is not marked in every source document, so Apply ignores it.
    Unmatched,
}

/// One source document's share of a manual alignment, as the user marked it. Marks with the same number in every source document are aligned with each other.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlignmentMark {
    pub document: Document,
    pub number: u32,
    /// The marked lines. An empty range is an anchor: the position before that line.
    pub lines: Range<usize>,
    pub status: MarkStatus,
    /// Set when its alignment shows the hunk away from its place in the document: the number of the alignment whose hunk is shown where this one was. Taken from the applied alignments when `status` is `Applied`, from where the alignment would be shown otherwise.
    #[serde(default)]
    pub replaced_by: Option<u32>,
    /// Made by Heuristics Alignment, which replaces these marks when it runs again.
    #[serde(default)]
    pub heuristic: bool,
}

impl AlignmentMark {
    pub fn is_anchor(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn is_moved(&self) -> bool {
        self.replaced_by.is_some()
    }
}

/// Every mark, ordered by document and number.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alignments {
    pub marks: Vec<AlignmentMark>,
    /// The marks give other alignments than the comparison was computed with, so Apply would compare again.
    pub changed: bool,
    /// Every complete alignment that some document has in another place than the others, with the places it could be shown at, ordered by number.
    #[serde(default)]
    pub insets: Vec<InsetChoice>,
    /// Set only in the answer to a mark: the alignment that mark completed, when it is an inset and the user should say where to show it. It is shown at the option that fits the documents best until then.
    #[serde(default)]
    pub ask: Option<u32>,
}

/// Where an alignment that the documents disagree on can be shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsetChoice {
    pub number: u32,
    /// The index in `options` of where it is shown now.
    pub current: usize,
    /// Each way to show it: one per set of documents that keep their order. The documents that don't are the ones whose hunk moves.
    pub options: Vec<InsetOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsetOption {
    /// The place in the display order of the other alignments, counting from 0 at the top: the `slot` of `PlaceAlignment`.
    pub slot: usize,
    /// The alignments shown right above and right below that place.
    pub above: Option<u32>,
    pub below: Option<u32>,
    /// The documents whose hunk is moved there.
    pub moved: Vec<Document>,
}

impl Alignments {
    pub fn of(&self, document: Document) -> impl Iterator<Item = &AlignmentMark> {
        self.marks.iter().filter(move |mark| mark.document == document)
    }
}

/// Per line of one source document, the char runs of the line in its row; empty for a line whose row has no differences.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineCharRuns {
    pub lines: Vec<Vec<CharRun>>,
}

impl LineCharRuns {
    /// Empty for a line out of range.
    pub fn line(&self, line: usize) -> &[CharRun] {
        self.lines.get(line).map(Vec::as_slice).unwrap_or(&[])
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
    /// The line ending style D is saved with.
    pub eol: EolChoice,
}

/// The line ending style of D. It is a conflict while the source documents disagree and nobody has chosen; D can't be saved then.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EolChoice {
    Style(EolStyle),
    Conflict,
}

impl Default for EolChoice {
    fn default() -> Self {
        EolChoice::Style(EolStyle::Lf)
    }
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
    /// The most frequent line terminator; `None` when the text has none, so it has no style to tell.
    pub eol: Option<EolStyle>,
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
        self.lines.get(index).and_then(|range| self.text.get(range.clone())).unwrap_or("")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorKind {
    Io,
    InvalidRequest,
    Canceled,
    /// A comparison engine failed or returned a result that broke the rules.
    Engine,
    /// The command would discard work in D. Send it again with `confirmed` set once the user agreed.
    NeedsConfirmation,
    /// The source documents disagree on the line ending style and none was chosen for D. Send `SetOutputEol` first.
    EolConflict,
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
        DocumentText {
            document: Document::A,
            path: PathBuf::from("a.txt"),
            label: None,
            eol: Some(EolStyle::Lf),
            text: text.to_string(),
            lines,
        }
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
        let mut files = OpenFiles {
            a: PathBuf::from("a"),
            b: PathBuf::from("b"),
            ..OpenFiles::default()
        };
        assert!(!files.is_merge());
        files.output = Some(PathBuf::from("d"));
        assert!(files.is_merge());
        files.output = None;
        files.c = Some(PathBuf::from("c"));
        assert!(files.is_merge());
    }

    #[test]
    fn labels_by_document() {
        let labels = Labels {
            a: Some("Base".to_string()),
            b: None,
            c: Some("Remote".to_string()),
        };
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

    fn mark(document: Document, number: u32, lines: Range<usize>) -> AlignmentMark {
        AlignmentMark {
            document,
            number,
            lines,
            status: MarkStatus::Pending,
            replaced_by: None,
            heuristic: false,
        }
    }

    #[test]
    fn an_empty_mark_is_an_anchor() {
        assert!(mark(Document::A, 1, 3..3).is_anchor());
        assert!(!mark(Document::A, 1, 3..4).is_anchor());
    }

    #[test]
    fn alignments_are_listed_per_document() {
        let alignments = Alignments {
            marks: vec![mark(Document::A, 1, 0..1), mark(Document::B, 1, 2..4), mark(Document::B, 2, 5..5)],
            changed: true,
            ..Alignments::default()
        };
        let numbers = |document| -> Vec<u32> { alignments.of(document).map(|mark| mark.number).collect() };
        assert_eq!(numbers(Document::A), [1]);
        assert_eq!(numbers(Document::B), [1, 2]);
        assert!(numbers(Document::C).is_empty());
    }

    #[test]
    fn document_letters() {
        let letters: Vec<&str> = Document::ALL.iter().map(|document| document.letter()).collect();
        assert_eq!(letters, ["A", "B", "C", "D"]);
    }
}
