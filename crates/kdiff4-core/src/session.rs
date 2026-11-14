use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use kdiff4_protocol::engine::{
    CompareInput, Comparison, ComparisonEngine, ComparisonOptions, EngineCapabilities, EngineError, HunkChange, Job, ManualAlignment, MergeStatus, Progress, Region, RegionStatus, SourceLines,
    run_comparison,
};
use kdiff4_protocol::{
    Alignments, Change, Choice, Document, DocumentText, EolChoice, EolStyle, HeuristicMatch, HeuristicOptions, HeuristicProposal, LineCharRuns, Navigation, OpenFiles, OutputDocument,
    OutputEdit, RegionMerge, Revision, Snapshot,
};

use crate::alignment_marks::AlignmentMarks;
use crate::error::CoreError;
use crate::folding;
use crate::heuristics::{self, DocumentLines, Proposed};
use crate::merge::{self, Merge, MergeStep, RegionState};
use crate::normalizer;
use crate::text::{self, SourceText};
use crate::undo::{History, Step};
use crate::whitespace_char_diff;

#[derive(Debug)]
struct Source {
    document: Document,
    path: PathBuf,
    label: Option<String>,
    text: SourceText,
}

impl Source {
    fn to_document_text(&self) -> DocumentText {
        DocumentText {
            document: self.document.clone(),
            path: self.path.clone(),
            label: self.label.clone(),
            eol: self.text.eol.clone(),
            text: self.text.text.clone(),
            lines: self.text.lines.clone(),
        }
    }

    fn lines(&self) -> Vec<&str> {
        self.text.lines.iter().map(|range| self.text.text.get(range.clone()).unwrap_or("")).collect()
    }
}

/// D. Its text uses `\n` line ends and is always the concatenation of the regions' texts.
#[derive(Debug)]
struct Output {
    path: Option<PathBuf>,
    text: String,
    /// What the sources give, or what the user chose over it.
    eol: EolChoice,
    explicit_final_eol: bool,
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
    /// The region navigation is at. Changes of D make their region current.
    current: Option<usize>,
    /// Per source document, the character-level diff of its lines; recomputed with the comparison.
    char_diffs: Vec<LineCharRuns>,
    /// The alignment marks and the alignments the comparison was computed with.
    alignments: AlignmentMarks,
}

