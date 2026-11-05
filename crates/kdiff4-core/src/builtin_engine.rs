use std::ops::Range;

use imara_diff::{Algorithm, Diff, Hunk, InternedInput, Interner};
use kdiff4_protocol::engine::{CharDiff, CharRun, CompareInput, Comparison, ComparisonEngine, ComparisonOptions, DiffAlgorithm, EngineCapabilities, EngineError, HunkChange, Job, MergeStatus, Region, RegionStatus, Row, SourceLines, Token};

use crate::fine_diff;


pub const BUILTIN_ENGINE_ID: &str = "builtin";


/// imara-diff refuses sequences of `i32::MAX` tokens or more.
const MAX_TOKENS: usize = i32::MAX as usize - 1;


/// The default comparison engine, built on `imara-diff`. Manual alignments go through the segmentation helper.
#[derive(Debug, Clone, Copy, Default)]
pub struct BuiltinEngine;


impl ComparisonEngine for BuiltinEngine {
    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            id: BUILTIN_ENGINE_ID.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            three_way: true,
            manual_alignment: false,
            char_diff: true,
            progress: true,
            algorithms: vec![DiffAlgorithm::Histogram, DiffAlgorithm::Myers],
            try_hard: true,
            align_b_with_c: false,
        }
    }


    fn compare(&self, input: &CompareInput<'_>, job: &Job<'_>) -> Result<Comparison, EngineError> {
        if input.sources.iter().any(|source| source.len() > MAX_TOKENS) {
            return Err(EngineError::Unsupported(format!("documents with more than {MAX_TOKENS} lines")));
        }
        let algorithm = algorithm(input.options);
        let steps = input.sources.len() as u64;
        let rows = match input.sources.as_slice() {
            [a, b] => {
                job.check_canceled()?;
                let rows = pairs(algorithm, a.tokens, b.tokens);
                job.report(1, steps);
                rows.into_iter()
                    .map(|(line_a, line_b)| vec![line_a, line_b])
                    .collect()
            }
            [a, b, c] => three_way_rows(algorithm, a, b, c, job, steps)?,
            sources => return Err(EngineError::InvalidInput(format!("{} source documents, expected 2 or 3", sources.len()))),
        };
        job.check_canceled()?;
        let regions = self.regions(input, rows)?;
        job.report(steps, steps);
        Ok(Comparison { regions })
    }


    fn classify(&self, hunks: &[SourceLines<'_>]) -> Result<RegionStatus, EngineError> {
        match hunks {
            [a, b] => Ok(RegionStatus::Diff(hunk_change(a, b))),
            [a, b, c] => Ok(RegionStatus::Merge(merge_status(a, b, c))),
            _ => Err(EngineError::InvalidInput(format!("{} hunks, expected 2 or 3", hunks.len()))),
        }
    }


    fn char_diff(&self, row: &[Option<&str>], options: &ComparisonOptions) -> Result<CharDiff, EngineError> {
        let limit = options.char_diff_line_limit.min(MAX_TOKENS);
        let lines: Vec<Option<Vec<char>>> = row
            .iter()
            .map(|line| line.map(|line| line.chars().collect()))
            .collect();
        let mut same: Vec<Vec<Vec<bool>>> = lines
            .iter()
            .enumerate()
            .map(|(index, line)| vec![initial_same_in(&lines, index); line.as_ref().map_or(0, Vec::len)])
            .collect();
        for left in 0..lines.len() {
            for right in left + 1..lines.len() {
                let (Some(left_chars), Some(right_chars)) = (&lines[left], &lines[right]) else { continue };
                if left_chars == right_chars {
                    continue;
                }
                let changed: Vec<(Range<usize>, Range<usize>)> = if left_chars.len() > limit || right_chars.len() > limit {
                    vec![(0..left_chars.len(), 0..right_chars.len())]
                } else if (left, right) == (0, 2) {
                    // kdiff3: diff.cpp, Diff3Line::fineDiff diffs C against A, and the result depends on the direction
                    fine_diff::changed_ranges(right_chars, left_chars)
                        .into_iter()
                        .map(|(right_columns, left_columns)| (left_columns, right_columns))
                        .collect()
                } else {
                    fine_diff::changed_ranges(left_chars, right_chars)
                };
                for (left_columns, right_columns) in changed {
                    for column in left_columns {
                        same[left][column][right] = false;
                    }
                    for column in right_columns {
                        same[right][column][left] = false;
                    }
                }
            }
        }
        let runs = same
            .into_iter()
            .map(char_runs)
            .collect();
        Ok(CharDiff { runs })
    }
}


