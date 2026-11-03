mod alignment;
mod check;
#[cfg(test)]
mod test_engine;

use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::messages::{Document, ErrorKind, ProtocolError};

pub use alignment::validate_alignments;
pub use check::check_comparison;


/// Interned line key. Two lines are equal when their tokens are equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Token(pub u32);


/// Lines of one source document, or of a part of it, with their tokens. Line indices count from the first line given.
#[derive(Debug, Clone, Copy)]
pub struct SourceLines<'a> {
    pub lines: &'a [&'a str],
    pub tokens: &'a [Token],
}


impl<'a> SourceLines<'a> {
    pub fn len(&self) -> usize {
        self.lines.len()
    }


    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }


    /// Returns no lines for a range that doesn't fit.
    pub fn slice(&self, range: Range<usize>) -> SourceLines<'a> {
        SourceLines { lines: self.lines.get(range.clone()).unwrap_or(&[]), tokens: self.tokens.get(range).unwrap_or(&[]) }
    }
}


#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffAlgorithm {
    #[default]
    Histogram,
    Myers,
}


/// The part of the engine config a comparison engine sees. Ignore options are applied by the session when it builds tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComparisonOptions {
    pub algorithm: DiffAlgorithm,
    pub try_hard: bool,
    /// Pair equal lines of B and C in regions where both changed (kdiff3 `Diff3AlignBC`).
    pub align_b_with_c: bool,
    /// Lines longer than this, in characters, are marked as different as a whole.
    pub char_diff_line_limit: usize,
}


impl Default for ComparisonOptions {
    fn default() -> ComparisonOptions {
        ComparisonOptions { algorithm: DiffAlgorithm::Histogram, try_hard: false, align_b_with_c: false, char_diff_line_limit: 10_000 }
    }
}


/// One part per source document, in A, B, C order. An empty range is an anchor: the position before that line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualAlignment {
    pub parts: Vec<Range<usize>>,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HunkChange {
    Unchanged,
    Added,
    Removed,
    Modified,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeStatus {
    Unchanged,
    ChangedInB,
    ChangedInC,
    ChangedAlike,
    Conflict,
    WhitespaceConflict,
}


/// `Diff` for Diff Regions (the hunk change of B), `Merge` for Merge Regions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegionStatus {
    Diff(HunkChange),
    Merge(MergeStatus),
}


/// One line index per source document, or `None` where that document has a void.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    pub lines: Vec<Option<usize>>,
}


/// One hunk per source document, as a line range, and the rows the region's lines are paired into.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub hunks: Vec<Range<usize>>,
    pub status: RegionStatus,
    pub rows: Vec<Row>,
}


#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comparison {
    pub regions: Vec<Region>,
}


/// Changed column ranges of each line of one row, per source document; empty where the row has no line.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharDiff {
    pub changed: Vec<Vec<Range<usize>>>,
}


/// Per source document, the first byte offset where it differs from A, or `None` when it equals A.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryComparison {
    pub first_difference: Vec<Option<usize>>,
}


/// Every engine compares 2 source documents, and compares binary files through the default method.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineCapabilities {
    pub id: String,
    pub version: String,
    pub three_way: bool,
    /// Honors manual alignments itself. Otherwise the segmentation helper compares each segment separately.
    pub manual_alignment: bool,
    pub char_diff: bool,
    pub progress: bool,
    pub algorithms: Vec<DiffAlgorithm>,
    pub try_hard: bool,
    pub align_b_with_c: bool,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
}


/// The cancel flag and progress sink of the host job running a comparison.
pub struct Job<'a> {
    cancel: &'a AtomicBool,
    progress: &'a (dyn Fn(Progress) + Sync),
}


impl<'a> Job<'a> {
    pub fn new(cancel: &'a AtomicBool, progress: &'a (dyn Fn(Progress) + Sync)) -> Job<'a> {
        Job { cancel, progress }
    }


    pub fn is_canceled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }


    pub fn check_canceled(&self) -> Result<(), EngineError> {
        if self.is_canceled() {
            return Err(EngineError::Canceled);
        }
        Ok(())
    }


    pub fn report(&self, done: u64, total: u64) {
        (self.progress)(Progress { done, total });
    }
}


#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EngineError {
    #[error("comparison canceled")]
    Canceled,
    #[error("not supported: {0}")]
    Unsupported(String),
    #[error("invalid comparison input: {0}")]
    InvalidInput(String),
    #[error("invalid manual alignment: {0}")]
    InvalidAlignment(String),
    #[error("invalid engine result: {0}")]
    InvalidResult(String),
    #[error("comparison failed: {0}")]
    Failed(String),
}