impl Session {
    /// For a merge, every region is auto-resolved and D is built from the result. The first conflict is current, or the first difference when there are no conflicts.
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
            sources.push(Source {
                document,
                path: path.to_path_buf(),
                label,
                text,
            });
        }
        let options = ComparisonOptions::default();
        let comparison = compare(&sources, engine, &options, &[])?;
        let char_diffs = char_diffs(&sources, &comparison, engine, &options);
        let (merge, output) = if files.is_merge() {
            let (merge, text) = Merge::with_output(comparison.regions, &source_lines(&sources));
            (
                merge,
                Some(Output {
                    path: files.output.clone(),
                    text,
                    eol: auto_eol(&sources),
                    explicit_final_eol: false,
                    history: History::new_saved(),
                }),
            )
        } else {
            (Merge::without_output(comparison.regions), None)
        };
        let regions = merge.regions();
        let current = regions
            .iter()
            .position(|region| region.status.is_conflict())
            .or_else(|| regions.iter().position(|region| merge::is_difference(&region.status)));
        Ok(Session {
            revision: 1,
            sources,
            output,
            options,
            merge,
            engine: engine.capabilities(),
            current,
            char_diffs,
            alignments: AlignmentMarks::default(),
        })
    }

    pub fn revision(&self) -> Revision {
        Revision(self.revision.clone())
    }

    pub fn unresolved_count(&self) -> usize {
        self.merge.unresolved_count()
    }

    pub fn snapshot(&self) -> Snapshot {
        let documents = self.sources.iter().map(Source::to_document_text).collect();
        let output = self.output.as_ref().map(|output| OutputDocument {
            path: output.path.clone(),
            text: output.text.clone(),
            modified: output.history.is_modified(),
            eol: output.eol.clone(),
        });
        let comparison = Comparison {
            regions: self.merge.regions().to_vec(),
        };
        Snapshot {
            revision: self.revision(),
            documents,
            output,
            comparison,
            engine: self.engine.clone(),
            merge: self.merge.region_merges(),
            options: self.options.clone(),
            current_region: self.current.clone(),
            char_diffs: self.char_diffs.clone(),
            alignments: self.alignment_summary(),
        }
    }

    pub fn current_region(&self) -> Option<usize> {
        self.current.clone()
    }

    /// Replaces A, B or C with the file at `path` and recomputes the regions and rows. D is rebuilt from them, its choices and edits are dropped and its undo history starts over. The alignment marks and the applied alignments go too, since their lines moved. On failure the session is unchanged.
    pub fn load_source(&mut self, document: Document, path: &Path, engine: &dyn ComparisonEngine) -> Result<(), CoreError> {
        let index = self
            .sources
            .iter()
            .position(|source| source.document == document)
            .ok_or(CoreError::NoSuchDocument { document: document.clone() })?;
        let text = SourceText::load(path)?;
        let old_text = std::mem::replace(&mut self.sources[index].text, text);
        let comparison = match compare(&self.sources, engine, &self.options, &[]) {
            Ok(comparison) => comparison,
            Err(error) => {
                self.sources[index].text = old_text;
                return Err(error);
            }
        };
        self.sources[index].path = path.to_path_buf();
        self.alignments.reset();
        self.rebuild(comparison, engine);
        let eol = auto_eol(&self.sources);
        if let Some(output) = self.output.as_mut() {
            output.eol = eol;
        }
        Ok(())
    }

    /// Reads every source document again from its path and recomputes the regions and rows, like `load_source` for all of them at once (see `Command::Reload`). Without `confirmed`, a D that holds choices or edits is refused. On failure the session is unchanged.
    pub fn reload(&mut self, confirmed: bool, engine: &dyn ComparisonEngine) -> Result<(), CoreError> {
        if !confirmed && self.output.is_some() && self.merge.has_user_work() {
            return Err(CoreError::NeedsConfirmation);
        }
        let mut texts = Vec::with_capacity(self.sources.len());
        for source in &self.sources {
            texts.push(SourceText::load(&source.path)?);
        }
        let old_texts: Vec<SourceText> = self
            .sources
            .iter_mut()
            .zip(texts)
            .map(|(source, text)| std::mem::replace(&mut source.text, text))
            .collect();
        let comparison = match compare(&self.sources, engine, &self.options, &[]) {
            Ok(comparison) => comparison,
            Err(error) => {
                for (source, text) in self.sources.iter_mut().zip(old_texts) {
                    source.text = text;
                }
                return Err(error);
            }
        };
        self.alignments.reset();
        self.rebuild(comparison, engine);
        let eol = auto_eol(&self.sources);
        if let Some(output) = self.output.as_mut() {
            output.eol = eol;
        }
        Ok(())
    }

    /// Recomputes the regions and rows with `options` and the applied alignments, and rebuilds D from them, like a reload. When only options of character-level diffs change, just those are recomputed, and D keeps its choices, edits and undo history. On failure the session is unchanged.
    pub fn set_comparison_options(&mut self, options: ComparisonOptions, engine: &dyn ComparisonEngine) -> Result<(), CoreError> {
        let only_char_diffs = ComparisonOptions {
            ignore_whitespace: options.ignore_whitespace.clone(),
            ..self.options.clone()
        } == options;
        if only_char_diffs {
            let comparison = Comparison {
                regions: self.merge.regions().to_vec(),
            };
            self.char_diffs = char_diffs(&self.sources, &comparison, engine, &options);
            self.options = options;
            self.revision += 1;
            return Ok(());
        }
        let comparison = compare(&self.sources, engine, &options, self.alignments.applied())?;
        self.options = options;
        self.rebuild(comparison, engine);
        Ok(())
    }

    /// Moves the current region from `from` (the current region when `None`, the document's start or end when there is none either) to the next or previous region `to` looks for. Differences include hand-edited regions, so a line split out of a stable region by an edit can be reached, and, with `include_alignments`, the first region of every applied alignment. `Ok(None)` when there is none; the current region stays.
    pub fn navigate(&mut self, base: &Revision, from: Option<usize>, to: Navigation, include_alignments: bool) -> Result<Option<usize>, CoreError> {
        self.check_base(base)?;
        let regions = self.merge.regions();
        let states = self.merge.states();
        let count = regions.len();
        if let Some(index) = from
            && index >= count
        {
            return Err(CoreError::NoSuchRegion { index, count });
        }
        let from = from.or(self.current.clone()).filter(|index| *index < count);
        let stops = if include_alignments {
            alignment_stops(regions, self.alignments.applied())
        } else {
            Vec::new()
        };
        let wanted = |index: &usize| match to {
            Navigation::PreviousDifference | Navigation::NextDifference => merge::is_difference(&regions[*index].status) || states.get(*index).is_some_and(|state| state.merge.edited) || stops.contains(index),
            Navigation::PreviousConflict | Navigation::NextConflict => regions[*index].status.is_conflict(),
        };
        let found = match to {
            Navigation::NextDifference | Navigation::NextConflict => (from.map_or(0, |index| index + 1)..count).find(wanted),
            Navigation::PreviousDifference | Navigation::PreviousConflict => (0..from.unwrap_or(count)).rev().find(wanted),
        };
        if found.is_some() {
            self.current = found.clone();
        }
        Ok(found)
    }

    pub fn set_current_region(&mut self, base: &Revision, region: Option<usize>) -> Result<(), CoreError> {
        self.check_base(base)?;
        let count = self.merge.regions().len();
        if let Some(index) = region
            && index >= count
        {
            return Err(CoreError::NoSuchRegion { index, count });
        }
        self.current = region;
        Ok(())
    }

    /// Marks `lines` of `document` for alignment under the lowest number the document has not used. An empty range is an anchor. The comparison does not change until the alignments are applied. When the mark completes an alignment that the documents disagree on the order of, the answer says so (`Alignments::ask`): it is shown where the fewest documents keep their order until the user places it.
    pub fn mark_alignment(&mut self, document: Document, lines: Range<usize>) -> Result<Change, CoreError> {
        let number = self.alignments.mark(document.clone(), lines.clone(), &self.line_counts())?;
        log::info!("alignment: marked lines {}..{} of {} as alignment {number}", lines.start, lines.end, document.letter());
        let ask = self.alignments.is_inset(number, self.sources.len()).then_some(number);
        Ok(self.marks_changed(ask))
    }

    /// Marks the lines of the innermost function or method of `document` that holds `line` (see `Command::MarkFunctionAlignment`).
    pub fn mark_function_alignment(&mut self, document: Document, line: usize, language: &str) -> Result<Change, CoreError> {
        let source = self
            .sources
            .iter()
            .find(|source| source.document == document)
            .ok_or_else(|| CoreError::NoSuchDocument { document: document.clone() })?;
        let lines = heuristics::function_at(language, &DocumentLines { text: &source.text.text, lines: &source.text.lines }, line)?;
        let lines = lines.ok_or_else(|| CoreError::NoFunction { document: document.clone(), line: line.clone() })?;
        self.mark_alignment(document, lines)
    }

    pub fn remove_alignment_mark(&mut self, document: Document, number: u32) -> Result<Change, CoreError> {
        self.alignments.remove(document, number)?;
        Ok(self.marks_changed(None))
    }

    /// Moves a mark to other lines (see `Command::SetAlignmentMarkLines`). The comparison does not change until the alignments are applied.
    pub fn set_alignment_mark_lines(&mut self, document: Document, number: u32, lines: Range<usize>) -> Result<Change, CoreError> {
        self.alignments.set_lines(document, number, lines, &self.line_counts())?;
        Ok(self.marks_changed(None))
    }

    /// Gives a mark another number; a mark of the same document that has it takes the old one.
    pub fn renumber_alignment_mark(&mut self, document: Document, number: u32, to: u32) -> Result<Change, CoreError> {
        self.alignments.renumber(document, number, to)?;
        Ok(self.marks_changed(None))
    }

    /// Shows the alignment `number` at `slot` among the other alignments (see `Command::PlaceAlignment`). The comparison does not change until the alignments are applied.
    pub fn place_alignment(&mut self, number: u32, slot: usize) -> Result<Change, CoreError> {
        self.alignments.place(number, slot)?;
        log::info!("alignment: alignment {number} placed at {slot}");
        Ok(self.marks_changed(None))
    }

    /// Removes every mark; `None` when there were none. The applied alignments stay until the next apply.
    pub fn clear_alignment_marks(&mut self) -> Option<Change> {
        if self.alignments.summary(self.sources.len()).marks.is_empty() {
            return None;
        }
        self.alignments.clear();
        Some(self.marks_changed(None))
    }

    /// Compares again with the alignments the marks give (see `Command::ApplyAlignments`), rebuilding D like a reload. Returns whether anything changed: `false` when the marks give the alignments already applied. Without `confirmed`, a D that holds choices or edits is refused. On failure the session is unchanged.
    pub fn apply_alignments(&mut self, confirmed: bool, engine: &dyn ComparisonEngine) -> Result<bool, CoreError> {
        let alignments = self.alignments.to_apply(&self.line_counts())?;
        if alignments == self.alignments.applied() {
            return Ok(false);
        }
        if !confirmed && self.output.is_some() && self.merge.has_user_work() {
            return Err(CoreError::NeedsConfirmation);
        }
        let comparison = compare(&self.sources, engine, &self.options, &alignments)?;
        log::info!("alignment: applied {} alignments", alignments.len());
        self.alignments.set_applied(alignments);
        self.rebuild(comparison, engine);
        Ok(true)
    }

    /// Finds the functions, methods, types and blocks of the source documents and pairs them (see `Command::ProposeHeuristicAlignments`). Matches that collide with a mark the user made are left out with the matches inside them, and counted. Changes nothing.
    /// The fold ranges of each of `documents` the session has, in the order asked.
    pub fn fold_ranges(&self, language: &str, documents: &[Document]) -> Vec<(Document, Vec<Range<usize>>)> {
        documents
            .iter()
            .filter_map(|document| {
                let ranges = match document {
                    Document::D => {
                        let output = self.output.as_ref()?;
                        folding::fold_ranges(language, &output.text, &text::split_lines(&output.text))
                    }
                    _ => {
                        let source = self.sources.iter().find(|source| source.document == *document)?;
                        folding::fold_ranges(language, &source.text.text, &source.text.lines)
                    }
                };
                Some((document.clone(), ranges))
            })
            .collect()
    }


    pub fn propose_heuristic_alignments(&self, language: &str, options: &HeuristicOptions, within: Option<u32>) -> Result<HeuristicProposal, CoreError> {
        let scope = match within {
            Some(number) => Some(self.alignments.applied_parts(number, self.sources.len()).ok_or(CoreError::NotApplied { number })?),
            None => None,
        };
        let documents: Vec<DocumentLines<'_>> = self
            .sources
            .iter()
            .map(|source| DocumentLines {
                text: &source.text.text,
                lines: &source.text.lines,
            })
            .collect();
        let (proposed, found) = heuristics::propose(language, &documents, options, scope.as_deref())?;
        let mut kept: Vec<Proposed> = Vec::with_capacity(proposed.len());
        let mut new_index: Vec<Option<usize>> = Vec::with_capacity(proposed.len());
        let mut blocked = 0;
        for mut entry in proposed {
            let parent_kept = entry.parent.is_none_or(|parent| new_index[parent].is_some());
            if !parent_kept || self.alignments.blocks(&entry.parts) {
                new_index.push(None);
                blocked += 1;
                continue;
            }
            entry.parent = entry.parent.and_then(|parent| new_index[parent]);
            new_index.push(Some(kept.len()));
            kept.push(entry);
        }
        let mut trial = self.alignments.clone();
        let parts: Vec<Vec<Range<usize>>> = kept.iter().map(|proposed| proposed.parts.clone()).collect();
        let numbers = match within {
            Some(number) => trial.replace_heuristic_within(&parts, &self.line_counts(), number)?,
            None => trial.replace_heuristic(&parts, &self.line_counts())?,
        };
        let summary = trial.summary(self.sources.len());
        let moved = |number: &Option<u32>| number.is_some_and(|number| summary.marks.iter().any(|mark| mark.number == number && mark.is_moved()));
        let matches = kept
            .into_iter()
            .zip(numbers.iter())
            .map(|(proposed, number)| HeuristicMatch {
                kind: proposed.kind,
                names: proposed.names,
                parts: proposed.parts,
                stage: proposed.stage,
                parent: proposed.parent,
                moved: moved(number),
            })
            .collect();
        Ok(HeuristicProposal {
            cause: None,
            matches,
            found,
            blocked,
        })
    }

    /// Replaces the heuristic marks with `alignments`, or with `within` only the ones nested inside that alignment, and applies all marks (see `Command::ApplyHeuristicAlignments`). Returns whether the comparison changed; when it did not, the marks may still have. On failure the marks are unchanged.
    pub fn apply_heuristic_alignments(&mut self, alignments: &[Vec<Range<usize>>], within: Option<u32>, confirmed: bool, engine: &dyn ComparisonEngine) -> Result<bool, CoreError> {
        let saved = self.alignments.clone();
        let line_counts = self.line_counts();
        let replaced = match within {
            Some(number) => self.alignments.replace_heuristic_within(alignments, &line_counts, number),
            None => self.alignments.replace_heuristic(alignments, &line_counts),
        };
        let result = replaced.and_then(|_| self.apply_alignments(confirmed, engine));
        if result.is_err() {
            self.alignments = saved;
        }
        result
    }

    /// The change of the alignment marks, for a command that changed them without comparing again.
    pub fn alignment_marks_change(&mut self) -> Change {
        self.marks_changed(None)
    }

    fn alignment_summary(&self) -> Alignments {
        self.alignments.summary(self.sources.len())
    }

    fn marks_changed(&mut self, ask: Option<u32>) -> Change {
        self.revision += 1;
        Change::AlignmentsChanged(Alignments { ask, ..self.alignment_summary() })
    }

    fn line_counts(&self) -> Vec<usize> {
        self.sources.iter().map(|source| source.text.lines.len()).collect()
    }

    /// Takes a new comparison: D is rebuilt from its regions, its choices and edits are dropped and its undo history starts over.
    fn rebuild(&mut self, comparison: Comparison, engine: &dyn ComparisonEngine) {
        self.engine = engine.capabilities();
        self.current = None;
        self.char_diffs = char_diffs(&self.sources, &comparison, engine, &self.options);
        match self.output.as_mut() {
            Some(output) => {
                let (merge, text) = Merge::with_output(comparison.regions, &source_lines(&self.sources));
                self.merge = merge;
                output.text = text;
                output.explicit_final_eol = false;
                output.history = History::new_unsaved();
            }
            None => self.merge = Merge::without_output(comparison.regions),
        }
        self.revision += 1;
    }

    /// A hand edit of D. The regions it touches become one edited region; several are joined, and `engine` classifies the result.
    pub fn edit_output(&mut self, edit: &OutputEdit, engine: &dyn ComparisonEngine) -> Result<Change, CoreError> {
        self.check_base(&edit.base)?;
        let output = self.output.as_ref().ok_or(CoreError::NotAMerge)?;
        let range = edit.range.clone();
        let fits = range.start <= range.end && output.text.is_char_boundary(range.start) && output.text.is_char_boundary(range.end) && range.end <= output.text.len();
        if !fits {
            return Err(CoreError::InvalidRange {
                start: range.start,
                end: range.end,
            });
        }
        let inserted = text::normalize_to_lf(&edit.text);
        if self.merge.regions().is_empty() {
            let region = self.empty_region();
            let state = RegionState {
                merge: RegionMerge {
                    choice: Choice::Unresolved,
                    edited: true,
                    output_lines: merge::line_count(&inserted),
                },
                text_len: inserted.len(),
            };
            return Ok(self.apply(
                range,
                inserted,
                MergeStep {
                    at: 0,
                    before: Vec::new(),
                    after: vec![(region, state)],
                    explicit_final_eol: None,
                },
            ));
        }
        let touched = match edit.region {
            Some(region) => self.merge.empty_target(region, range.clone(), &inserted).ok_or(CoreError::InvalidEditTarget { region })?,
            None => self.merge.touched(range.clone(), &inserted, &output.text),
        };
        let offsets = self.merge.offsets();
        let span = offsets[touched.start]..offsets[touched.end];
        let mut region_text = output.text[span.start..range.start].to_string();
        region_text.push_str(&inserted);
        region_text.push_str(&output.text[range.end..span.end]);
        let before = self.merge.entries(touched.clone());
        if let [(region, state)] = before.as_slice() {
            let relative = range.start - span.start..range.end - span.start;
            let split = merge::split_stable(region, state, &output.text[span.clone()], relative, &inserted);
            if let Some(after) = split {
                return Ok(self.apply(range, inserted, MergeStep { at: touched.start, before, after, explicit_final_eol: None }));
            }
        }
        let (region, choice) = match before.as_slice() {
            [(region, state)] => (region.clone(), state.merge.choice.clone()),
            _ => (
                merge::join(&self.merge.regions()[touched.clone()], &source_lines(&self.sources), engine)?,
                Choice::Unresolved,
            ),
        };
        let state = RegionState {
            merge: RegionMerge {
                choice,
                edited: true,
                output_lines: merge::line_count(&region_text),
            },
            text_len: region_text.len(),
        };
        Ok(self.apply(
            range,
            inserted,
            MergeStep {
                at: touched.start,
                before,
                after: vec![(region, state)],
                explicit_final_eol: None,
            },
        ))
    }

    /// Sets the choice of one region and drops its hand edits. A region whose hunks are equal is joined back with unedited neighbors like it, undoing the split of a hand edit.
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
        let state = RegionState {
            merge: RegionMerge {
                choice,
                edited: false,
                output_lines: merge::line_count(&text),
            },
            text_len: text.len(),
        };
        let mut entry = (region, state);
        let mut text = text;
        let mut first = index;
        let mut last = index;
        if index > 0 {
            let previous = self.merge.entries(index - 1..index).remove(0);
            if merge::joins_stable(&previous, &entry) {
                let output = self.output.as_ref().ok_or(CoreError::NotAMerge)?;
                text = format!("{}{}", &output.text[offsets[index - 1]..offsets[index]], text);
                entry = merge::join_stable(&previous, &entry);
                first = index - 1;
            }
        }
        if index + 1 < count {
            let next = self.merge.entries(index + 1..index + 2).remove(0);
            if merge::joins_stable(&entry, &next) {
                let output = self.output.as_ref().ok_or(CoreError::NotAMerge)?;
                text.push_str(&output.text[offsets[index + 1]..offsets[index + 2]]);
                entry = merge::join_stable(&entry, &next);
                last = index + 1;
            }
        }
        let step = MergeStep {
            at: first,
            before: self.merge.entries(first..last + 1),
            after: vec![entry],
            explicit_final_eol: None,
        };
        Ok(self.apply_choice(offsets[first]..offsets[last + 1], text, step))
    }

    /// Chooses the hunk of `document` for every unresolved region, as one undo step. `Ok(None)` when no region is unresolved.
    pub fn choose_for_unresolved(&mut self, base: &Revision, document: Document) -> Result<Option<Change>, CoreError> {
        self.check_base(base)?;
        let output = self.output.as_ref().ok_or(CoreError::NotAMerge)?;
        let choice = Choice::Hunks(vec![document]);
        merge::validate_choice(&choice, self.sources.len())?;
        let unresolved: Vec<usize> = self
            .merge
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
                after.push((
                    region.clone(),
                    RegionState {
                        merge: RegionMerge {
                            choice: choice.clone(),
                            edited: false,
                            output_lines: merge::line_count(&rendered),
                        },
                        text_len: rendered.len(),
                    },
                ));
                text.push_str(&rendered);
            } else {
                after.push((region.clone(), state.clone()));
                text.push_str(&output.text[offsets[index]..offsets[index + 1]]);
            }
        }
        Ok(Some(self.apply_choice(offsets[first]..offsets[last + 1], text, MergeStep { at: first, before, after, explicit_final_eol: None })))
    }

    /// Replaces D with the chosen source as one undo step. Hand edits require confirmation for this revision.
    pub fn choose_everywhere(&mut self, base: &Revision, document: Document, confirmed: bool, engine: &dyn ComparisonEngine) -> Result<Option<Change>, CoreError> {
        self.check_base(base)?;
        let output = self.output.as_ref().ok_or(CoreError::NotAMerge)?;
        let source_index = self.sources.iter().position(|source| source.document == document).ok_or(CoreError::NoSuchDocument { document })?;
        if !confirmed && self.merge.states().iter().any(|state| state.merge.edited) {
            return Err(CoreError::NeedsConfirmation);
        }

        let choice = Choice::Hunks(vec![document]);
        let source = &self.sources[source_index].text;
        let lines = source_lines(&self.sources);
        let regions = self.merge.regions();
        let before = self.merge.entries(0..regions.len());
        let mut end = 0;
        let in_source_order = regions.iter().map(|region| &region.hunks[source_index]).filter(|hunk| !hunk.is_empty()).all(|hunk| {
            let adjacent = hunk.start == end;
            end = hunk.end;
            adjacent
        });
        // Insets can display hunks out of source order. Join the regions so D can take the source in its original order.
        let joined;
        let regions = if in_source_order && end == source.lines.len() {
            regions
        } else {
            joined = [merge::join(regions, &lines, engine)?];
            &joined
        };

        let mut text = String::with_capacity(source.text.len());
        let mut after = Vec::with_capacity(regions.len());
        for region in regions {
            let hunk = &region.hunks[source_index];
            let mut rendered = merge::render(region, &choice, &lines);
            if !hunk.is_empty() && hunk.end == source.lines.len() && !source.final_eol {
                rendered.pop();
            }
            let state = RegionState {
                merge: RegionMerge { choice: choice.clone(), edited: false, output_lines: merge::line_count(&rendered) },
                text_len: rendered.len(),
            };
            text.push_str(&rendered);
            after.push((region.clone(), state));
        }
        if before == after && output.text == text && (output.explicit_final_eol || regions.is_empty()) {
            return Ok(None);
        }
        let explicit_final_eol = Some((output.explicit_final_eol, true));
        Ok(Some(self.apply(0..output.text.len(), text, MergeStep { at: 0, before, after, explicit_final_eol })))
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
        if let Some((before, _)) = data.explicit_final_eol {
            output.explicit_final_eol = before;
        }
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
        if let Some((_, after)) = data.explicit_final_eol {
            output.explicit_final_eol = after;
        }
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

    /// Chooses the line ending style D is saved with, which settles a conflict between the sources. It stays until a source is loaded again.
    pub fn set_output_eol(&mut self, eol: EolStyle) -> Result<Change, CoreError> {
        let output = self.output.as_mut().ok_or(CoreError::NotAMerge)?;
        output.eol = EolChoice::Style(eol);
        self.revision += 1;
        Ok(Change::OutputEolChanged(EolChoice::Style(eol)))
    }

    pub fn save(&mut self) -> Result<Change, CoreError> {
        let path = self.output.as_ref().ok_or(CoreError::NotAMerge)?.path.clone().ok_or(CoreError::NoOutputPath)?;
        self.write_output(&path)
    }

    /// The output keeps its old path if the write fails.
    pub fn save_as(&mut self, path: &Path) -> Result<Change, CoreError> {
        self.write_output(path)
    }

    /// Writes D with the EOL style chosen for it, and the BOM and final line break of A, the output reference. Refused while unresolved conflicts remain or the line ending style is in conflict.
    fn write_output(&mut self, path: &Path) -> Result<Change, CoreError> {
        if path.is_dir() {
            return Err(CoreError::Directory { path: path.to_path_buf() });
        }
        self.output.as_ref().ok_or(CoreError::NotAMerge)?;
        let unresolved = self.merge.unresolved_count();
        if unresolved > 0 {
            return Err(CoreError::UnresolvedConflicts { count: unresolved });
        }
        let (bom, final_eol) = self
            .sources
            .first()
            .map(|source| (source.text.bom.clone(), source.text.final_eol.clone()))
            .unwrap_or((false, true));
        let output = self.output.as_mut().ok_or(CoreError::NotAMerge)?;
        let EolChoice::Style(eol) = output.eol.clone() else {
            return Err(CoreError::EolConflict);
        };
        let bytes = text::encode_output(&output.text, eol, bom, final_eol || output.explicit_final_eol);
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
        Region {
            hunks: vec![0..0; self.sources.len()],
            status,
            rows: Vec::new(),
        }
    }

    /// A hunk appended after an unterminated final line starts on its own line.
    fn apply_choice(&mut self, range: Range<usize>, mut text: String, mut step: MergeStep) -> Change {
        if !text.is_empty()
            && self.output.as_ref().is_some_and(|output| range.start == output.text.len() && !output.text.ends_with('\n'))
            && let Some(previous) = self.merge.states()[..step.at].iter().rposition(|state| state.text_len > 0)
        {
            let mut before = self.merge.entries(previous..step.at);
            let mut after = before.clone();
            after[0].1.text_len += 1;
            text.insert(0, '\n');
            before.extend(step.before);
            after.extend(step.after);
            step.at = previous;
            step.before = before;
            step.after = after;
        }
        self.apply(range, text, step)
    }


    /// Makes a merge op: replaces `range` of D with `inserted` and the regions of `step.before` with `step.after`, and records it for undo.
    fn apply(&mut self, range: Range<usize>, inserted: String, step: MergeStep) -> Change {
        let at = step.at.clone();
        let removed = step.before.len();
        let added = step.after.len();
        self.merge.splice(at.clone(), removed.clone(), step.after.clone());
        if let Some(output) = self.output.as_mut() {
            if let Some((_, after)) = step.explicit_final_eol {
                output.explicit_final_eol = after;
            }
            let removed_text = output.text[range.clone()].to_string();
            output.history.record(Step {
                start: range.start,
                removed: removed_text,
                inserted: inserted.clone(),
                data: step,
            });
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
        self.current = Some(at.clone());
        Change::OutputEdited {
            range,
            text: inserted,
            modified,
            merge: self.merge.update(at, removed, added),
        }
    }
}

/// For every alignment, the first region that has lines inside it. Regions joined across an alignment's edge still count.
fn alignment_stops(regions: &[Region], alignments: &[ManualAlignment]) -> Vec<usize> {
    alignments
        .iter()
        .filter_map(|alignment| {
            let (document, part) = alignment.parts.iter().enumerate().find(|(_, part)| !part.is_empty())?;
            regions
                .iter()
                .position(|region| region.hunks.get(document).is_some_and(|hunk| hunk.start < part.end && part.start < hunk.end))
        })
        .collect()
}

/// The style all sources with a line terminator share; LF when none has one. A conflict when two of them differ, even if a third agrees with one: kdiff3 takes the odd one out then.
fn auto_eol(sources: &[Source]) -> EolChoice {
    let mut styles = sources.iter().filter_map(|source| source.text.eol.clone());
    let Some(first) = styles.next() else {
        return EolChoice::Style(EolStyle::Lf);
    };
    if styles.all(|style| style == first) { EolChoice::Style(first) } else { EolChoice::Conflict }
}

fn source_lines(sources: &[Source]) -> Vec<Vec<&str>> {
    sources.iter().map(Source::lines).collect()
}

/// Compares the source documents with `engine`. Runs to completion: cancelable worker jobs come later.
fn compare(sources: &[Source], engine: &dyn ComparisonEngine, options: &ComparisonOptions, alignments: &[ManualAlignment]) -> Result<Comparison, CoreError> {
    let lines = source_lines(sources);
    let tokens = normalizer::tokenize(&lines);
    let input = CompareInput {
        sources: lines.iter().zip(&tokens).map(|(lines, tokens)| SourceLines { lines, tokens }).collect(),
        alignments,
        options,
    };
    let cancel = AtomicBool::new(false);
    let progress = |progress: Progress| log::debug!("compare: step {} of {}", progress.done, progress.total);
    let comparison = run_comparison(engine, &input, &Job::new(&cancel, &progress))?;
    log::info!("compare: {} regions with {}", comparison.regions.len(), engine.capabilities().id);
    Ok(comparison)
}

/// The character-level diff of every row whose lines differ, keyed by source line. An engine without character-level diffs, or one that fails, leaves every line unmarked.
fn char_diffs(sources: &[Source], comparison: &Comparison, engine: &dyn ComparisonEngine, options: &ComparisonOptions) -> Vec<LineCharRuns> {
    let lines = source_lines(sources);
    let unmarked = || -> Vec<LineCharRuns> {
        lines
            .iter()
            .map(|lines| LineCharRuns {
                lines: vec![Vec::new(); lines.len()],
            })
            .collect()
    };
    let mut diffs = unmarked();
    if !engine.capabilities().char_diff {
        return diffs;
    }
    if options.ignore_whitespace {
        return whitespace_char_diffs(&lines, comparison, engine, options).unwrap_or_else(|error| {
            log::warn!("char diff: {error}; lines are left unmarked");
            unmarked()
        });
    }
    for row in comparison.regions.iter().flat_map(|region| region.rows.iter()) {
        let texts: Vec<Option<&str>> = row
            .lines
            .iter()
            .enumerate()
            .map(|(document, line)| line.and_then(|line| lines.get(document)?.get(line).copied()))
            .collect();
        let all_equal = texts.iter().all(|text| text.is_some() && *text == texts[0]);
        if all_equal {
            continue;
        }
        let diff = match engine.char_diff(&texts, options) {
            Ok(diff) => diff,
            Err(error) => {
                log::warn!("char diff: {error}; lines are left unmarked");
                return unmarked();
            }
        };
        if diff.runs.len() != texts.len() {
            log::warn!("char diff: {} lines for a row of {}; lines are left unmarked", diff.runs.len(), texts.len());
            return unmarked();
        }
        for (document, runs) in diff.runs.into_iter().enumerate() {
            let slot = row.lines[document].clone().and_then(|line| diffs.get_mut(document)?.lines.get_mut(line));
            if let Some(slot) = slot {
                *slot = runs;
            }
        }
    }
    diffs
}

/// The character-level diffs with whitespace ignored: each region whose hunks differ is diffed as a whole, or row by row when it is longer than the char-diff line limit.
fn whitespace_char_diffs(lines: &[Vec<&str>], comparison: &Comparison, engine: &dyn ComparisonEngine, options: &ComparisonOptions) -> Result<Vec<LineCharRuns>, EngineError> {
    let mut diffs: Vec<LineCharRuns> = lines
        .iter()
        .map(|lines| LineCharRuns {
            lines: vec![Vec::new(); lines.len()],
        })
        .collect();
    for region in &comparison.regions {
        let hunks: Vec<Vec<(usize, &str)>> = region
            .hunks
            .iter()
            .enumerate()
            .map(|(document, hunk)| hunk.clone().filter_map(|line| Some((line.clone(), lines.get(document)?.get(line).copied()?))).collect())
            .collect();
        if hunks.iter().all(|hunk| texts(hunk) == texts(&hunks[0])) {
            continue;
        }
        let chunks: Vec<Vec<Vec<(usize, &str)>>> = if whitespace_char_diff::exceeds_limit(&hunks.iter().map(|hunk| texts(hunk)).collect::<Vec<_>>(), options) {
            region
                .rows
                .iter()
                .map(|row| {
                    row.lines
                        .iter()
                        .enumerate()
                        .map(|(document, line)| line.and_then(|line| Some((line.clone(), lines.get(document)?.get(line).copied()?))).into_iter().collect())
                        .collect()
                })
                .collect()
        } else {
            vec![hunks]
        };
        for chunk in chunks {
            let runs = whitespace_char_diff::chunk_runs(engine, &chunk.iter().map(|part| texts(part)).collect::<Vec<_>>(), options)?;
            for (document, (part, runs)) in chunk.iter().zip(runs).enumerate() {
                for ((line, _), runs) in part.iter().zip(runs) {
                    if let Some(slot) = diffs.get_mut(document).and_then(|diffs| diffs.lines.get_mut(line.clone())) {
                        *slot = runs;
                    }
                }
            }
        }
    }
    Ok(diffs)
}


fn texts<'a>(part: &[(usize, &'a str)]) -> Vec<&'a str> {
    part.iter().map(|(_, text)| *text).collect()
}