impl BuiltinEngine {
    /// Runs of rows where every document has an equal line become unchanged regions; the runs between them are changed regions.
    fn regions(&self, input: &CompareInput<'_>, rows: Vec<Vec<Option<usize>>>) -> Result<Vec<Region>, EngineError> {
        let mut regions = Vec::new();
        let mut cursor = vec![0; input.sources.len()];
        let mut run: Vec<Row> = Vec::new();
        let mut run_stable = false;
        for lines in rows {
            let stable = is_stable(input, &lines);
            if !run.is_empty() && stable != run_stable {
                regions.push(self.region(input, std::mem::take(&mut run), &mut cursor)?);
            }
            run_stable = stable;
            run.push(Row { lines });
        }
        if !run.is_empty() {
            regions.push(self.region(input, run, &mut cursor)?);
        }
        Ok(regions)
    }


    fn region(&self, input: &CompareInput<'_>, rows: Vec<Row>, cursor: &mut [usize]) -> Result<Region, EngineError> {
        let hunks: Vec<Range<usize>> = cursor
            .iter_mut()
            .enumerate()
            .map(|(document, start)| {
                let count = rows.iter().filter(|row| row.lines[document].is_some()).count();
                let hunk = start.clone()..start.clone() + count;
                *start += count;
                hunk
            })
            .collect();
        let lines: Vec<SourceLines<'_>> = input
            .sources
            .iter()
            .zip(&hunks)
            .map(|(source, hunk)| source.slice(hunk.clone()))
            .collect();
        let status = self.classify(&lines)?;
        Ok(Region { hunks, status, rows })
    }
}


fn algorithm(options: &ComparisonOptions) -> Algorithm {
    match (options.algorithm, options.try_hard) {
        (DiffAlgorithm::Histogram, _) => Algorithm::Histogram,
        (DiffAlgorithm::Myers, false) => Algorithm::Myers,
        (DiffAlgorithm::Myers, true) => Algorithm::MyersMinimal,
    }
}


fn diff_hunks(algorithm: Algorithm, before: Vec<u32>, after: Vec<u32>) -> Vec<Hunk> {
    let mut input = InternedInput { before: Vec::new(), after: Vec::new(), interner: Interner::new(before.len() + after.len()) };
    input.update_before(before.into_iter());
    input.update_after(after.into_iter());
    let mut diff = Diff::compute(algorithm, &input);
    diff.postprocess_no_heuristic(&input);
    diff.hunks().collect()
}


fn range(range: &Range<u32>) -> Range<usize> {
    range.start as usize..range.end as usize
}


/// Rows of a 2-way alignment of `left` and `right`: equal lines share a row, the lines of a changed chunk are paired by position, and the rest of the longer side gets rows of its own.
fn pairs(algorithm: Algorithm, left: &[Token], right: &[Token]) -> Vec<(Option<usize>, Option<usize>)> {
    let values = |tokens: &[Token]| tokens.iter().map(|token| token.0.clone()).collect::<Vec<u32>>();
    let mut rows = Vec::with_capacity(left.len().max(right.len()));
    let (mut next_left, mut next_right) = (0, 0);
    let equal_rows = |rows: &mut Vec<(Option<usize>, Option<usize>)>, next_left: &mut usize, next_right: &mut usize, end_left: usize| {
        while *next_left < end_left {
            rows.push((Some(next_left.clone()), Some(next_right.clone())));
            *next_left += 1;
            *next_right += 1;
        }
    };
    for hunk in diff_hunks(algorithm, values(left), values(right)) {
        let (before, after) = (range(&hunk.before), range(&hunk.after));
        equal_rows(&mut rows, &mut next_left, &mut next_right, before.start.clone());
        let paired = before.len().min(after.len());
        rows.extend((0..paired).map(|offset| (Some(before.start + offset), Some(after.start + offset))));
        rows.extend((before.start + paired..before.end).map(|line| (Some(line), None)));
        rows.extend((after.start + paired..after.end).map(|line| (None, Some(line))));
        next_left = before.end;
        next_right = after.end;
    }
    equal_rows(&mut rows, &mut next_left, &mut next_right, left.len());
    rows
}