impl From<EngineError> for ProtocolError {
    fn from(error: EngineError) -> ProtocolError {
        let kind = match error {
            EngineError::Canceled => ErrorKind::Canceled,
            EngineError::InvalidInput(_) | EngineError::InvalidAlignment(_) => ErrorKind::InvalidRequest,
            EngineError::Unsupported(_) | EngineError::InvalidResult(_) | EngineError::Failed(_) => ErrorKind::Engine,
        };
        ProtocolError { kind, message: error.to_string() }
    }
}


/// A pluggable comparison algorithm. Holds no session state; the host may call it from several worker threads.
pub trait ComparisonEngine: Send + Sync {
    fn capabilities(&self) -> EngineCapabilities;
    /// Called through `run_comparison` only: with 2 or 3 valid sources, valid alignments, and alignments only when the engine honors them itself.
    fn compare(&self, input: &CompareInput<'_>, job: &Job<'_>) -> Result<Comparison, EngineError>;
    /// Status of a region with these hunks, e.g. one part of a split region.
    fn classify(&self, hunks: &[SourceLines<'_>]) -> Result<RegionStatus, EngineError>;
    /// Character-level diff of the lines sharing one row.
    fn char_diff(&self, row: &[Option<&str>], options: &ComparisonOptions) -> Result<CharDiff, EngineError>;

    fn compare_binary(&self, sources: &[&[u8]], job: &Job<'_>) -> Result<BinaryComparison, EngineError> {
        job.check_canceled()?;
        let base = sources.first().ok_or_else(|| EngineError::InvalidInput("no source documents".to_string()))?;
        let first_difference = sources
            .iter()
            .map(|source| first_byte_difference(base, source))
            .collect();
        Ok(BinaryComparison { first_difference })
    }
}


pub struct CompareInput<'a> {
    /// A, B and, in 3-way mode, C.
    pub sources: Vec<SourceLines<'a>>,
    /// In document order, with line ranges relative to `sources`.
    pub alignments: &'a [ManualAlignment],
    pub options: &'a ComparisonOptions,
}


impl CompareInput<'_> {
    pub fn line_counts(&self) -> Vec<usize> {
        self.sources
            .iter()
            .map(|source| source.len())
            .collect()
    }
}


/// The only way the host compares: validates the input, routes manual alignments to the engine or the segmentation helper, and checks the result.
pub fn run_comparison(engine: &dyn ComparisonEngine, input: &CompareInput<'_>, job: &Job<'_>) -> Result<Comparison, EngineError> {
    let capabilities = engine.capabilities();
    validate_input(input, &capabilities)?;
    job.check_canceled()?;
    let comparison = if input.alignments.is_empty() || capabilities.manual_alignment {
        engine.compare(input, job)?
    } else {
        alignment::compare_segments(engine, input, job)?
    };
    check_comparison(&comparison, input)?;
    Ok(comparison)
}


fn validate_input(input: &CompareInput<'_>, capabilities: &EngineCapabilities) -> Result<(), EngineError> {
    match input.sources.len() {
        2 => {}
        3 if capabilities.three_way => {}
        3 => return Err(EngineError::Unsupported(format!("engine {} has no 3-way comparison", capabilities.id))),
        count => return Err(EngineError::InvalidInput(format!("{count} source documents, expected 2 or 3"))),
    }
    for (index, source) in input.sources.iter().enumerate() {
        if source.lines.len() != source.tokens.len() {
            return Err(EngineError::InvalidInput(format!("document {}: {} lines but {} tokens", document_letter(index), source.lines.len(), source.tokens.len())));
        }
    }
    validate_alignments(input.alignments, &input.line_counts())
}


fn document_letter(index: usize) -> &'static str {
    Document::ALL
        .get(index)
        .map(|document| document.letter())
        .unwrap_or("?")
}


fn first_byte_difference(base: &[u8], other: &[u8]) -> Option<usize> {
    base.iter()
        .zip(other)
        .position(|(left, right)| left != right)
        .or_else(|| (base.len() != other.len()).then(|| base.len().min(other.len())))
}