#[cfg(test)]
mod tests {
    use std::fs;

    use kdiff4_protocol::engine::{CharDiff, HunkChange, MergeStatus, RegionStatus, Row};
    use kdiff4_protocol::{EntityKind, ErrorKind, Labels, MarkStatus, ProtocolError};

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
        OpenFiles {
            a,
            b,
            c,
            output,
            labels: Labels::default(),
        }
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
            assert_eq!(
                merge::line_count(&text[offsets[index]..offsets[index + 1]]),
                state.merge.output_lines,
                "line count of region {index}"
            );
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
        session.snapshot().merge.iter().map(|merge| merge.choice.clone()).collect()
    }

    fn edit(session: &Session, range: std::ops::Range<usize>, text: &str) -> OutputEdit {
        OutputEdit {
            base: session.revision(),
            range,
            text: text.to_string(),
            region: None,
        }
    }

    /// Fails every comparison, so a reload with it must leave the session as it was.
    struct FailingEngine;

    impl ComparisonEngine for FailingEngine {
        fn capabilities(&self) -> EngineCapabilities {
            EngineCapabilities {
                id: "failing".to_string(),
                ..BuiltinEngine.capabilities()
            }
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
        comparison.regions.iter().map(|region| region.status.clone()).collect()
    }

    #[test]
    fn open_compares_sources() {
        let dir = temp_dir("open-compares");
        let a = write(&dir, "a.txt", "same\nbase\n");
        let b = write(&dir, "b.txt", "same\nleft\n");
        let c = write(&dir, "c.txt", "same\nbase\n");
        let snapshot = Session::open(&files(a, b, Some(c), None), &BuiltinEngine).unwrap().snapshot();
        assert_eq!(
            statuses(&snapshot.comparison),
            [RegionStatus::Merge(MergeStatus::Unchanged), RegionStatus::Merge(MergeStatus::ChangedInB)]
        );
        assert_eq!(
            snapshot.comparison.regions[1].rows,
            [Row {
                lines: vec![Some(1), Some(1), Some(1)]
            }]
        );
        assert_eq!(snapshot.engine.id, BUILTIN_ENGINE_ID);
    }

    #[test]
    fn snapshot_has_char_diffs_of_rows_that_differ() {
        let dir = temp_dir("char-diffs");
        let session = merge_of(&dir, &["same", "abcdefg"], &["same", "xbcdefg"], &["same", "xbcdefz"]);
        let snapshot = session.snapshot();
        assert_eq!(snapshot.char_diffs.len(), 3);
        let same_in = |document: usize, line: usize| -> Vec<Vec<bool>> { snapshot.char_diffs[document].line(line).iter().map(|run| run.same_in.clone()).collect() };
        assert!(snapshot.char_diffs.iter().all(|diff| diff.line(0).is_empty()));
        assert_eq!(same_in(1, 1), [vec![false, true, true], vec![true, true, true], vec![true, true, false]]);
        assert_eq!(snapshot.char_diffs[2].line(1)[2].columns, 6..7);
    }

    #[test]
    fn char_diffs_survive_edits_of_d_and_follow_new_comparisons() {
        let dir = temp_dir("char-diffs-edits");
        let mut session = merge_of(&dir, &["abc"], &["abd"], &["abc"]);
        let before = session.snapshot().char_diffs;
        assert!(!before[1].line(0).is_empty());
        let change = session.edit_output(&edit(&session, 0..0, "typed"), &BuiltinEngine).unwrap();
        assert!(matches!(change, Change::OutputEdited { .. }));
        assert_eq!(session.snapshot().char_diffs, before);
        let b = write(&dir, "b2.txt", "abc\n");
        session.load_source(Document::B, &b, &BuiltinEngine).unwrap();
        assert!(session.snapshot().char_diffs.iter().all(|diff| diff.line(0).is_empty()));
    }

    #[test]
    fn engines_without_char_diffs_leave_lines_unmarked() {
        let dir = temp_dir("char-diffs-unsupported");
        let a = write(&dir, "a.txt", "abc\n");
        let b = write(&dir, "b.txt", "abd\n");
        struct NoCharDiffs;
        impl ComparisonEngine for NoCharDiffs {
            fn capabilities(&self) -> EngineCapabilities {
                EngineCapabilities {
                    char_diff: false,
                    ..BuiltinEngine.capabilities()
                }
            }
            fn compare(&self, input: &CompareInput<'_>, job: &Job<'_>) -> Result<Comparison, EngineError> {
                BuiltinEngine.compare(input, job)
            }
            fn classify(&self, hunks: &[SourceLines<'_>]) -> Result<RegionStatus, EngineError> {
                BuiltinEngine.classify(hunks)
            }
            fn char_diff(&self, _row: &[Option<&str>], _options: &ComparisonOptions) -> Result<CharDiff, EngineError> {
                panic!("char_diff called on an engine without character-level diffs")
            }
        }
        let snapshot = Session::open(&files(a, b, None, None), &NoCharDiffs).unwrap().snapshot();
        assert_eq!(snapshot.char_diffs.len(), 2);
        assert!(snapshot.char_diffs.iter().all(|diff| diff.line(0).is_empty()));
    }

    #[test]
    fn two_way_open_compares_a_and_b() {
        let dir = temp_dir("open-two-way");
        let a = write(&dir, "a.txt", "x\n");
        let b = write(&dir, "b.txt", "x\ny\n");
        let snapshot = Session::open(&files(a, b, None, None), &BuiltinEngine).unwrap().snapshot();
        assert_eq!(
            statuses(&snapshot.comparison),
            [RegionStatus::Diff(HunkChange::Unchanged), RegionStatus::Diff(HunkChange::Added)]
        );
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
        let labels = Labels {
            a: Some("Base".to_string()),
            b: Some("Local".to_string()),
            c: None,
        };
        let session = Session::open(
            &OpenFiles {
                a,
                b,
                c: None,
                output: None,
                labels,
            },
            &BuiltinEngine,
        )
        .unwrap();
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
        assert_eq!(
            output,
            OutputDocument {
                path: None,
                text: "a\n".to_string(),
                modified: false,
                eol: EolChoice::Style(EolStyle::Lf)
            }
        );
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
        assert!(matches!(
            session.load_source(Document::C, &a, &BuiltinEngine),
            Err(CoreError::NoSuchDocument { document: Document::C })
        ));
        assert!(matches!(
            session.load_source(Document::D, &a, &BuiltinEngine),
            Err(CoreError::NoSuchDocument { document: Document::D })
        ));
        assert!(matches!(
            session.load_source(Document::B, &dir.join("gone.txt"), &BuiltinEngine),
            Err(CoreError::Read { .. })
        ));
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
        let stale = OutputEdit {
            base: Revision(1),
            range: 0..0,
            text: "x".to_string(),
            region: None,
        };
        assert!(matches!(session.edit_output(&stale, &BuiltinEngine), Err(CoreError::StaleRevision { .. })));
        assert!(matches!(
            session.edit_output(&edit(&session, 1..1, "x"), &BuiltinEngine),
            Err(CoreError::InvalidRange { .. })
        ));
        assert!(matches!(
            session.edit_output(&edit(&session, 0..9, "x"), &BuiltinEngine),
            Err(CoreError::InvalidRange { .. })
        ));
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

    /// A 3-way merge of the texts as written, with the output at `out.txt`.
    fn merge_of_texts(dir: &Path, a: &str, b: &str, c: &str) -> Session {
        let (a, b, c) = (write(dir, "a.txt", a), write(dir, "b.txt", b), write(dir, "c.txt", c));
        Session::open(&files(a, b, Some(c), Some(dir.join("out.txt"))), &BuiltinEngine).unwrap()
    }

    fn output_eol(session: &Session) -> EolChoice {
        session.snapshot().output.unwrap().eol
    }

    #[test]
    fn output_eol_is_the_style_the_sources_share() {
        let dir = temp_dir("eol-shared");
        assert_eq!(output_eol(&merge_of_texts(&dir, "a\r\n", "a\r\n", "a\r\n")), EolChoice::Style(EolStyle::CrLf));
        assert_eq!(output_eol(&merge_of_texts(&dir, "a\r", "a\r", "a\r")), EolChoice::Style(EolStyle::Cr));
        assert_eq!(output_eol(&merge_of_texts(&dir, "a", "a", "a")), EolChoice::Style(EolStyle::Lf));
    }

    #[test]
    fn sources_without_a_terminator_do_not_conflict() {
        let dir = temp_dir("eol-undefined");
        assert_eq!(output_eol(&merge_of_texts(&dir, "a\r\nb", "a\r\nb", "single")), EolChoice::Style(EolStyle::CrLf));
        assert_eq!(output_eol(&merge_of_texts(&dir, "", "a\n", "b")), EolChoice::Style(EolStyle::Lf));
    }

    #[test]
    fn output_eol_conflicts_when_any_two_sources_differ() {
        let dir = temp_dir("eol-conflict");
        assert_eq!(output_eol(&merge_of_texts(&dir, "a\r\n", "a\n", "a\n")), EolChoice::Conflict);
        assert_eq!(output_eol(&merge_of_texts(&dir, "a\n", "a\n", "a\r\n")), EolChoice::Conflict);
        assert_eq!(output_eol(&merge_of_texts(&dir, "a\n", "a\r", "a\r\n")), EolChoice::Conflict);
    }

    #[test]
    fn save_is_refused_while_the_line_ending_is_in_conflict() {
        let dir = temp_dir("eol-save-refused");
        let mut session = merge_of_texts(&dir, "a\r\n", "a\n", "a\n");
        assert!(matches!(session.save(), Err(CoreError::EolConflict)));
        assert!(!dir.join("out.txt").exists());
    }

    #[test]
    fn chosen_output_eol_settles_the_conflict_and_is_saved() {
        let dir = temp_dir("eol-chosen");
        let mut session = merge_of_texts(&dir, "a\r\nb\r\n", "a\nb\n", "a\nb\n");
        let revision = session.revision();
        assert_eq!(session.set_output_eol(EolStyle::Cr).unwrap(), Change::OutputEolChanged(EolChoice::Style(EolStyle::Cr)));
        assert_ne!(session.revision(), revision);
        assert_eq!(output_eol(&session), EolChoice::Style(EolStyle::Cr));
        session.save().unwrap();
        assert_eq!(fs::read(dir.join("out.txt")).unwrap(), b"a\rb\r");
    }

    #[test]
    fn chosen_output_eol_overrides_what_the_sources_share() {
        let dir = temp_dir("eol-override");
        let mut session = merge_of_texts(&dir, "a\r\n", "a\r\n", "a\r\n");
        session.set_output_eol(EolStyle::Lf).unwrap();
        session.save().unwrap();
        assert_eq!(fs::read(dir.join("out.txt")).unwrap(), b"a\n");
    }

    #[test]
    fn loading_a_source_again_recomputes_the_output_eol() {
        let dir = temp_dir("eol-reload");
        let mut session = merge_of_texts(&dir, "a\r\n", "a\r\n", "a\r\n");
        session.set_output_eol(EolStyle::Cr).unwrap();
        let other = write(&dir, "other.txt", "a\n");
        session.load_source(Document::B, &other, &BuiltinEngine).unwrap();
        assert_eq!(output_eol(&session), EolChoice::Conflict);
    }

    #[test]
    fn setting_the_output_eol_needs_a_merge() {
        let dir = temp_dir("eol-two-way");
        let (a, b) = (write(&dir, "a.txt", "a\n"), write(&dir, "b.txt", "b\n"));
        let mut session = Session::open(&files(a, b, None, None), &BuiltinEngine).unwrap();
        assert!(matches!(session.set_output_eol(EolStyle::Lf), Err(CoreError::NotAMerge)));
    }

    #[test]
    fn save_keeps_a_missing_final_line_break_of_a() {
        let dir = temp_dir("final-eol-missing");
        let mut session = merge_of_texts(&dir, "11111\r\n\r\n22222", "11111\r\n\r\n22222", "11111\r\n\r\n22222");
        session.save().unwrap();
        assert_eq!(fs::read(dir.join("out.txt")).unwrap(), b"11111\r\n\r\n22222");
    }

    #[test]
    fn save_ends_with_a_line_break_when_a_does() {
        let dir = temp_dir("final-eol-present");
        let mut session = merge_of_texts(&dir, "one\ntwo\n", "one\ntwo", "one\ntwo");
        session.save().unwrap();
        assert_eq!(fs::read(dir.join("out.txt")).unwrap(), b"one\ntwo\n");
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
        let session = merge_of(
            &dir,
            &["same", "base", "x", "keep", "a"],
            &["same", "left", "x", "keep", "b"],
            &["same", "base", "x", "keep", "c"],
        );
        assert_eq!(
            statuses(&session.snapshot().comparison),
            [
                RegionStatus::Merge(MergeStatus::Unchanged),
                RegionStatus::Merge(MergeStatus::ChangedInB),
                RegionStatus::Merge(MergeStatus::Unchanged),
                RegionStatus::Merge(MergeStatus::Conflict)
            ]
        );
        assert_eq!(
            choices(&session),
            [
                Choice::Hunks(vec![Document::A]),
                Choice::Hunks(vec![Document::B]),
                Choice::Hunks(vec![Document::A]),
                Choice::Unresolved
            ]
        );
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
        assert_eq!(
            update.merge[0],
            RegionMerge {
                choice: Choice::Hunks(vec![Document::C, Document::B]),
                edited: false,
                output_lines: 2
            }
        );
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
    fn navigation_finds_differences_and_conflicts() {
        let dir = temp_dir("navigate");
        let mut session = merge_of(
            &dir,
            &["same", "base", "x", "keep", "a"],
            &["same", "left", "x", "keep", "b"],
            &["same", "base", "x", "keep", "c"],
        );
        let base = session.revision();
        assert_eq!(session.current_region(), Some(3));
        assert_eq!(session.navigate(&base, Some(0), Navigation::NextDifference, false).unwrap(), Some(1));
        assert_eq!(session.navigate(&base, None, Navigation::NextDifference, false).unwrap(), Some(3));
        assert_eq!(session.navigate(&base, None, Navigation::NextDifference, false).unwrap(), None);
        assert_eq!(session.current_region(), Some(3));
        assert_eq!(session.navigate(&base, None, Navigation::PreviousDifference, false).unwrap(), Some(1));
        assert_eq!(session.navigate(&base, Some(0), Navigation::NextConflict, false).unwrap(), Some(3));
        assert_eq!(session.navigate(&base, Some(3), Navigation::PreviousConflict, false).unwrap(), None);
        assert_eq!(session.navigate(&base, Some(2), Navigation::PreviousConflict, false).unwrap(), None);
        assert_eq!(session.revision(), base);
    }

    #[test]
    fn navigation_works_without_output() {
        let dir = temp_dir("navigate-no-output");
        let a = write(&dir, "a.txt", "x
y
");
        let b = write(&dir, "b.txt", "x
z
");
        let mut session = Session::open(&files(a, b, None, None), &BuiltinEngine).unwrap();
        let base = session.revision();
        assert_eq!(session.current_region(), Some(1));
        assert_eq!(session.navigate(&base, Some(0), Navigation::NextDifference, false).unwrap(), Some(1));
        assert_eq!(session.navigate(&base, None, Navigation::PreviousDifference, false).unwrap(), None);
    }

    #[test]
    fn navigation_finds_hand_edited_stable_lines() {
        let dir = temp_dir("navigate-split");
        let mut session = merge_session(
            &dir,
            "aa
bb
cc
",
        );
        session.edit_output(&edit(&session, 4..4, "x"), &BuiltinEngine).unwrap();
        let base = session.revision();
        assert_eq!(session.navigate(&base, Some(0), Navigation::NextDifference, false).unwrap(), Some(1));
        assert_eq!(session.navigate(&base, Some(2), Navigation::PreviousDifference, false).unwrap(), Some(1));
        assert_eq!(session.navigate(&base, Some(1), Navigation::NextDifference, false).unwrap(), None);
        assert_eq!(session.navigate(&base, Some(0), Navigation::NextConflict, false).unwrap(), None);
    }

    #[test]
    fn navigation_without_a_current_region_starts_at_the_ends() {
        let dir = temp_dir("navigate-ends");
        let mut session = merge_of(
            &dir,
            &["same", "base", "x", "keep", "a"],
            &["same", "left", "x", "keep", "b"],
            &["same", "base", "x", "keep", "c"],
        );
        let base = session.revision();
        session.set_current_region(&base, None).unwrap();
        assert_eq!(session.navigate(&base, None, Navigation::PreviousDifference, false).unwrap(), Some(3));
        session.set_current_region(&base, None).unwrap();
        assert_eq!(session.navigate(&base, None, Navigation::PreviousConflict, false).unwrap(), Some(3));
    }

    #[test]
    fn navigation_counts_resolved_conflicts() {
        let dir = temp_dir("navigate-resolved");
        let mut session = merge_of(&dir, &["s", "a"], &["s", "b"], &["s", "c"]);
        session.set_choice(&session.revision(), 1, Choice::Hunks(vec![Document::B])).unwrap();
        assert_eq!(session.navigate(&session.revision(), Some(0), Navigation::NextConflict, false).unwrap(), Some(1));
    }

    #[test]
    fn navigation_rejects_bad_requests() {
        let dir = temp_dir("navigate-bad");
        let mut session = merge_of(&dir, &["a"], &["b"], &["c"]);
        let base = session.revision();
        assert!(matches!(
            session.navigate(&Revision(9), None, Navigation::NextConflict, false),
            Err(CoreError::StaleRevision { .. })
        ));
        assert!(matches!(
            session.navigate(&base, Some(4), Navigation::NextConflict, false),
            Err(CoreError::NoSuchRegion { index: 4, count: 1 })
        ));
        assert!(matches!(session.set_current_region(&base, Some(1)), Err(CoreError::NoSuchRegion { index: 1, count: 1 })));
        session.set_current_region(&base, Some(0)).unwrap();
        assert_eq!(session.current_region(), Some(0));
    }

    /// The Foo and Qux example of Manual alignment in ARCHITECTURE.md: B adds Foo and C adds Qux between Bar and Baz.
    fn foo_and_qux(dir: &Path) -> Session {
        merge_of(dir, &["Bar", "Baz"], &["Bar", "Foo", "Baz"], &["Bar", "Qux", "Baz"])
    }

    /// A gets two anchors before Baz; B gets Foo and an anchor after it; C gets an anchor before Qux, and Qux.
    fn mark_foo_and_qux(session: &mut Session) {
        for (document, lines) in [
            (Document::A, 1..1),
            (Document::A, 1..1),
            (Document::B, 1..2),
            (Document::B, 2..2),
            (Document::C, 1..1),
            (Document::C, 1..2),
        ] {
            session.mark_alignment(document, lines).unwrap();
        }
    }

    fn alignment_of(session: &Session) -> Alignments {
        session.snapshot().alignments
    }

    #[test]
    fn marks_wait_for_apply_and_apply_separates_unrelated_insertions() {
        let dir = temp_dir("alignment-apply");
        let mut session = foo_and_qux(&dir);
        let unaligned = [
            RegionStatus::Merge(MergeStatus::Unchanged),
            RegionStatus::Merge(MergeStatus::Conflict),
            RegionStatus::Merge(MergeStatus::Unchanged),
        ];
        assert_eq!(statuses(&session.snapshot().comparison), unaligned);
        let revision = session.revision();
        mark_foo_and_qux(&mut session);
        assert_eq!(session.revision().0, revision.0 + 6);
        assert_eq!(statuses(&session.snapshot().comparison), unaligned);
        let summary = alignment_of(&session);
        assert!(summary.changed);
        assert_eq!(summary.marks.len(), 6);
        assert!(summary.marks.iter().all(|mark| mark.status == MarkStatus::Pending));
        assert_eq!(session.apply_alignments(false, &BuiltinEngine).unwrap(), true);
        let snapshot = session.snapshot();
        let aligned = [
            RegionStatus::Merge(MergeStatus::Unchanged),
            RegionStatus::Merge(MergeStatus::ChangedInB),
            RegionStatus::Merge(MergeStatus::ChangedInC),
            RegionStatus::Merge(MergeStatus::Unchanged),
        ];
        assert_eq!(statuses(&snapshot.comparison), aligned);
        assert_eq!(snapshot.output.unwrap().text, "Bar\nFoo\nQux\nBaz\n");
        assert!(!snapshot.alignments.changed);
        assert!(snapshot.alignments.marks.iter().all(|mark| mark.status == MarkStatus::Applied));
        assert_consistent(&session);
    }

    #[test]
    fn applying_nothing_new_changes_nothing() {
        let dir = temp_dir("alignment-noop");
        let mut session = foo_and_qux(&dir);
        assert_eq!(session.apply_alignments(false, &BuiltinEngine).unwrap(), false);
        mark_foo_and_qux(&mut session);
        session.apply_alignments(false, &BuiltinEngine).unwrap();
        let before = session.snapshot();
        assert_eq!(session.apply_alignments(false, &BuiltinEngine).unwrap(), false);
        assert_eq!(session.snapshot(), before);
    }

    #[test]
    fn marks_of_an_incomplete_number_are_kept_and_ignored() {
        let dir = temp_dir("alignment-incomplete");
        let mut session = foo_and_qux(&dir);
        session.mark_alignment(Document::B, 1..2).unwrap();
        session.mark_alignment(Document::C, 1..2).unwrap();
        assert!(alignment_of(&session).marks.iter().all(|mark| mark.status == MarkStatus::Unmatched));
        assert!(!alignment_of(&session).changed);
        assert_eq!(session.apply_alignments(false, &BuiltinEngine).unwrap(), false);
        assert_eq!(alignment_of(&session).marks.len(), 2);
    }

    #[test]
    fn bad_marks_are_rejected_without_a_change() {
        let dir = temp_dir("alignment-bad-marks");
        let mut session = foo_and_qux(&dir);
        let before = session.snapshot();
        assert!(matches!(session.mark_alignment(Document::D, 0..1), Err(CoreError::NoSuchDocument { .. })));
        assert!(matches!(session.mark_alignment(Document::A, 1..3), Err(CoreError::InvalidMark { .. })));
        assert_eq!(session.snapshot(), before);
        session.mark_alignment(Document::B, 0..2).unwrap();
        let error = session.mark_alignment(Document::B, 1..3).unwrap_err();
        assert_eq!(ProtocolError::from(error).kind, ErrorKind::InvalidRequest);
        assert!(matches!(session.remove_alignment_mark(Document::A, 4), Err(CoreError::InvalidMark { .. })));
        assert!(matches!(session.renumber_alignment_mark(Document::A, 1, 0), Err(CoreError::InvalidMark { .. })));
        assert_eq!(alignment_of(&session).marks.len(), 1);
        assert_eq!(session.snapshot().comparison, before.comparison);
    }

    #[test]
    fn marks_can_be_removed_renumbered_and_cleared() {
        let dir = temp_dir("alignment-edit-marks");
        let mut session = foo_and_qux(&dir);
        session.mark_alignment(Document::A, 0..1).unwrap();
        session.mark_alignment(Document::A, 1..2).unwrap();
        session.renumber_alignment_mark(Document::A, 1, 2).unwrap();
        let numbers = |session: &Session| -> Vec<(u32, Range<usize>)> { alignment_of(session).marks.iter().map(|mark| (mark.number, mark.lines.clone())).collect() };
        assert_eq!(numbers(&session), [(1, 1..2), (2, 0..1)]);
        session.remove_alignment_mark(Document::A, 1).unwrap();
        assert_eq!(numbers(&session), [(2, 0..1)]);
        assert!(matches!(session.clear_alignment_marks(), Some(Change::AlignmentsChanged(_))));
        assert!(numbers(&session).is_empty());
        assert!(session.clear_alignment_marks().is_none());
    }

    /// A and C have a before b; B has them the other way around.
    fn mark_a_and_b_reversed_in_b(session: &mut Session) -> Change {
        let mut last = None;
        for (document, lines) in [
            (Document::A, 0..1),
            (Document::A, 1..2),
            (Document::B, 1..2),
            (Document::B, 0..1),
            (Document::C, 0..1),
            (Document::C, 1..2),
        ] {
            last = Some(session.mark_alignment(document, lines).unwrap());
        }
        last.unwrap()
    }

    fn hunks_of(session: &Session) -> Vec<Vec<Range<usize>>> {
        session.snapshot().comparison.regions.iter().map(|region| region.hunks.clone()).collect()
    }

    #[test]
    fn crossing_alignments_are_insets_shown_in_the_order_of_the_document_that_differs() {
        let dir = temp_dir("alignment-inset");
        let mut session = merge_of(&dir, &["a", "b"], &["b", "a"], &["a", "b"]);
        let Change::AlignmentsChanged(summary) = mark_a_and_b_reversed_in_b(&mut session) else {
            panic!("expected the marks to change")
        };
        assert_eq!(summary.ask, Some(2));
        assert_eq!(summary.insets.len(), 2);
        assert_eq!(alignment_of(&session).ask, None);
        assert_eq!(session.apply_alignments(true, &BuiltinEngine).unwrap(), true);
        assert_eq!(hunks_of(&session), [vec![1..2, 0..1, 1..2], vec![0..1, 1..2, 0..1]]);
        assert_eq!(output_text(&session), "b\na\n");
        assert_consistent(&session);
        let moved: Vec<(Document, u32, Option<u32>)> = alignment_of(&session).marks.iter().filter(|mark| mark.is_moved()).map(|mark| (mark.document.clone(), mark.number, mark.replaced_by)).collect();
        assert_eq!(moved, [(Document::A, 1, Some(2)), (Document::C, 1, Some(2))]);
        assert!(alignment_of(&session).marks.iter().all(|mark| mark.status == MarkStatus::Applied));
    }

    #[test]
    fn placing_an_inset_elsewhere_waits_for_apply_and_moves_the_other_documents() {
        let dir = temp_dir("alignment-place");
        let mut session = merge_of(&dir, &["a", "b"], &["b", "a"], &["a", "b"]);
        mark_a_and_b_reversed_in_b(&mut session);
        session.apply_alignments(true, &BuiltinEngine).unwrap();
        let before = session.snapshot().comparison;
        let revision = session.revision();
        let Change::AlignmentsChanged(summary) = session.place_alignment(2, 1).unwrap() else {
            panic!("expected the marks to change")
        };
        assert_eq!(summary.ask, None);
        assert!(summary.changed);
        assert_eq!(session.revision().0, revision.0 + 1);
        assert_eq!(session.snapshot().comparison, before);
        assert_eq!(session.apply_alignments(true, &BuiltinEngine).unwrap(), true);
        assert_eq!(hunks_of(&session), [vec![0..1, 1..2, 0..1], vec![1..2, 0..1, 1..2]]);
        assert_eq!(output_text(&session), "a\nb\n");
        assert_consistent(&session);
        let moved: Vec<(Document, u32, Option<u32>)> = alignment_of(&session).marks.iter().filter(|mark| mark.is_moved()).map(|mark| (mark.document.clone(), mark.number, mark.replaced_by)).collect();
        assert_eq!(moved, [(Document::B, 2, Some(1))]);
    }

    #[test]
    fn bad_places_are_rejected_without_a_change() {
        let dir = temp_dir("alignment-bad-place");
        let mut session = merge_of(&dir, &["a", "b"], &["b", "a"], &["a", "b"]);
        mark_a_and_b_reversed_in_b(&mut session);
        let before = alignment_of(&session);
        let revision = session.revision();
        assert!(matches!(session.place_alignment(5, 0), Err(CoreError::InvalidMark { .. })));
        assert!(matches!(session.place_alignment(2, 4), Err(CoreError::InvalidMark { .. })));
        assert_eq!(alignment_of(&session), before);
        assert_eq!(session.revision(), revision);
    }

    #[test]
    fn insets_with_anchors_apply() {
        let dir = temp_dir("alignment-inset-anchors");
        let mut session = merge_of(&dir, &["a", "x", "b"], &["b", "x", "a"], &["a", "x", "b"]);
        for (document, lines) in [
            (Document::A, 0..1),
            (Document::A, 2..3),
            (Document::A, 1..1),
            (Document::B, 2..3),
            (Document::B, 0..1),
            (Document::B, 1..2),
            (Document::C, 0..1),
            (Document::C, 2..3),
            (Document::C, 1..1),
        ] {
            session.mark_alignment(document, lines).unwrap();
        }
        assert_eq!(session.apply_alignments(true, &BuiltinEngine).unwrap(), true);
        assert_consistent(&session);
        let mut lines: Vec<usize> = session.snapshot().comparison.regions.iter().flat_map(|region| region.rows.iter().filter_map(|row| row.lines[1])).collect();
        lines.sort();
        assert_eq!(lines, [0, 1, 2]);
    }

    #[test]
    fn numbers_may_run_against_the_positions() {
        let dir = temp_dir("alignment-reversed-numbers");
        let mut session = merge_of(&dir, &["a", "b"], &["a", "b"], &["a", "b"]);
        for document in [Document::A, Document::B, Document::C] {
            session.mark_alignment(document.clone(), 1..2).unwrap();
            session.mark_alignment(document, 0..1).unwrap();
        }
        assert_eq!(session.apply_alignments(true, &BuiltinEngine).unwrap(), true);
        assert!(alignment_of(&session).marks.iter().all(|mark| mark.status == MarkStatus::Applied));
    }

    #[test]
    fn apply_asks_before_discarding_work() {
        let dir = temp_dir("alignment-confirm");
        let mut session = merge_of(&dir, &["s", "a"], &["s", "b"], &["s", "c"]);
        for document in [Document::A, Document::B, Document::C] {
            session.mark_alignment(document, 1..2).unwrap();
        }
        session.set_choice(&session.revision(), 1, Choice::Hunks(vec![Document::B])).unwrap();
        let before = session.snapshot();
        let error = session.apply_alignments(false, &BuiltinEngine).unwrap_err();
        assert_eq!(ProtocolError::from(error).kind, ErrorKind::NeedsConfirmation);
        assert_eq!(session.snapshot(), before);
        assert_eq!(session.apply_alignments(true, &BuiltinEngine).unwrap(), true);
        assert_eq!(choices(&session)[1], Choice::Unresolved);
        assert_eq!(session.undo().unwrap(), None);
    }

    #[test]
    fn hand_edits_count_as_work_but_auto_resolved_regions_do_not() {
        let dir = temp_dir("alignment-work");
        let mut session = merge_of(&dir, &["s", "a"], &["s", "b"], &["s", "c"]);
        for document in [Document::A, Document::B, Document::C] {
            session.mark_alignment(document, 0..1).unwrap();
        }
        session.edit_output(&edit(&session, 0..0, "x"), &BuiltinEngine).unwrap();
        assert!(matches!(session.apply_alignments(false, &BuiltinEngine), Err(CoreError::NeedsConfirmation)));
        let mut untouched = merge_of(&dir, &["s", "a"], &["s", "b"], &["s", "c"]);
        for document in [Document::A, Document::B, Document::C] {
            untouched.mark_alignment(document, 0..1).unwrap();
        }
        assert_eq!(untouched.apply_alignments(false, &BuiltinEngine).unwrap(), true);
    }

    #[test]
    fn a_diff_without_d_applies_without_asking() {
        let dir = temp_dir("alignment-diff");
        let a = write(&dir, "a.txt", "x\ny\n");
        let b = write(&dir, "b.txt", "y\nx\n");
        let mut session = Session::open(&files(a, b, None, None), &BuiltinEngine).unwrap();
        session.mark_alignment(Document::A, 1..2).unwrap();
        session.mark_alignment(Document::B, 0..1).unwrap();
        assert_eq!(session.apply_alignments(false, &BuiltinEngine).unwrap(), true);
        assert_eq!(session.snapshot().comparison.regions.len(), 3);
    }

    #[test]
    fn reloading_a_source_drops_marks_and_applied_alignments() {
        let dir = temp_dir("alignment-reload");
        let mut session = foo_and_qux(&dir);
        mark_foo_and_qux(&mut session);
        session.apply_alignments(false, &BuiltinEngine).unwrap();
        let path = write(&dir, "b2.txt", "Bar\nFoo\nBaz\n");
        session.load_source(Document::B, &path, &BuiltinEngine).unwrap();
        assert_eq!(alignment_of(&session), Alignments::default());
        assert_eq!(statuses(&session.snapshot().comparison).len(), 3);
    }

    #[test]
    fn a_failed_reload_keeps_the_marks() {
        let dir = temp_dir("alignment-reload-fails");
        let mut session = foo_and_qux(&dir);
        mark_foo_and_qux(&mut session);
        assert!(session.load_source(Document::B, &dir.join("missing.txt"), &BuiltinEngine).is_err());
        assert_eq!(alignment_of(&session).marks.len(), 6);
    }

    #[test]
    fn reload_rereads_every_source_and_drops_the_marks() {
        let dir = temp_dir("reload-all");
        let mut session = merge_of(&dir, &["s", "a"], &["s", "b"], &["s", "c"]);
        for document in [Document::A, Document::B, Document::C] {
            session.mark_alignment(document, 0..1).unwrap();
        }
        write(&dir, "b.txt", "s\na\n");
        write(&dir, "c.txt", "s\nc\nd\n");
        session.reload(false, &BuiltinEngine).unwrap();
        let texts: Vec<String> = session
            .snapshot()
            .documents
            .into_iter()
            .map(|document| document.text)
            .collect();
        assert_eq!(texts, ["s\na\n", "s\na\n", "s\nc\nd\n"]);
        assert_eq!(output_text(&session), "s\nc\nd\n");
        assert_eq!(alignment_of(&session), Alignments::default());
    }

    #[test]
    fn reload_asks_before_discarding_work() {
        let dir = temp_dir("reload-confirm");
        let mut session = merge_of(&dir, &["s", "a"], &["s", "b"], &["s", "c"]);
        session.set_choice(&session.revision(), 1, Choice::Hunks(vec![Document::B])).unwrap();
        let before = session.snapshot();
        let error = session.reload(false, &BuiltinEngine).unwrap_err();
        assert_eq!(ProtocolError::from(error).kind, ErrorKind::NeedsConfirmation);
        assert_eq!(session.snapshot(), before);
        session.reload(true, &BuiltinEngine).unwrap();
        assert_eq!(choices(&session)[1], Choice::Unresolved);
        assert_eq!(session.undo().unwrap(), None);
    }

    #[test]
    fn a_failed_reload_leaves_the_session_unchanged() {
        let dir = temp_dir("reload-all-fails");
        let mut session = merge_of(&dir, &["s", "a"], &["s", "b"], &["s", "c"]);
        for document in [Document::A, Document::B, Document::C] {
            session.mark_alignment(document, 0..1).unwrap();
        }
        let before = session.snapshot();
        write(&dir, "a.txt", "changed\n");
        fs::remove_file(dir.join("c.txt")).unwrap();
        assert!(session.reload(true, &BuiltinEngine).is_err());
        assert_eq!(session.snapshot(), before);
    }

    #[test]
    fn comparison_options_keep_marks_and_applied_alignments() {
        let dir = temp_dir("alignment-options");
        let mut session = foo_and_qux(&dir);
        mark_foo_and_qux(&mut session);
        session.apply_alignments(false, &BuiltinEngine).unwrap();
        session
            .set_comparison_options(
                ComparisonOptions {
                    try_hard: true,
                    ..ComparisonOptions::default()
                },
                &BuiltinEngine,
            )
            .unwrap();
        assert_eq!(statuses(&session.snapshot().comparison).len(), 4);
        let summary = alignment_of(&session);
        assert_eq!(summary.marks.len(), 6);
        assert!(!summary.changed);
    }

    #[test]
    fn clearing_the_marks_keeps_the_layout_until_apply() {
        let dir = temp_dir("alignment-clear");
        let mut session = foo_and_qux(&dir);
        mark_foo_and_qux(&mut session);
        session.apply_alignments(false, &BuiltinEngine).unwrap();
        session.clear_alignment_marks().unwrap();
        assert_eq!(statuses(&session.snapshot().comparison).len(), 4);
        assert!(alignment_of(&session).changed);
        assert_eq!(session.apply_alignments(false, &BuiltinEngine).unwrap(), true);
        assert_eq!(statuses(&session.snapshot().comparison).len(), 3);
        assert!(!alignment_of(&session).changed);
    }

    #[test]
    fn navigation_stops_at_alignments_only_when_asked() {
        let dir = temp_dir("alignment-navigate");
        let mut session = merge_of(&dir, &["x", "same", "y"], &["x", "same", "y"], &["x", "same", "y"]);
        for document in [Document::A, Document::B, Document::C] {
            session.mark_alignment(document, 1..2).unwrap();
        }
        session.apply_alignments(false, &BuiltinEngine).unwrap();
        assert_eq!(statuses(&session.snapshot().comparison).len(), 3);
        let base = session.revision();
        assert_eq!(session.navigate(&base, None, Navigation::NextDifference, false).unwrap(), None);
        assert_eq!(session.navigate(&base, None, Navigation::NextDifference, true).unwrap(), Some(1));
        assert_eq!(session.navigate(&base, None, Navigation::NextDifference, true).unwrap(), None);
        assert_eq!(session.navigate(&base, Some(2), Navigation::PreviousDifference, true).unwrap(), Some(1));
        assert_eq!(session.navigate(&base, Some(2), Navigation::PreviousConflict, true).unwrap(), None);
    }

    #[test]
    fn navigation_stops_at_the_first_region_of_an_alignment() {
        let dir = temp_dir("alignment-navigate-first");
        let mut session = merge_of(&dir, &["x", "s1", "s2", "s3", "y"], &["x", "s1", "b2", "s3", "y"], &["x", "s1", "s2", "s3", "y"]);
        for document in [Document::A, Document::B, Document::C] {
            session.mark_alignment(document, 1..4).unwrap();
        }
        session.apply_alignments(false, &BuiltinEngine).unwrap();
        assert_eq!(statuses(&session.snapshot().comparison).len(), 5);
        let base = session.revision();
        assert_eq!(session.navigate(&base, Some(0), Navigation::NextDifference, true).unwrap(), Some(1));
        assert_eq!(session.navigate(&base, Some(1), Navigation::NextDifference, true).unwrap(), Some(2));
        assert_eq!(session.navigate(&base, Some(2), Navigation::NextDifference, true).unwrap(), None);
    }

    #[test]
    fn changes_of_d_and_their_undo_make_their_region_current() {
        let dir = temp_dir("current-follows-changes");
        let mut session = merge_of(&dir, &["a1", "s", "a2"], &["b1", "s", "b2"], &["c1", "s", "c2"]);
        session.set_choice(&session.revision(), 2, Choice::Hunks(vec![Document::B])).unwrap();
        assert_eq!(session.current_region(), Some(2));
        session.set_choice(&session.revision(), 0, Choice::Hunks(vec![Document::C])).unwrap();
        assert_eq!(session.current_region(), Some(0));
        session.undo().unwrap();
        assert_eq!(session.current_region(), Some(0));
        session.undo().unwrap();
        assert_eq!(session.current_region(), Some(2));
        assert_eq!(session.snapshot().current_region, Some(2));
    }

    #[test]
    fn comparison_options_recompute_and_rebuild_d() {
        let dir = temp_dir("options");
        let mut session = merge_of(&dir, &["s", "a"], &["s", "b"], &["s", "c"]);
        session.set_choice(&session.revision(), 1, Choice::Hunks(vec![Document::B])).unwrap();
        let revision = session.revision();
        let options = ComparisonOptions {
            try_hard: true,
            ..ComparisonOptions::default()
        };
        session.set_comparison_options(options.clone(), &BuiltinEngine).unwrap();
        let snapshot = session.snapshot();
        assert_eq!(snapshot.options, options);
        assert_eq!(snapshot.current_region, None);
        assert!(snapshot.revision.0 > revision.0);
        assert_eq!(choices(&session)[1], Choice::Unresolved);
        assert_eq!(session.undo().unwrap(), None);
        assert_consistent(&session);
    }

    #[test]
    fn ignore_whitespace_recomputes_only_char_diffs_and_keeps_d() {
        let dir = temp_dir("ignore-whitespace");
        let mut session = merge_of(&dir, &["s", "call(a, b);"], &["s", "call(", "  a,", "  b);"], &["s", "call(a, b);", "x"]);
        session.set_choice(&session.revision(), 1, Choice::Hunks(vec![Document::C])).unwrap();
        let before = session.snapshot();
        assert!(before.char_diffs[0].line(1).iter().any(|run| !run.same_in[1]));
        let options = ComparisonOptions {
            ignore_whitespace: true,
            ..ComparisonOptions::default()
        };
        session.set_comparison_options(options.clone(), &BuiltinEngine).unwrap();
        let snapshot = session.snapshot();
        assert_eq!(snapshot.options, options);
        assert!(snapshot.revision.0 > before.revision.0);
        assert_eq!(snapshot.comparison, before.comparison);
        assert_eq!(snapshot.output, before.output);
        assert_eq!(snapshot.current_region, before.current_region);
        assert_eq!(choices(&session)[1], Choice::Hunks(vec![Document::C]));
        for line in 1..4 {
            assert!(snapshot.char_diffs[1].line(line).iter().all(|run| run.same_in[0]), "B line {line} is the same as A");
        }
        assert!(snapshot.char_diffs[0].line(1).iter().all(|run| run.same_in[1]));
        assert!(snapshot.char_diffs[2].line(2).iter().all(|run| !run.same_in[0] && !run.same_in[1]));
        assert!(session.undo().unwrap().is_some());
        session.set_comparison_options(ComparisonOptions::default(), &BuiltinEngine).unwrap();
        assert_eq!(session.snapshot().char_diffs, before.char_diffs);
        assert_consistent(&session);
    }


    #[test]
    fn ignore_whitespace_diffs_a_region_over_the_limit_row_by_row() {
        let dir = temp_dir("ignore-whitespace-limit");
        let mut session = merge_of(&dir, &["s", "ab", "cd"], &["s", "a b", "x d"], &["s", "ab", "cd"]);
        let options = ComparisonOptions {
            ignore_whitespace: true,
            char_diff_line_limit: 3,
            ..ComparisonOptions::default()
        };
        session.set_comparison_options(options, &BuiltinEngine).unwrap();
        let snapshot = session.snapshot();
        assert!(snapshot.char_diffs[1].line(1).iter().all(|run| run.same_in[0]));
        let marked: Vec<usize> = snapshot.char_diffs[1].line(2).iter().filter(|run| !run.same_in[0]).flat_map(|run| run.columns.clone()).collect();
        assert_eq!(marked, [0]);
    }


    #[test]
    fn failed_comparison_options_keep_the_session() {
        let dir = temp_dir("options-fail");
        let mut session = merge_of(&dir, &["s", "a"], &["s", "b"], &["s", "c"]);
        let before = session.snapshot();
        assert!(
            session
                .set_comparison_options(
                    ComparisonOptions {
                        try_hard: true,
                        ..ComparisonOptions::default()
                    },
                    &FailingEngine
                )
                .is_err()
        );
        assert_eq!(session.snapshot(), before);
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
        assert!(matches!(
            session.set_choice(&base, 5, Choice::Unresolved),
            Err(CoreError::NoSuchRegion { index: 5, count: 1 })
        ));
        assert!(matches!(
            session.set_choice(&base, 0, Choice::Hunks(vec![Document::D])),
            Err(CoreError::NoSuchDocument { .. })
        ));
        assert!(matches!(
            session.set_choice(&base, 0, Choice::Hunks(vec![Document::A, Document::A])),
            Err(CoreError::InvalidChoice { .. })
        ));
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
    fn choose_everywhere_replaces_choices_and_edits_with_one_undo() {
        let dir = temp_dir("choose-everywhere");
        let mut session = merge_of_texts(&dir, "a\ns\nx\nt\np", "b\ns\nX\nt\nq", "c\ns\nx\nt\nr");
        session.set_choice(&session.revision(), 0, Choice::Hunks(vec![Document::B, Document::C])).unwrap();
        session.edit_output(&edit(&session, 4..5, "manual"), &BuiltinEngine).unwrap();
        let before = session.snapshot();
        let base = session.revision();
        assert!(matches!(session.choose_everywhere(&base, Document::A, false, &BuiltinEngine), Err(CoreError::NeedsConfirmation)));
        assert_eq!(session.snapshot(), before);
        session.choose_everywhere(&base, Document::A, true, &BuiltinEngine).unwrap().unwrap();
        assert_eq!(output_text(&session), "a\ns\nx\nt\np");
        assert_eq!(session.unresolved_count(), 0);
        assert!(session.snapshot().merge.iter().all(|state| !state.edited && state.choice == Choice::Hunks(vec![Document::A])));
        assert_consistent(&session);
        assert_eq!(session.choose_everywhere(&session.revision(), Document::A, false, &BuiltinEngine).unwrap(), None);
        session.undo().unwrap().unwrap();
        assert_eq!(session.snapshot().merge, before.merge);
        assert_eq!(session.snapshot().output, before.output);
        assert_eq!(session.snapshot().comparison, before.comparison);
        session.redo().unwrap().unwrap();
        assert_eq!(output_text(&session), "a\ns\nx\nt\np");
        assert_consistent(&session);
    }


    #[test]
    fn choose_everywhere_uses_source_order_across_insets() {
        let dir = temp_dir("choose-everywhere-insets");
        let mut session = merge_of(&dir, &["a", "b"], &["b", "a"], &["a", "b"]);
        mark_a_and_b_reversed_in_b(&mut session);
        session.apply_alignments(true, &BuiltinEngine).unwrap();
        let before = session.snapshot();
        session.choose_everywhere(&session.revision(), Document::A, false, &BuiltinEngine).unwrap().unwrap();
        assert_eq!(output_text(&session), "a\nb\n");
        assert_consistent(&session);
        session.undo().unwrap().unwrap();
        assert_eq!(session.snapshot().comparison, before.comparison);
        assert_eq!(output_text(&session), "b\na\n");
    }


    #[test]
    fn choosing_a_trailing_hunk_after_choose_everywhere_starts_a_new_line() {
        let dir = temp_dir("choose-everywhere-trailing-hunk");
        let mut session = merge_of_texts(&dir, "first", "first\nsecond\n", "first");
        session.choose_everywhere(&session.revision(), Document::A, false, &BuiltinEngine).unwrap().unwrap();
        assert_eq!(output_text(&session), "first");
        session.set_choice(&session.revision(), 1, Choice::Hunks(vec![Document::B])).unwrap();
        assert_eq!(output_text(&session), "first\nsecond\n");
        assert_consistent(&session);
        session.undo().unwrap().unwrap();
        assert_eq!(output_text(&session), "first");
        assert_consistent(&session);
    }


    #[test]
    fn choosing_a_hunk_of_a_split_stable_line_joins_it_back() {
        for document in [Document::A, Document::B, Document::C] {
            let dir = temp_dir("rejoin-stable");
            let mut session = merge_session(
                &dir,
                "aa
bb
cc
",
            );
            let original = session.merge.entries(0..1);
            session.edit_output(&edit(&session, 4..4, "x"), &BuiltinEngine).unwrap();
            assert_eq!(session.merge.regions().len(), 3);
            assert_eq!(
                output_text(&session),
                "aa
bxb
cc
"
            );
            let change = session.set_choice(&session.revision(), 1, Choice::Hunks(vec![document])).unwrap();
            let Change::OutputEdited { merge: update, .. } = &change else {
                panic!("expected an output change");
            };
            assert_eq!((update.at, update.removed, update.merge.len()), (0, 3, 1));
            assert_eq!(
                output_text(&session),
                "aa
bb
cc
"
            );
            assert_eq!(session.merge.entries(0..1), original);
            assert_eq!(session.current_region(), Some(0));
            assert_consistent(&session);
            session.undo().unwrap();
            assert_eq!(session.merge.regions().len(), 3);
            assert_eq!(
                output_text(&session),
                "aa
bxb
cc
"
            );
            assert_consistent(&session);
        }
    }

    #[test]
    fn split_lines_join_back_one_at_a_time() {
        let dir = temp_dir("rejoin-stable-lines");
        let mut session = merge_session(
            &dir,
            "aa
bb
cc
",
        );
        session
            .edit_output(
                &edit(
                    &session,
                    0..8,
                    "1
2
3",
                ),
                &BuiltinEngine,
            )
            .unwrap();
        assert_eq!(session.merge.regions().len(), 3);
        session.set_choice(&session.revision(), 1, Choice::Hunks(vec![Document::B])).unwrap();
        assert_eq!(session.merge.regions().len(), 3);
        assert_eq!(
            output_text(&session),
            "1
bb
3
"
        );
        session.set_choice(&session.revision(), 0, Choice::Hunks(vec![Document::C])).unwrap();
        assert_eq!(session.merge.regions().len(), 2);
        session.set_choice(&session.revision(), 1, Choice::Hunks(vec![Document::A])).unwrap();
        assert_eq!(session.merge.regions().len(), 1);
        assert_eq!(
            output_text(&session),
            "aa
bb
cc
"
        );
        assert_consistent(&session);
    }

    #[test]
    fn choosing_nothing_for_a_split_line_keeps_it_apart() {
        let dir = temp_dir("rejoin-stable-nothing");
        let mut session = merge_session(
            &dir,
            "aa
bb
cc
",
        );
        session.edit_output(&edit(&session, 4..4, "x"), &BuiltinEngine).unwrap();
        session.set_choice(&session.revision(), 1, Choice::Hunks(Vec::new())).unwrap();
        assert_eq!(session.merge.regions().len(), 3);
        assert_eq!(
            output_text(&session),
            "aa
cc
"
        );
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
        assert_eq!(
            update.merge[0],
            RegionMerge {
                choice: Choice::Hunks(vec![Document::B]),
                edited: true,
                output_lines: 1
            }
        );
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
    fn typing_on_an_empty_region_goes_to_it() {
        let dir = temp_dir("edit-empty-region");
        let mut session = merge_of(&dir, &["s", "x", "t"], &["s", "t"], &["s", "x", "t"]);
        assert_eq!(output_text(&session), "s\nt\n");
        assert_eq!(
            session.snapshot().merge[1],
            RegionMerge {
                choice: Choice::Hunks(vec![Document::B]),
                edited: false,
                output_lines: 0
            }
        );
        let change = session
            .edit_output(
                &OutputEdit {
                    region: Some(1),
                    ..edit(&session, 2..2, "new\n")
                },
                &BuiltinEngine,
            )
            .unwrap();
        let Change::OutputEdited { merge: update, .. } = &change else {
            panic!("expected an output change");
        };
        assert_eq!((update.at, update.removed), (1, 1));
        assert_eq!(
            update.merge[0],
            RegionMerge {
                choice: Choice::Hunks(vec![Document::B]),
                edited: true,
                output_lines: 1
            }
        );
        assert_eq!(output_text(&session), "s\nnew\nt\n");
        assert_eq!(session.snapshot().comparison.regions.len(), 3);
        assert_consistent(&session);
    }

    #[test]
    fn edits_that_do_not_fit_their_target_region_are_rejected() {
        let dir = temp_dir("edit-bad-target");
        let mut session = merge_of(&dir, &["s", "x", "t"], &["s", "t"], &["s", "x", "t"]);
        let cases = [
            (Some(1), 2..2, "new"),
            (Some(1), 0..0, "new\n"),
            (Some(1), 2..3, "new\n"),
            (Some(0), 0..0, "new\n"),
            (Some(9), 2..2, "new\n"),
        ];
        for (region, range, text) in cases {
            let result = session.edit_output(
                &OutputEdit {
                    region,
                    ..edit(&session, range.clone(), text)
                },
                &BuiltinEngine,
            );
            assert!(matches!(result, Err(CoreError::InvalidEditTarget { .. })), "{region:?} {range:?} {text:?}");
        }
        assert_eq!(output_text(&session), "s\nt\n");
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
        assert_eq!(
            snapshot.merge[0],
            RegionMerge {
                choice: Choice::Unresolved,
                edited: true,
                output_lines: 1
            }
        );
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
        Edit {
            at: usize,
            len: usize,
            text: &'static str,
        },
        Choose {
            region: usize,
            documents: Vec<Document>,
        },
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

    const TWO_FUNCTIONS: [&str; 18] = [
        "int f(int n)",
        "{",
        "    if (n)",
        "    {",
        "        a();",
        "    }",
        "    return n;",
        "}",
        "",
        "int g(int n)",
        "{",
        "    while (n)",
        "    {",
        "        b();",
        "    }",
        "    return n;",
        "}",
        "",
    ];

    fn heuristic_lines(session: &Session) -> Vec<std::ops::Range<usize>> {
        let alignments = session.snapshot().alignments;
        alignments.of(Document::A).filter(|mark| mark.heuristic).map(|mark| mark.lines.clone()).collect()
    }

    #[test]
    fn a_blocked_heuristic_match_takes_the_matches_inside_it_along() {
        let dir = temp_dir("heuristic-blocked");
        let mut lines = vec!["int x;"];
        lines.extend(TWO_FUNCTIONS);
        let mut session = merge_of(&dir, &lines, &lines, &lines);
        let options = HeuristicOptions { blocks: true, ..HeuristicOptions::default() };
        let proposal = session.propose_heuristic_alignments("C++", &options, None).unwrap();
        assert_eq!(proposal.matches.len(), 4);
        assert_eq!(proposal.matches.iter().map(|found| found.parent).collect::<Vec<_>>(), [None, Some(0), None, Some(2)]);
        session.mark_alignment(Document::A, 0..3).unwrap();
        let proposal = session.propose_heuristic_alignments("C++", &options, None).unwrap();
        assert_eq!(proposal.blocked, 2);
        assert_eq!(proposal.matches.len(), 2);
        assert_eq!(proposal.matches.iter().map(|found| found.parent).collect::<Vec<_>>(), [None, Some(0)]);
        assert_eq!(proposal.matches[0].names[0].as_deref(), Some("g"));
    }

    #[test]
    fn a_scoped_proposal_needs_an_applied_alignment_and_pairs_what_is_inside() {
        let dir = temp_dir("heuristic-within");
        let mut session = merge_of(&dir, &TWO_FUNCTIONS, &TWO_FUNCTIONS, &TWO_FUNCTIONS);
        let options = HeuristicOptions::default();
        assert!(matches!(session.propose_heuristic_alignments("C++", &options, Some(1)), Err(CoreError::NotApplied { number: 1 })));
        for document in [Document::A, Document::B, Document::C] {
            session.mark_alignment(document, 0..8).unwrap();
        }
        assert!(matches!(session.propose_heuristic_alignments("C++", &options, Some(1)), Err(CoreError::NotApplied { number: 1 })));
        session.apply_alignments(false, &BuiltinEngine).unwrap();
        let proposal = session.propose_heuristic_alignments("C++", &options, Some(1)).unwrap();
        assert_eq!(proposal.matches.iter().map(|found| (found.kind.clone(), found.parts.clone(), found.parent)).collect::<Vec<_>>(), [(EntityKind::Block, vec![2..6, 2..6, 2..6], None)]);
        assert_eq!(proposal.found, vec![1, 1, 1]);
    }

    #[test]
    fn applying_a_scoped_proposal_keeps_the_heuristic_marks_outside_the_scope() {
        let dir = temp_dir("heuristic-within-apply");
        let mut session = merge_of(&dir, &TWO_FUNCTIONS, &TWO_FUNCTIONS, &TWO_FUNCTIONS);
        let options = HeuristicOptions::default();
        let whole: Vec<Vec<std::ops::Range<usize>>> = session.propose_heuristic_alignments("C++", &options, None).unwrap().matches.into_iter().map(|found| found.parts).collect();
        session.apply_heuristic_alignments(&whole, None, false, &BuiltinEngine).unwrap();
        assert_eq!(heuristic_lines(&session), [0..8, 9..17]);
        let inside: Vec<Vec<std::ops::Range<usize>>> = session.propose_heuristic_alignments("C++", &options, Some(1)).unwrap().matches.into_iter().map(|found| found.parts).collect();
        assert_eq!(inside, [vec![2..6, 2..6, 2..6]]);
        session.apply_heuristic_alignments(&inside, Some(1), false, &BuiltinEngine).unwrap();
        assert_eq!(heuristic_lines(&session), [0..8, 9..17, 2..6]);
        let other: Vec<Vec<std::ops::Range<usize>>> = session.propose_heuristic_alignments("C++", &options, Some(2)).unwrap().matches.into_iter().map(|found| found.parts).collect();
        assert_eq!(other, [vec![11..15, 11..15, 11..15]]);
        session.apply_heuristic_alignments(&other, Some(2), false, &BuiltinEngine).unwrap();
        assert_eq!(heuristic_lines(&session), [0..8, 9..17, 2..6, 11..15]);
        session.apply_heuristic_alignments(&whole, None, true, &BuiltinEngine).unwrap();
        assert_eq!(heuristic_lines(&session), [0..8, 9..17]);
        assert!(matches!(session.apply_heuristic_alignments(&inside, Some(9), true, &BuiltinEngine), Err(CoreError::NotApplied { number: 9 })));
        assert_eq!(heuristic_lines(&session), [0..8, 9..17]);
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