/// How the lines of B or C relate to the lines of A.
struct Pairing {
    /// Per line of A, the line in the same row.
    partners: Vec<Option<usize>>,
    /// Per line of A, and once more after the last one: the lines with no line of A in their row that come right before it.
    unpaired: Vec<Range<usize>>,
}


impl Pairing {
    fn new(rows: Vec<(Option<usize>, Option<usize>)>, a_count: usize) -> Pairing {
        let mut partners = vec![None; a_count];
        let mut unpaired = vec![0..0; a_count + 1];
        let mut pending: Option<Range<usize>> = None;
        for (line_a, line) in rows {
            match line_a {
                Some(line_a) => {
                    unpaired[line_a] = pending.take().unwrap_or(0..0);
                    partners[line_a] = line;
                }
                None => {
                    let Some(line) = line else { continue };
                    pending = Some(pending.map_or(line..line + 1, |range| range.start..line + 1));
                }
            }
        }
        unpaired[a_count] = pending.unwrap_or(0..0);
        Pairing { partners, unpaired }
    }
}


/// B and C are each aligned with A. Lines of B and C between the same two lines of A are aligned with each other, so equal ones share a row.
fn three_way_rows(algorithm: Algorithm, a: &SourceLines<'_>, b: &SourceLines<'_>, c: &SourceLines<'_>, job: &Job<'_>, steps: u64) -> Result<Vec<Vec<Option<usize>>>, EngineError> {
    job.check_canceled()?;
    let with_b = Pairing::new(pairs(algorithm, a.tokens, b.tokens), a.len());
    job.report(1, steps);
    job.check_canceled()?;
    let with_c = Pairing::new(pairs(algorithm, a.tokens, c.tokens), a.len());
    job.report(2, steps);
    let mut rows = Vec::new();
    for line_a in 0..=a.len() {
        let (run_b, run_c) = (with_b.unpaired[line_a].clone(), with_c.unpaired[line_a].clone());
        if !run_b.is_empty() || !run_c.is_empty() {
            for (line_b, line_c) in pairs(algorithm, b.slice(run_b.clone()).tokens, c.slice(run_c.clone()).tokens) {
                rows.push(vec![None, line_b.map(|line| line + run_b.start), line_c.map(|line| line + run_c.start)]);
            }
        }
        if line_a < a.len() {
            rows.push(vec![Some(line_a.clone()), with_b.partners[line_a].clone(), with_c.partners[line_a].clone()]);
        }
    }
    Ok(rows)
}


fn is_stable(input: &CompareInput<'_>, lines: &[Option<usize>]) -> bool {
    let tokens: Option<Vec<Token>> = input
        .sources
        .iter()
        .zip(lines)
        .map(|(source, line)| line.and_then(|line| source.tokens.get(line).cloned()))
        .collect();
    match tokens {
        Some(tokens) => tokens.iter().all(|token| *token == tokens[0]),
        None => false,
    }
}


fn hunk_change(a: &SourceLines<'_>, b: &SourceLines<'_>) -> HunkChange {
    if a.tokens == b.tokens {
        HunkChange::Unchanged
    } else if a.is_empty() {
        HunkChange::Added
    } else if b.is_empty() {
        HunkChange::Removed
    } else {
        HunkChange::Modified
    }
}