#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::test_engine::{Fixture, SimpleEngine, with_job};
    use super::*;


    #[test]
    fn rejects_wrong_source_count() {
        let fixture = Fixture::new(&[&["a"]]);
        let error = with_job(|job| run_comparison(&SimpleEngine::default(), &fixture.input(&[]), job)).unwrap_err();
        assert!(matches!(error, EngineError::InvalidInput(_)));
        let fixture = Fixture::new(&[&["a"], &["a"], &["a"], &["a"]]);
        let error = with_job(|job| run_comparison(&SimpleEngine::default(), &fixture.input(&[]), job)).unwrap_err();
        assert!(matches!(error, EngineError::InvalidInput(_)));
    }


    #[test]
    fn rejects_three_way_without_capability() {
        let fixture = Fixture::new(&[&["a"], &["b"], &["c"]]);
        let engine = SimpleEngine { three_way: false, ..SimpleEngine::default() };
        let error = with_job(|job| run_comparison(&engine, &fixture.input(&[]), job)).unwrap_err();
        assert!(matches!(error, EngineError::Unsupported(_)));
    }


    #[test]
    fn rejects_token_count_mismatch() {
        let options = ComparisonOptions::default();
        let lines = ["a", "b"];
        let tokens = [Token(0)];
        let source = SourceLines { lines: &lines, tokens: &tokens };
        let input = CompareInput { sources: vec![source.clone(), source], alignments: &[], options: &options };
        let error = with_job(|job| run_comparison(&SimpleEngine::default(), &input, job)).unwrap_err();
        assert!(matches!(error, EngineError::InvalidInput(_)));
    }


    #[test]
    fn canceled_job_stops_comparison() {
        let fixture = Fixture::new(&[&["a"], &["b"]]);
        let cancel = AtomicBool::new(true);
        let progress = |_: Progress| {};
        let error = run_comparison(&SimpleEngine::default(), &fixture.input(&[]), &Job::new(&cancel, &progress)).unwrap_err();
        assert_eq!(error, EngineError::Canceled);
    }


    #[test]
    fn job_reports_progress() {
        let cancel = AtomicBool::new(false);
        let reports = Mutex::new(Vec::new());
        let progress = |progress: Progress| reports.lock().unwrap().push(progress);
        Job::new(&cancel, &progress).report(3, 10);
        assert_eq!(reports.into_inner().unwrap(), [Progress { done: 3, total: 10 }]);
    }


    #[test]
    fn invalid_result_is_rejected() {
        let fixture = Fixture::new(&[&["a", "b"], &["a", "c"]]);
        let engine = SimpleEngine { drop_last_region: true, ..SimpleEngine::default() };
        let error = with_job(|job| run_comparison(&engine, &fixture.input(&[]), job)).unwrap_err();
        assert!(matches!(error, EngineError::InvalidResult(_)));
    }


    #[test]
    fn native_engine_gets_alignments() {
        let fixture = Fixture::new(&[&["a", "b", "c"], &["a", "x", "c"]]);
        let alignments = [ManualAlignment { parts: vec![1..2, 1..2] }];
        let engine = SimpleEngine { manual_alignment: true, ..SimpleEngine::default() };
        with_job(|job| run_comparison(&engine, &fixture.input(&alignments), job)).unwrap();
        assert_eq!(engine.seen_alignments.into_inner().unwrap(), [1, 0, 0, 0]);
    }


    #[test]
    fn other_engines_get_segments() {
        let fixture = Fixture::new(&[&["a", "b", "c"], &["a", "x", "c"]]);
        let alignments = [ManualAlignment { parts: vec![1..2, 1..2] }];
        let engine = SimpleEngine::default();
        with_job(|job| run_comparison(&engine, &fixture.input(&alignments), job)).unwrap();
        assert_eq!(engine.seen_alignments.into_inner().unwrap(), [0, 0, 0]);
    }


    #[test]
    fn binary_compare_by_default() {
        let engine = SimpleEngine::default();
        let result = with_job(|job| engine.compare_binary(&[b"abc", b"abc", b"abd", b"ab", b"abcd"], job)).unwrap();
        assert_eq!(result.first_difference, [None, None, Some(2), Some(2), Some(3)]);
        let error = with_job(|job| engine.compare_binary(&[], job)).unwrap_err();
        assert!(matches!(error, EngineError::InvalidInput(_)));
    }


    #[test]
    fn engine_errors_cross_as_protocol_errors() {
        assert_eq!(ProtocolError::from(EngineError::Canceled).kind, ErrorKind::Canceled);
        assert_eq!(ProtocolError::from(EngineError::InvalidAlignment("x".to_string())).kind, ErrorKind::InvalidRequest);
        assert_eq!(ProtocolError::from(EngineError::InvalidInput("x".to_string())).kind, ErrorKind::InvalidRequest);
        let error = ProtocolError::from(EngineError::Failed("boom".to_string()));
        assert_eq!(error.kind, ErrorKind::Engine);
        assert_eq!(error.message, "comparison failed: boom");
    }


    #[test]
    fn slice_outside_lines_is_empty() {
        let lines = ["a", "b"];
        let tokens = [Token(0), Token(1)];
        let source = SourceLines { lines: &lines, tokens: &tokens };
        assert_eq!(source.slice(1..2).lines, ["b"]);
        assert!(source.slice(1..5).is_empty());
    }
}
