use std::collections::HashMap;
use std::ops::Range;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use super::*;

/// Common leading and trailing lines become unchanged regions; everything between them is one region with lines paired by position.
pub struct SimpleEngine {
    pub three_way: bool,
    pub manual_alignment: bool,
    /// Breaks the result, to test the result check.
    pub drop_last_region: bool,
    /// Number of manual alignments each `compare` call was given.
    pub seen_alignments: Mutex<Vec<usize>>,
}

impl Default for SimpleEngine {
    fn default() -> SimpleEngine {
        SimpleEngine {
            three_way: true,
            manual_alignment: false,
            drop_last_region: false,
            seen_alignments: Mutex::new(Vec::new()),
        }
    }
}

impl ComparisonEngine for SimpleEngine {
    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            id: "simple".to_string(),
            version: "1".to_string(),
            three_way: self.three_way.clone(),
            manual_alignment: self.manual_alignment.clone(),
            char_diff: false,
            progress: false,
            algorithms: vec![DiffAlgorithm::Histogram],
            try_hard: false,
            align_b_with_c: false,
            pair_similar_lines: false,
        }
    }

    fn compare(&self, input: &CompareInput<'_>, job: &Job<'_>) -> Result<Comparison, EngineError> {
        job.check_canceled()?;
        self.seen_alignments.lock().expect("Test: poisoned").push(input.alignments.len());
        if !input.alignments.is_empty() {
            return alignment::compare_segments(self, input, job);
        }
        let counts = input.line_counts();
        let shortest = counts.iter().min().cloned().unwrap_or(0);
        let prefix = (0..shortest).take_while(|&line| same_token(input, |_| line)).count();
        let suffix = (0..shortest - prefix).take_while(|&offset| same_token(input, |count| count - 1 - offset)).count();
        let spans: [Vec<Range<usize>>; 3] = [
            counts.iter().map(|_| 0..prefix).collect(),
            counts.iter().map(|count| prefix..count - suffix).collect(),
            counts.iter().map(|count| count - suffix..count.clone()).collect(),
        ];
        let mut regions = Vec::new();
        for hunks in spans {
            if hunks.iter().any(|hunk| !hunk.is_empty()) {
                regions.push(self.region(input, hunks)?);
            }
        }
        if self.drop_last_region {
            regions.pop();
        }
        Ok(Comparison { regions })
    }

    fn classify(&self, hunks: &[SourceLines<'_>]) -> Result<RegionStatus, EngineError> {
        match hunks {
            [a, b] => Ok(RegionStatus::Diff(hunk_change(a, b))),
            [a, b, c] => {
                let status = match (a.tokens != b.tokens, a.tokens != c.tokens) {
                    (false, false) => MergeStatus::Unchanged,
                    (true, false) => MergeStatus::ChangedInB,
                    (false, true) => MergeStatus::ChangedInC,
                    _ if b.tokens == c.tokens => MergeStatus::ChangedAlike,
                    _ if without_whitespace(b) == without_whitespace(c) => MergeStatus::WhitespaceConflict,
                    _ => MergeStatus::Conflict,
                };
                Ok(RegionStatus::Merge(status))
            }
            _ => Err(EngineError::InvalidInput(format!("{} hunks", hunks.len()))),
        }
    }

    fn char_diff(&self, _row: &[Option<&str>], _options: &ComparisonOptions) -> Result<CharDiff, EngineError> {
        Err(EngineError::Unsupported("character-level diffs".to_string()))
    }
}

impl SimpleEngine {
    fn region(&self, input: &CompareInput<'_>, hunks: Vec<Range<usize>>) -> Result<Region, EngineError> {
        let lines: Vec<SourceLines<'_>> = input.sources.iter().zip(&hunks).map(|(source, hunk)| source.slice(hunk.clone())).collect();
        let status = self.classify(&lines)?;
        let height = hunks.iter().map(|hunk| hunk.len()).max().unwrap_or(0);
        let rows = (0..height)
            .map(|offset| Row {
                lines: hunks.iter().map(|hunk| (offset < hunk.len()).then(|| hunk.start + offset)).collect(),
            })
            .collect();
        Ok(Region { hunks, status, rows })
    }
}

fn same_token(input: &CompareInput<'_>, line_of: impl Fn(usize) -> usize) -> bool {
    let base = input.sources[0].tokens[line_of(input.sources[0].len())];
    input.sources.iter().all(|source| source.tokens[line_of(source.len())] == base)
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

fn without_whitespace(hunk: &SourceLines<'_>) -> Vec<String> {
    hunk.lines.iter().map(|line| line.split_whitespace().collect()).collect()
}

/// Source documents with tokens interned across all of them.
pub struct Fixture {
    pub lines: Vec<Vec<&'static str>>,
    pub tokens: Vec<Vec<Token>>,
    pub options: ComparisonOptions,
}

impl Fixture {
    pub fn new(documents: &[&[&'static str]]) -> Fixture {
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
        let lines = documents.iter().map(|document| document.to_vec()).collect();
        Fixture {
            lines,
            tokens,
            options: ComparisonOptions::default(),
        }
    }

    pub fn input<'a>(&'a self, alignments: &'a [ManualAlignment]) -> CompareInput<'a> {
        let sources = self.lines.iter().zip(&self.tokens).map(|(lines, tokens)| SourceLines { lines, tokens }).collect();
        CompareInput {
            sources,
            alignments,
            options: &self.options,
        }
    }
}

pub fn with_job<R>(run: impl FnOnce(&Job<'_>) -> R) -> R {
    let cancel = AtomicBool::new(false);
    let progress = |_: Progress| {};
    run(&Job::new(&cancel, &progress))
}