fn merge_status(a: &SourceLines<'_>, b: &SourceLines<'_>, c: &SourceLines<'_>) -> MergeStatus {
    match (a.tokens != b.tokens, a.tokens != c.tokens) {
        (false, false) => MergeStatus::Unchanged,
        (true, false) => MergeStatus::ChangedInB,
        (false, true) => MergeStatus::ChangedInC,
        _ if b.tokens == c.tokens => MergeStatus::ChangedAlike,
        _ if without_whitespace(b) == without_whitespace(c) => MergeStatus::WhitespaceConflict,
        _ => MergeStatus::Conflict,
    }
}


/// All characters of a hunk except whitespace, so hunks that differ only in spaces, tabs, line breaks or blank lines compare equal.
fn without_whitespace(hunk: &SourceLines<'_>) -> String {
    hunk.lines
        .iter()
        .flat_map(|line| line.chars())
        .filter(|char| !char.is_whitespace())
        .collect()
}


/// Per source document of a row, whether it has a line the line at `index` can match: its own line always does, a void never.
fn initial_same_in(lines: &[Option<Vec<char>>], index: usize) -> Vec<bool> {
    lines
        .iter()
        .enumerate()
        .map(|(other, line)| other == index || line.is_some())
        .collect()
}


/// Joins consecutive characters with the same flags into runs.
fn char_runs(flags: Vec<Vec<bool>>) -> Vec<CharRun> {
    let mut runs: Vec<CharRun> = Vec::new();
    for (column, same_in) in flags.into_iter().enumerate() {
        match runs.last_mut() {
            Some(run) if run.same_in == same_in => run.columns.end = column + 1,
            _ => runs.push(CharRun { columns: column..column + 1, same_in }),
        }
    }
    runs
}


#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicBool;

    use kdiff4_protocol::engine::{ManualAlignment, Progress, run_comparison};
    use proptest::collection::vec;
    use proptest::prelude::*;
    use proptest::sample::select;

    use super::*;


    struct Documents {
        lines: Vec<Vec<&'static str>>,
        tokens: Vec<Vec<Token>>,
    }


    impl Documents {
        fn new(documents: &[&[&'static str]]) -> Documents {
            let mut interned: HashMap<&str, Token> = HashMap::new();
            let mut tokens = Vec::new();
            for document in documents {
                let mut document_tokens = Vec::new();
                for line in document.iter() {
                    let next = Token(interned.len() as u32);
                    document_tokens.push(interned.entry(*line).or_insert(next).clone());
                }
                tokens.push(document_tokens);
            }
            let lines = documents
                .iter()
                .map(|document| document.to_vec())
                .collect();
            Documents { lines, tokens }
        }


        fn sources(&self) -> Vec<SourceLines<'_>> {
            self.lines
                .iter()
                .zip(&self.tokens)
                .map(|(lines, tokens)| SourceLines { lines, tokens })
                .collect()
        }


        fn compare_with(&self, alignments: &[ManualAlignment], options: &ComparisonOptions) -> Result<Comparison, EngineError> {
            let cancel = AtomicBool::new(false);
            let progress = |_: Progress| {};
            run_comparison(&BuiltinEngine, &CompareInput { sources: self.sources(), alignments, options }, &Job::new(&cancel, &progress))
        }


        fn compare(&self, alignments: &[ManualAlignment]) -> Comparison {
            self.compare_with(alignments, &ComparisonOptions::default()).unwrap()
        }
    }


    fn summary(comparison: &Comparison) -> Vec<(Vec<Range<usize>>, RegionStatus)> {
        comparison
            .regions
            .iter()
            .map(|region| (region.hunks.clone(), region.status.clone()))
            .collect()
    }


    fn rows(comparison: &Comparison) -> Vec<Vec<Option<usize>>> {
        comparison
            .regions
            .iter()
            .flat_map(|region| region.rows.iter().map(|row| row.lines.clone()))
            .collect()
    }


    const UNCHANGED_2: RegionStatus = RegionStatus::Diff(HunkChange::Unchanged);
    const UNCHANGED_3: RegionStatus = RegionStatus::Merge(MergeStatus::Unchanged);


    #[test]
    fn identical_documents_are_one_unchanged_region() {
        let comparison = Documents::new(&[&["a", "b"], &["a", "b"]]).compare(&[]);
        assert_eq!(summary(&comparison), [(vec![0..2, 0..2], UNCHANGED_2)]);
        assert_eq!(rows(&comparison), [vec![Some(0), Some(0)], vec![Some(1), Some(1)]]);
    }


    #[test]
    fn empty_documents_have_no_regions() {
        assert_eq!(Documents::new(&[&[], &[]]).compare(&[]).regions, []);
        assert_eq!(Documents::new(&[&[], &[], &[]]).compare(&[]).regions, []);
    }


    #[test]
    fn two_way_hunk_changes() {
        let comparison = Documents::new(&[&["a", "b", "c", "d"], &["x", "a", "B", "c"]]).compare(&[]);
        let expected = [
            (vec![0..0, 0..1], RegionStatus::Diff(HunkChange::Added)),
            (vec![0..1, 1..2], UNCHANGED_2),
            (vec![1..2, 2..3], RegionStatus::Diff(HunkChange::Modified)),
            (vec![2..3, 3..4], UNCHANGED_2),
            (vec![3..4, 4..4], RegionStatus::Diff(HunkChange::Removed)),
        ];
        assert_eq!(summary(&comparison), expected);
    }


    #[test]
    fn changed_lines_are_paired_by_position() {
        let comparison = Documents::new(&[&["a", "b"], &["x", "y", "z"]]).compare(&[]);
        assert_eq!(rows(&comparison), [vec![Some(0), Some(0)], vec![Some(1), Some(1)], vec![None, Some(2)]]);
    }


    #[test]
    fn three_way_statuses() {
        let comparison = Documents::new(&[&["a", "b", "c", "x", "d", "e", "f", "g"], &["A", "b", "c", "x", "D", "e", "F", "g"], &["a", "b", "C", "x", "D", "e", "F 2", "g"]]).compare(&[]);
        let expected = [
            (vec![0..1, 0..1, 0..1], RegionStatus::Merge(MergeStatus::ChangedInB)),
            (vec![1..2, 1..2, 1..2], UNCHANGED_3),
            (vec![2..3, 2..3, 2..3], RegionStatus::Merge(MergeStatus::ChangedInC)),
            (vec![3..4, 3..4, 3..4], UNCHANGED_3),
            (vec![4..5, 4..5, 4..5], RegionStatus::Merge(MergeStatus::ChangedAlike)),
            (vec![5..6, 5..6, 5..6], UNCHANGED_3),
            (vec![6..7, 6..7, 6..7], RegionStatus::Merge(MergeStatus::Conflict)),
            (vec![7..8, 7..8, 7..8], UNCHANGED_3),
        ];
        assert_eq!(summary(&comparison), expected);
    }


    #[test]
    fn whitespace_conflict() {
        let comparison = Documents::new(&[&["a", "x = 1", "b"], &["a", "x = 2", "b"], &["a", "x  =  2", "b"]]).compare(&[]);
        assert_eq!(comparison.regions[1].status, RegionStatus::Merge(MergeStatus::WhitespaceConflict));
    }


    #[test]
    fn adjacent_changes_join() {
        let comparison = Documents::new(&[&["x", "y"], &["X", "y"], &["x", "Y"]]).compare(&[]);
        assert_eq!(summary(&comparison), [(vec![0..2, 0..2, 0..2], RegionStatus::Merge(MergeStatus::Conflict))]);
        assert_eq!(rows(&comparison), [vec![Some(0), Some(0), Some(0)], vec![Some(1), Some(1), Some(1)]]);
    }


    #[test]
    fn equal_insertions_of_b_and_c_share_rows() {
        let comparison = Documents::new(&[&["a"], &["a", "same", "only b"], &["a", "same"]]).compare(&[]);
        assert_eq!(summary(&comparison), [(vec![0..1, 0..1, 0..1], UNCHANGED_3), (vec![1..1, 1..3, 1..2], RegionStatus::Merge(MergeStatus::Conflict))]);
        assert_eq!(rows(&comparison)[1..], [vec![None, Some(1), Some(1)], vec![None, Some(2), None]]);
    }


    // The Foo and Qux example of Manual alignment in ARCHITECTURE.md.
    #[test]
    fn manual_alignments_separate_unrelated_insertions() {
        let documents = Documents::new(&[&["Bar", "Baz"], &["Bar", "Foo", "Baz"], &["Bar", "Qux", "Baz"]]);
        assert_eq!(documents.compare(&[]).regions[1].status, RegionStatus::Merge(MergeStatus::Conflict));
        let alignments = [ManualAlignment { parts: vec![1..1, 1..2, 1..1] }, ManualAlignment { parts: vec![1..1, 2..2, 1..2] }];
        let expected = [
            (vec![0..1, 0..1, 0..1], UNCHANGED_3),
            (vec![1..1, 1..2, 1..1], RegionStatus::Merge(MergeStatus::ChangedInB)),
            (vec![1..1, 2..2, 1..2], RegionStatus::Merge(MergeStatus::ChangedInC)),
            (vec![1..2, 2..3, 2..3], UNCHANGED_3),
        ];
        assert_eq!(summary(&documents.compare(&alignments)), expected);
    }


    #[test]
    fn two_way_manual_alignment() {
        let documents = Documents::new(&[&["a", "b", "c"], &["c", "b", "a"]]);
        let alignments = [ManualAlignment { parts: vec![0..1, 2..3] }];
        let comparison = documents.compare(&alignments);
        assert_eq!(comparison.regions[0].hunks, [0..0, 0..2]);
        assert_eq!(comparison.regions[1], Region { hunks: vec![0..1, 2..3], status: UNCHANGED_2, rows: vec![Row { lines: vec![Some(0), Some(2)] }] });
    }


    #[test]
    fn every_algorithm_gives_valid_results() {
        let documents = Documents::new(&[&["a", "b", "c", "a", "b"], &["b", "a", "c", "b", "a"], &["c", "a", "b"]]);
        for (algorithm, try_hard) in [(DiffAlgorithm::Histogram, false), (DiffAlgorithm::Myers, false), (DiffAlgorithm::Myers, true)] {
            let options = ComparisonOptions { algorithm, try_hard, ..ComparisonOptions::default() };
            assert!(documents.compare_with(&[], &options).is_ok(), "{algorithm:?}, try hard {try_hard}");
        }
    }


    #[test]
    fn algorithm_follows_options() {
        let options = |algorithm, try_hard| ComparisonOptions { algorithm, try_hard, ..ComparisonOptions::default() };
        assert_eq!(super::algorithm(&options(DiffAlgorithm::Histogram, true)), Algorithm::Histogram);
        assert_eq!(super::algorithm(&options(DiffAlgorithm::Myers, false)), Algorithm::Myers);
        assert_eq!(super::algorithm(&options(DiffAlgorithm::Myers, true)), Algorithm::MyersMinimal);
    }


    #[test]
    fn canceled_comparison_stops() {
        let documents = Documents::new(&[&["a"], &["b"]]);
        let options = ComparisonOptions::default();
        let cancel = AtomicBool::new(true);
        let progress = |_: Progress| {};
        let result = BuiltinEngine.compare(&CompareInput { sources: documents.sources(), alignments: &[], options: &options }, &Job::new(&cancel, &progress));
        assert_eq!(result, Err(EngineError::Canceled));
    }


    #[test]
    fn reports_progress_per_step() {
        let documents = Documents::new(&[&["a"], &["b"], &["c"]]);
        let options = ComparisonOptions::default();
        let cancel = AtomicBool::new(false);
        let reports = Mutex::new(Vec::new());
        let progress = |progress: Progress| reports.lock().unwrap().push(progress.done);
        BuiltinEngine.compare(&CompareInput { sources: documents.sources(), alignments: &[], options: &options }, &Job::new(&cancel, &progress)).unwrap();
        assert_eq!(reports.into_inner().unwrap(), [1, 2, 3]);
    }


    #[test]
    fn classify_needs_two_or_three_hunks() {
        let lines = ["a"];
        let tokens = [Token(0)];
        let hunk = SourceLines { lines: &lines, tokens: &tokens };
        assert!(matches!(BuiltinEngine.classify(&[hunk]), Err(EngineError::InvalidInput(_))));
        assert_eq!(BuiltinEngine.classify(&[hunk.clone(), hunk]), Ok(UNCHANGED_2));
    }


    fn char_diff(row: &[Option<&str>]) -> Vec<Vec<CharRun>> {
        BuiltinEngine.char_diff(row, &ComparisonOptions::default()).unwrap().runs
    }


    fn run(columns: Range<usize>, same_in: &[bool]) -> CharRun {
        CharRun { columns, same_in: same_in.to_vec() }
    }


    #[test]
    fn char_diff_runs_cover_each_line() {
        assert_eq!(char_diff(&[Some("abcdef"), Some("abcdeg")]), [vec![run(0..5, &[true, true]), run(5..6, &[true, false])], vec![run(0..5, &[true, true]), run(5..6, &[false, true])]]);
        assert_eq!(char_diff(&[Some("abc"), Some("abc")]), [vec![run(0..3, &[true, true])], vec![run(0..3, &[true, true])]]);
        assert_eq!(char_diff(&[Some("abcdefgh"), Some("abcdXefgh")]), [vec![run(0..8, &[true, true])], vec![run(0..4, &[true, true]), run(4..5, &[false, true]), run(5..9, &[true, true])]]);
        assert_eq!(char_diff(&[Some(""), Some("a")]), [vec![], vec![run(0..1, &[false, true])]]);
    }


    #[test]
    fn char_diff_marks_lines_without_a_long_match_as_a_whole() {
        assert_eq!(char_diff(&[Some("abc"), Some("abd")]), [vec![run(0..3, &[true, false])], vec![run(0..3, &[false, true])]]);
    }


    #[test]
    fn char_diff_counts_characters_not_bytes() {
        assert_eq!(char_diff(&[Some("äöüßäöü"), Some("äöüßxäöü")])[1], [run(0..4, &[true, true]), run(4..5, &[false, true]), run(5..8, &[true, true])]);
    }


    #[test]
    fn char_diff_tells_which_documents_share_each_run() {
        let runs = char_diff(&[Some("abcdefg"), Some("xbcdefg"), Some("xbcdefz")]);
        assert_eq!(runs[0], [run(0..1, &[true, false, false]), run(1..6, &[true, true, true]), run(6..7, &[true, true, false])]);
        assert_eq!(runs[1], [run(0..1, &[false, true, true]), run(1..6, &[true, true, true]), run(6..7, &[true, true, false])]);
        assert_eq!(runs[2], [run(0..1, &[false, true, true]), run(1..6, &[true, true, true]), run(6..7, &[false, false, true])]);
    }


    #[test]
    fn char_diff_does_not_match_single_letters_of_renamed_words() {
        let runs = char_diff(&[Some("let total = price * quantity;"), Some("let total = cost * quantity;")]);
        assert_eq!(runs[0], [run(0..12, &[true, true]), run(12..17, &[true, false]), run(17..29, &[true, true])]);
        assert_eq!(runs[1], [run(0..12, &[true, true]), run(12..16, &[false, true]), run(16..28, &[true, true])]);
    }


    #[test]
    fn char_diff_of_three_changed_expressions_does_not_mark_the_space_all_have_after_the_equals_sign() {
        let runs = char_diff(&[Some("let total = price * quantity;"), Some("let total = cost * quantity;"), Some("let total = price * count + tax;")]);
        assert_eq!(runs[0], [run(0..12, &[true, true, true]), run(12..17, &[true, false, true]), run(17..20, &[true, true, true]), run(20..28, &[true, true, false]), run(28..29, &[true, true, true])]);
        assert_eq!(runs[1], [run(0..12, &[true, true, true]), run(12..16, &[false, true, false]), run(16..27, &[true, true, false]), run(27..28, &[true, true, true])]);
        assert_eq!(runs[2], [run(0..12, &[true, true, true]), run(12..20, &[true, false, true]), run(20..31, &[false, false, true]), run(31..32, &[true, true, true])]);
    }


    #[test]
    fn char_diff_diffs_c_against_a_not_a_against_c() {
        let runs = char_diff(&[Some("alpha a"), None, Some("a a")]);
        assert_eq!(runs, [vec![run(0..4, &[true, false, false]), run(4..7, &[true, false, true])], vec![], vec![run(0..3, &[true, false, true])]]);
    }


    #[test]
    fn char_diff_never_matches_voids() {
        assert_eq!(char_diff(&[None, Some("ab"), Some("ab")]), [vec![], vec![run(0..2, &[false, true, true])], vec![run(0..2, &[false, true, true])]]);
        assert_eq!(char_diff(&[Some("ab"), None]), [vec![run(0..2, &[true, false])], vec![]]);
    }


    #[test]
    fn char_diff_marks_long_lines_as_a_whole() {
        let options = ComparisonOptions { char_diff_line_limit: 3, ..ComparisonOptions::default() };
        let runs = BuiltinEngine.char_diff(&[Some("abcd"), Some("abce")], &options).unwrap().runs;
        assert_eq!(runs, [vec![run(0..4, &[true, false])], vec![run(0..4, &[false, true])]]);
    }


    #[test]
    fn capabilities() {
        let capabilities = BuiltinEngine.capabilities();
        assert_eq!(capabilities.id, BUILTIN_ENGINE_ID);
        assert!(capabilities.three_way && capabilities.char_diff && !capabilities.manual_alignment && !capabilities.align_b_with_c);
    }


    const ALPHABET: [&str; 4] = ["a", "b", "c", " c"];


    fn documents_with_alignments() -> impl Strategy<Value = (Vec<Vec<&'static str>>, Vec<ManualAlignment>)> {
        (2usize..=3, 0usize..3)
            .prop_flat_map(|(documents, count)| {
                vec(vec(select(ALPHABET.to_vec()), 0..15), documents).prop_flat_map(move |lines| {
                    let cuts: Vec<_> = lines.iter().map(|document| vec(0..=document.len(), count * 2)).collect();
                    (Just(lines), cuts)
                })
            })
            .prop_map(|(lines, mut cuts)| {
                cuts.iter_mut().for_each(|document| document.sort());
                let count = cuts.first().map(|document| document.len() / 2).unwrap_or(0);
                let alignments = (0..count)
                    .map(|index| ManualAlignment { parts: cuts.iter().map(|document| document[index * 2]..document[index * 2 + 1]).collect() })
                    .filter(|alignment| alignment.parts.iter().any(|part| !part.is_empty()))
                    .collect();
                (lines, alignments)
            })
    }


    proptest! {
        // Passing run_comparison means the result check accepted it: every line once, in order, alignments at region boundaries.
        #[test]
        fn results_are_valid_and_statuses_match_hunks((lines, alignments) in documents_with_alignments(), myers in any::<bool>()) {
            let documents: Vec<&[&'static str]> = lines.iter().map(|document| document.as_slice()).collect();
            let documents = Documents::new(&documents);
            let algorithm = if myers { DiffAlgorithm::Myers } else { DiffAlgorithm::Histogram };
            let options = ComparisonOptions { algorithm, ..ComparisonOptions::default() };
            let comparison = documents.compare_with(&alignments, &options);
            prop_assert!(comparison.is_ok(), "{comparison:?}");
            for region in comparison.unwrap().regions {
                let hunks: Vec<Vec<Token>> = region
                    .hunks
                    .iter()
                    .zip(&documents.tokens)
                    .map(|(hunk, tokens)| tokens[hunk.clone()].to_vec())
                    .collect();
                let all_equal = hunks.iter().all(|hunk| *hunk == hunks[0]);
                prop_assert_eq!(all_equal, region.status == UNCHANGED_2 || region.status == UNCHANGED_3);
            }
        }
    }
}
