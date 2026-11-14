use std::collections::VecDeque;
use std::ops::Range;
use std::path::PathBuf;

use kdiff4_protocol::engine::{Comparison, ComparisonOptions, HunkChange, MergeStatus, RegionStatus};
use kdiff4_protocol::{
    Alignments, Change, Choice, CurrentRegion, Delta, Document, DocumentText, EolChoice, FoldRanges, LineCharRuns, MergeUpdate, OutputEdit, RegionMerge, RequestId, Revision, Snapshot,
};

use crate::editing::{self, Edit, Lines};
use crate::folding::Folds;
use crate::rows::{self, RowMap, Rows};

/// The client's copy of D.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputMirror {
    pub path: Option<PathBuf>,
    pub text: String,
    pub lines: Vec<Range<usize>>,
    pub modified: bool,
    pub eol: EolChoice,
}

impl OutputMirror {
    pub fn lines(&self) -> Lines<'_> {
        Lines {
            text: &self.text,
            ranges: &self.lines,
        }
    }

    fn fits(&self, range: &Range<usize>) -> bool {
        range.start <= range.end && range.end <= self.text.len() && self.text.is_char_boundary(range.start) && self.text.is_char_boundary(range.end)
    }


    fn replace(&mut self, range: Range<usize>, text: &str) -> bool {
        let fits = self.fits(&range);
        if fits {
            self.text.replace_range(range.clone(), text);
            editing::update_editor_lines(&mut self.lines, &self.text, range, text.len());
        }
        fits
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaOutcome {
    Applied,
    /// The mirror can no longer follow the host; send a resync request.
    NeedsResync,
    /// Dropped while a resync is pending.
    Ignored,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConflictCounts {
    pub unresolved: usize,
    pub total: usize,
}

/// The client's copy of the session, built from a snapshot and the deltas after it. D edits are applied here at once and confirmed by the host later.
#[derive(Debug, Clone)]
pub struct Mirror {
    revision: Revision,
    documents: Vec<DocumentText>,
    output: Option<OutputMirror>,
    comparison: Comparison,
    /// Per document, in `documents` order: its rows in the synchronized view, without folds.
    row_maps: Vec<RowMap>,
    /// `row_maps` with each document's folds folded: the rows its pane shows.
    folded_row_maps: Vec<RowMap>,
    /// Per region, its merge state; empty without D.
    merge: Vec<RegionMerge>,
    conflicts: ConflictCounts,
    /// D's rows: its lines and the placeholders of unresolved conflicts, without folds.
    output_rows: RowMap,
    /// `output_rows` with D's folds folded.
    folded_output_rows: RowMap,
    /// Per document, in `Document::ALL` order: its fold ranges and folds.
    folds: [Folds; 4],
    /// Every pane gives each region the same number of rows, padding with voids, so D lines up with the source panes.
    aligned: bool,
    options: ComparisonOptions,
    current_region: Option<usize>,
    /// Requests of edits applied locally that the host has not confirmed yet, oldest first.
    pending_edits: VecDeque<RequestId>,
    awaiting_resync: bool,
    /// Per document, in `documents` order: the character-level diff of its lines.
    char_diffs: Vec<LineCharRuns>,
    /// The engine computes character-level diffs; without them `char_diffs` marks nothing.
    has_char_diffs: bool,
    alignments: Alignments,
}

impl Mirror {
    /// `aligned` lines D up with the source panes; see `set_aligned`.
    pub fn from_snapshot(snapshot: Snapshot, aligned: bool) -> Mirror {
        let output = snapshot.output.map(|output| {
            let lines = editing::editor_lines(&output.text);
            OutputMirror {
                path: output.path,
                text: output.text,
                lines,
                modified: output.modified,
                eol: output.eol,
            }
        });
        let mut mirror = Mirror {
            revision: snapshot.revision,
            documents: snapshot.documents,
            output,
            comparison: snapshot.comparison,
            row_maps: Vec::new(),
            folded_row_maps: Vec::new(),
            merge: snapshot.merge,
            conflicts: ConflictCounts::default(),
            output_rows: RowMap::default(),
            folded_output_rows: RowMap::default(),
            folds: Default::default(),
            aligned,
            options: snapshot.options,
            current_region: snapshot.current_region,
            pending_edits: VecDeque::new(),
            awaiting_resync: false,
            char_diffs: snapshot.char_diffs,
            has_char_diffs: snapshot.engine.char_diff,
            alignments: snapshot.alignments,
        };
        mirror.refresh_conflicts();
        mirror.refresh_rows();
        mirror
    }

    /// Lines D up with the source panes, or lets it have just its own rows. Only a merge can line up.
    pub fn set_aligned(&mut self, aligned: bool) {
        if self.aligned != aligned {
            self.aligned = aligned;
            self.refresh_rows();
        }
    }

    /// Whether D shares the source panes' rows, and so their scrolling.
    pub fn is_aligned(&self) -> bool {
        self.aligned && self.is_merge() && self.merge.len() == self.comparison.regions.len()
    }

    pub fn options(&self) -> &ComparisonOptions {
        &self.options
    }

    /// The alignment marks and whether applying them would compare again.
    pub fn alignments(&self) -> &Alignments {
        &self.alignments
    }

    pub fn current_region(&self) -> Option<usize> {
        self.current_region.clone()
    }

    /// The revision navigation commands are based on: like choices, region indices must be known.
    pub fn navigation_base(&self) -> Option<Revision> {
        (self.pending_edits.is_empty() && !self.awaiting_resync).then(|| self.revision.clone())
    }

    /// Applies a current-region event; false when it refers to another revision, so it is dropped.
    pub fn apply_current_region(&mut self, current: &CurrentRegion) -> bool {
        let fits = current.revision == self.revision && current.region.is_none_or(|region| region < self.comparison.regions.len());
        if fits {
            self.current_region = current.region.clone();
        }
        fits
    }

    #[cfg(test)]
    pub fn revision(&self) -> Revision {
        self.revision.clone()
    }

    pub fn documents(&self) -> &[DocumentText] {
        &self.documents
    }

    pub fn document(&self, document: Document) -> Option<&DocumentText> {
        self.documents.iter().find(|text| text.document == document)
    }

    /// The character-level diff of `document`'s lines, if the snapshot has one for it.
    pub fn char_diffs(&self, document: Document) -> Option<&LineCharRuns> {
        self.documents
            .iter()
            .position(|text| text.document == document)
            .and_then(|index| self.char_diffs.get(index))
    }

    /// The rows `document`'s pane shows, with its folds folded.
    pub fn rows(&self, document: Document) -> Option<&RowMap> {
        self.documents.iter().position(|text| text.document == document).and_then(|index| self.folded_row_maps.get(index))
    }


    /// The rows of `document` without folds, which line up with the other documents' rows.
    pub fn unfolded_rows(&self, document: Document) -> Option<&RowMap> {
        match document {
            Document::D => self.output.as_ref().map(|_| &self.output_rows),
            document => self.documents.iter().position(|text| text.document == document).and_then(|index| self.row_maps.get(index)),
        }
    }


    pub fn folds(&self, document: Document) -> &Folds {
        &self.folds[fold_index(document)]
    }


    /// Changes the folds of `document` with `change`, which returns whether it changed anything, and folds its rows again if it did.
    pub fn change_folds(&mut self, document: Document, change: impl FnOnce(&mut Folds) -> bool) -> bool {
        let changed = change(&mut self.folds[fold_index(document.clone())]);
        if changed {
            self.refold(document);
        }
        changed
    }


    /// Takes the fold ranges the engine found. Those of D are dropped when D changed since, or has local edits the host has not confirmed; returns true then, so they can be asked for again.
    pub fn set_fold_ranges(&mut self, found: FoldRanges) -> bool {
        let mut stale = false;
        for (document, ranges) in found.documents {
            if document == Document::D && (found.revision != self.revision || !self.pending_edits.is_empty()) {
                stale = true;
                continue;
            }
            self.folds[fold_index(document.clone())].set_ranges(ranges);
            self.refold(document);
        }
        stale
    }


    /// Keeps the folds of `previous` for every document whose text is the same, as after a comparison that changed only the rows.
    pub fn keep_folds_from(&mut self, previous: &Mirror) {
        for document in Document::ALL {
            let same = match document {
                Document::D => self.output.as_ref().map(|output| &output.text) == previous.output.as_ref().map(|output| &output.text),
                document => self.document(document.clone()).map(|text| &text.text) == previous.document(document.clone()).map(|text| &text.text),
            };
            if same {
                self.folds[fold_index(document.clone())] = previous.folds[fold_index(document.clone())].clone();
                self.refold(document);
            }
        }
    }


    /// Unfolds, in every pane, the folds that hide a line of region `region`, or the place of a void, so a jump there shows it.
    pub fn reveal_region(&mut self, region: usize) {
        for document in Document::ALL {
            let Some(rows) = self.unfolded_rows(document.clone()).map(Rows::Mapped) else {
                continue;
            };
            let Some(range) = rows.region_rows(region) else {
                continue;
            };
            let lines: Vec<usize> = range.clone().filter_map(|row| rows.row(row).and_then(|pane_row| pane_row.line)).collect();
            let reveal = match (lines.iter().min(), lines.iter().max()) {
                (Some(first), Some(last)) => first.clone()..last + 1,
                _ => match rows.line_at_or_after(range.start.clone()) {
                    Some(line) => line..line,
                    None => continue,
                },
            };
            self.change_folds(document, |folds| folds.reveal(reveal));
        }
    }


    fn refold(&mut self, document: Document) {
        match document {
            Document::D => self.folded_output_rows = self.output_rows.folded(self.folds[fold_index(Document::D)].folded()),
            document => {
                if let Some(index) = self.documents.iter().position(|text| text.document == document)
                    && let Some(map) = self.row_maps.get(index)
                {
                    let folded = map.folded(self.folds[fold_index(document)].folded());
                    self.folded_row_maps[index] = folded;
                }
            }
        }
    }

    pub fn output(&self) -> Option<&OutputMirror> {
        self.output.as_ref()
    }

    pub fn is_merge(&self) -> bool {
        self.output.is_some()
    }

    pub fn comparison(&self) -> &Comparison {
        &self.comparison
    }

    /// The rows D's pane shows, with its folds folded.
    pub fn output_rows(&self) -> &RowMap {
        &self.folded_output_rows
    }

    pub fn unresolved_count(&self) -> usize {
        self.conflicts.unresolved
    }

    pub fn conflict_counts(&self) -> Option<ConflictCounts> {
        self.is_merge().then_some(self.conflicts)
    }

    fn refresh_conflicts(&mut self) {
        let mut counts = ConflictCounts::default();
        for (index, state) in self.merge.iter().enumerate() {
            let unresolved = state.is_unresolved();
            counts.unresolved += usize::from(unresolved);
            counts.total += usize::from(unresolved || self.comparison.regions.get(index).is_some_and(|region| region.status.is_conflict()));
        }
        self.conflicts = counts;
    }

    /// The revision a choice command is based on, or `None` while local edits are unconfirmed: their deltas may join regions, so region indices aren't known yet.
    pub fn choice_base(&self) -> Option<Revision> {
        (self.pending_edits.is_empty() && !self.awaiting_resync && !self.merge.is_empty()).then(|| self.revision.clone())
    }

    /// The choice after toggling `document` in region `region`: added at the end, or removed if it is already chosen. An unresolved or hand-edited region starts from no choice, and removing the last chosen hunk makes the region unresolved again.
    pub fn toggled_choice(&self, region: usize, document: Document) -> Option<Choice> {
        let state = self.merge.get(region)?;
        let mut documents = match &state.choice {
            Choice::Hunks(documents) if !state.edited => documents.clone(),
            _ => Vec::new(),
        };
        match documents.iter().position(|chosen| *chosen == document) {
            Some(position) => {
                documents.remove(position);
            }
            None => documents.push(document),
        }
        if documents.is_empty() {
            Some(Choice::Unresolved)
        } else {
            Some(Choice::Hunks(documents))
        }
    }

    /// Applies a merge update; false when it doesn't fit, so the mirror needs a resync.
    fn apply_merge(&mut self, update: MergeUpdate) -> bool {
        let fits = update.at + update.removed <= self.comparison.regions.len() && update.at + update.removed <= self.merge.len() && update.regions.len() == update.merge.len();
        if !fits {
            return false;
        }
        let range = update.at..update.at + update.removed;
        self.comparison.regions.splice(range.clone(), update.regions);
        self.merge.splice(range, update.merge);
        self.refresh_conflicts();
        self.current_region = Some(update.at);
        self.refresh_rows();
        true
    }

    fn refresh_rows(&mut self) {
        let heights = self.is_aligned().then(|| rows::aligned_heights(&self.comparison.regions, &self.merge));
        let whitespace_runs = (self.options.ignore_whitespace && self.has_char_diffs).then_some(self.char_diffs.as_slice());
        self.row_maps = self
            .documents
            .iter()
            .enumerate()
            .map(|(index, text)| RowMap::for_source(&self.comparison, &self.merge, index, text.line_count(), heights.as_deref(), whitespace_runs))
            .collect();
        self.folded_row_maps = self
            .documents
            .iter()
            .zip(self.row_maps.iter())
            .map(|(text, map)| map.folded(self.folds[fold_index(text.document.clone())].folded()))
            .collect();
        self.refresh_output_rows();
    }

    fn refresh_output_rows(&mut self) {
        let line_count = self.output.as_ref().map_or(0, |output| output.lines.len());
        let heights = self.is_aligned().then(|| rows::aligned_heights(&self.comparison.regions, &self.merge));
        self.output_rows = RowMap::for_output(&self.merge, &self.comparison.regions, line_count, heights.as_deref());
        self.folded_output_rows = self.output_rows.folded(self.folds[fold_index(Document::D)].folded());
    }


    /// Moves D's folds with an edit of `range` to `text` that has not been applied yet.
    fn fold_output_edit(&mut self, range: &Range<usize>, text: &str) {
        let Some(output) = self.output.as_ref() else {
            return;
        };
        let lines = output.lines();
        let first = editing::position_of(lines, range.start.clone()).line;
        let old_last = editing::position_of(lines, range.end.clone()).line;
        let new_last = first + editing::editor_lines(text).len().saturating_sub(1);
        self.folds[fold_index(Document::D)].edited(first, old_last, new_last);
    }

    /// For the status bar, e.g. "3 differences, 1 conflict (whitespace only); 1 unresolved".
    pub fn difference_summary(&self) -> String {
        let summary = self.comparison_summary();
        if self.merge.is_empty() || self.merge.len() != self.comparison.regions.len() {
            return summary;
        }
        match self.unresolved_count() {
            0 => format!("{summary}; nothing unresolved"),
            count => format!("{summary}; {count} unresolved"),
        }
    }

    fn comparison_summary(&self) -> String {
        let changed = self
            .comparison
            .regions
            .iter()
            .filter(|region| !matches!(region.status, RegionStatus::Diff(HunkChange::Unchanged) | RegionStatus::Merge(MergeStatus::Unchanged)))
            .count();
        let conflicts = self
            .comparison
            .regions
            .iter()
            .filter(|region| matches!(region.status, RegionStatus::Merge(MergeStatus::Conflict | MergeStatus::WhitespaceConflict)))
            .count();
        let whitespace = self
            .comparison
            .regions
            .iter()
            .filter(|region| region.status == RegionStatus::Merge(MergeStatus::WhitespaceConflict))
            .count();
        let mut summary = match changed {
            0 => return "No differences".to_string(),
            1 => "1 difference".to_string(),
            count => format!("{count} differences"),
        };
        if conflicts > 0 {
            let noun = if conflicts == 1 { "conflict" } else { "conflicts" };
            summary.push_str(&format!(", {conflicts} {noun}"));
        }
        if whitespace == conflicts && whitespace > 0 {
            summary.push_str(" (whitespace only)");
        } else if whitespace > 0 {
            summary.push_str(&format!(" ({whitespace} whitespace only)"));
        }
        summary
    }

    /// The region of D whose void row is `row`, when that region has no lines and isn't an unresolved conflict, so typing there goes to it. `None` while local edits are unconfirmed: they may join regions, so region indices aren't known yet.
    pub fn empty_region_at_row(&self, row: usize) -> Option<usize> {
        self.choice_base()?;
        let pane_row = Rows::Mapped(&self.output_rows).row(row)?;
        let region = pane_row.region?;
        let state = self.merge.get(region)?;
        let empty = pane_row.line.is_none() && !pane_row.conflict_marker && state.output_lines == 0 && !state.is_unresolved();
        empty.then_some(region)
    }

    /// Applies an edit to D at once. `region` is the region without lines the edit goes to, which then has the edit's lines and is hand-edited. Returns the command to send, based on the revision the host will be at when it gets there.
    pub fn apply_local_edit(&mut self, edit: &Edit, region: Option<usize>) -> Option<OutputEdit> {
        let base = Revision(self.revision.0 + self.pending_edits.len() as u64);
        if !self.output.as_ref()?.fits(&edit.range) {
            return None;
        }
        self.fold_output_edit(&edit.range, &edit.text);
        let output = self.output.as_mut()?;
        if !output.replace(edit.range.clone(), &edit.text) {
            return None;
        }
        output.modified = true;
        if let Some(state) = region.and_then(|region| self.merge.get_mut(region)) {
            state.edited = true;
            state.output_lines = edit.text.matches('\n').count();
            self.refresh_conflicts();
        }
        self.refresh_output_rows();
        Some(OutputEdit {
            base,
            range: edit.range.clone(),
            text: edit.text.clone(),
            region,
        })
    }

    /// Records the request that carries a locally applied edit.
    pub fn edit_sent(&mut self, id: RequestId) {
        self.pending_edits.push_back(id);
    }

    /// Returns true if `id` was a pending edit; the local text is then wrong and needs a resync.
    pub fn edit_rejected(&self, id: &RequestId) -> bool {
        self.pending_edits.contains(id)
    }

    pub fn resync_requested(&mut self) {
        self.awaiting_resync = true;
    }

    pub fn apply_delta(&mut self, delta: Delta) -> DeltaOutcome {
        if self.awaiting_resync {
            return DeltaOutcome::Ignored;
        }
        if delta.revision.0 != self.revision.0 + 1 {
            return DeltaOutcome::NeedsResync;
        }
        let own_edit = matches!(delta.change, Change::OutputEdited { .. }) && delta.cause.is_some() && self.pending_edits.front() == delta.cause.as_ref();
        if own_edit {
            self.pending_edits.pop_front();
            self.revision = delta.revision;
            if let Change::OutputEdited { modified, merge, .. } = delta.change {
                if !self.apply_merge(merge) {
                    return DeltaOutcome::NeedsResync;
                }
                if self.pending_edits.is_empty()
                    && let Some(output) = self.output.as_mut()
                {
                    output.modified = modified;
                }
            }
            return DeltaOutcome::Applied;
        }
        let has_pending = !self.pending_edits.is_empty();
        match delta.change {
            Change::OutputEdited { range, text, modified, merge } => {
                if !has_pending && self.output.as_ref().is_some_and(|output| output.fits(&range)) {
                    self.fold_output_edit(&range, &text);
                }
                let applied = !has_pending && self.output.as_mut().is_some_and(|output| output.replace(range, &text));
                if !applied || !self.apply_merge(merge) {
                    return DeltaOutcome::NeedsResync;
                }
                if let Some(output) = self.output.as_mut() {
                    output.modified = modified;
                }
            }
            Change::OutputPathChanged(path) => match self.output.as_mut() {
                Some(output) => output.path = path,
                None => return DeltaOutcome::NeedsResync,
            },
            Change::OutputSaved(path) => match self.output.as_mut() {
                Some(output) => {
                    output.path = Some(path);
                    output.modified = has_pending;
                }
                None => return DeltaOutcome::NeedsResync,
            },
            Change::OutputEolChanged(eol) => match self.output.as_mut() {
                Some(output) => output.eol = eol,
                None => return DeltaOutcome::NeedsResync,
            },
            Change::AlignmentsChanged(alignments) => self.alignments = alignments,
        }
        self.revision = delta.revision;
        DeltaOutcome::Applied
    }
}

fn fold_index(document: Document) -> usize {
    match document {
        Document::A => 0,
        Document::B => 1,
        Document::C => 2,
        Document::D => 3,
    }
}


#[cfg(test)]
mod fold_tests {
    use kdiff4_protocol::engine::EngineCapabilities;
    use kdiff4_protocol::{EolChoice, OutputDocument};

    use super::*;

    fn mirror_with_output(text: &str) -> Mirror {
        Mirror::from_snapshot(
            Snapshot {
                revision: Revision(1),
                documents: Vec::new(),
                output: Some(OutputDocument {
                    path: None,
                    text: text.to_string(),
                    modified: false,
                    eol: EolChoice::default(),
                }),
                comparison: Comparison::default(),
                engine: EngineCapabilities::default(),
                merge: Vec::new(),
                options: ComparisonOptions::default(),
                current_region: None,
                char_diffs: Vec::new(),
                alignments: Alignments::default(),
            },
            false,
        )
    }


    fn ranges_of_d(revision: Revision, ranges: Vec<Range<usize>>) -> FoldRanges {
        FoldRanges {
            cause: None,
            revision,
            documents: vec![(Document::D, ranges)],
        }
    }


    #[test]
    fn edits_of_d_move_its_folds_and_ranges_found_before_them_are_dropped() {
        let mut mirror = mirror_with_output("a\nb\nc\nd\n");
        assert!(!mirror.set_fold_ranges(ranges_of_d(Revision(1), vec![1..4])));
        mirror.change_folds(Document::D, |folds| folds.fold(1..4));
        assert_eq!(Rows::Mapped(mirror.output_rows()).len(), 3);
        mirror.apply_local_edit(&Edit { range: 0..0, text: "x\n".to_string() }, None).unwrap();
        mirror.edit_sent(RequestId(7));
        assert_eq!(mirror.folds(Document::D).folded(), [2..5]);
        assert!(Rows::Mapped(mirror.output_rows()).is_line_hidden(4));
        assert!(mirror.set_fold_ranges(ranges_of_d(Revision(1), vec![0..2])));
        assert_eq!(mirror.folds(Document::D).ranges(), [2..5]);
    }


    #[test]
    fn removing_the_closing_token_shown_after_a_fold_ellipsis_unfolds_the_fold() {
        let mut mirror = mirror_with_output("f {\n  a\n}\nz\n");
        mirror.set_fold_ranges(ranges_of_d(Revision(1), vec![0..3]));
        mirror.change_folds(Document::D, |folds| folds.fold(0..3));
        mirror.apply_local_edit(&Edit { range: 8..9, text: String::new() }, None).unwrap();
        assert!(mirror.folds(Document::D).folded().is_empty());
        assert_eq!(mirror.output().unwrap().text, "f {\n  a\n\nz\n");
        assert!(!Rows::Mapped(mirror.output_rows()).is_line_hidden(2));
    }


    #[test]
    fn folds_carry_over_to_a_snapshot_with_the_same_text() {
        let mut previous = mirror_with_output("a\nb\nc\n");
        previous.set_fold_ranges(ranges_of_d(Revision(1), vec![0..3]));
        previous.change_folds(Document::D, |folds| folds.fold(0..3));
        let mut same = mirror_with_output("a\nb\nc\n");
        same.keep_folds_from(&previous);
        assert_eq!(same.folds(Document::D).folded(), [0..3]);
        assert_eq!(Rows::Mapped(same.output_rows()).len(), 2);
        let mut changed = mirror_with_output("a\nb\nz\n");
        changed.keep_folds_from(&previous);
        assert!(changed.folds(Document::D).folded().is_empty());
    }
}


#[cfg(test)]
mod tests {
    use kdiff4_protocol::engine::{EngineCapabilities, Region, Row};

    use crate::char_diff::CharStyle;
    use crate::rows::{Block, Mark, PaneRow, Rows};
    use kdiff4_protocol::{EolStyle, OutputDocument};

    use super::*;

    fn source(document: Document, text: &str) -> DocumentText {
        let lines = if text.is_empty() { Vec::new() } else { vec![0..text.len()] };
        DocumentText {
            document,
            path: PathBuf::from(format!("{}.txt", document.letter())),
            label: None,
            eol: Some(EolStyle::Lf),
            text: text.to_string(),
            lines,
        }
    }

    fn merge_mirror(text: &str) -> Mirror {
        let output = OutputDocument {
            path: Some(PathBuf::from("out.txt")),
            text: text.to_string(),
            modified: false,
            eol: EolChoice::default(),
        };
        Mirror::from_snapshot(
            Snapshot {
                revision: Revision(1),
                documents: vec![source(Document::A, "a"), source(Document::B, "b")],
                output: Some(output),
                comparison: Comparison::default(),
                engine: EngineCapabilities::default(),
                merge: Vec::new(),
                options: ComparisonOptions::default(),
                current_region: None,
                char_diffs: Vec::new(),
                alignments: Alignments::default(),
            },
            false,
        )
    }

    fn delta(revision: u64, cause: Option<u64>, change: Change) -> Delta {
        Delta {
            revision: Revision(revision),
            cause: cause.map(RequestId),
            change,
        }
    }

    fn no_merge() -> MergeUpdate {
        MergeUpdate {
            at: 0,
            removed: 0,
            regions: Vec::new(),
            merge: Vec::new(),
        }
    }

    fn unresolved() -> RegionMerge {
        RegionMerge {
            choice: Choice::Unresolved,
            edited: false,
            output_lines: 0,
        }
    }

    /// A 2-way merge of "a" and "b" with one modified region, unresolved, and D empty.
    fn conflict_mirror() -> Mirror {
        Mirror::from_snapshot(conflict_mirror_snapshot(), false)
    }


    fn conflict_mirror_snapshot() -> Snapshot {
        let output = OutputDocument {
            path: None,
            text: String::new(),
            modified: false,
            eol: EolChoice::default(),
        };
        Snapshot {
            revision: Revision(1),
            documents: vec![source(Document::A, "a"), source(Document::B, "b")],
            output: Some(output),
            comparison: comparison(&[RegionStatus::Diff(HunkChange::Modified)]),
            engine: EngineCapabilities::default(),
            merge: vec![unresolved()],
            options: ComparisonOptions::default(),
            current_region: None,
            char_diffs: Vec::new(),
            alignments: Alignments::default(),
        }
    }

    #[test]
    fn ignoring_whitespace_styles_gutter_blocks_from_the_char_diffs() {
        let snapshot = |ignore_whitespace: bool, char_diff: bool| {
            let mut snapshot = conflict_mirror_snapshot();
            snapshot.options.ignore_whitespace = ignore_whitespace;
            snapshot.engine.char_diff = char_diff;
            snapshot.char_diffs = vec![
                LineCharRuns {
                    lines: vec![vec![kdiff4_protocol::engine::CharRun { columns: 0..1, same_in: vec![true, true] }]],
                },
                LineCharRuns {
                    lines: vec![vec![kdiff4_protocol::engine::CharRun { columns: 0..1, same_in: vec![true, true] }]],
                },
            ];
            snapshot
        };
        let style = |mirror: &Mirror| mirror.rows(Document::A).and_then(|rows| Rows::Mapped(rows).block_of_row(0)).and_then(|block| block.style);
        assert_eq!(style(&Mirror::from_snapshot(snapshot(true, true), false)), None);
        assert_eq!(style(&Mirror::from_snapshot(snapshot(false, true), false)), Some(CharStyle::Document(Document::B)));
        assert_eq!(style(&Mirror::from_snapshot(snapshot(true, false), false)), Some(CharStyle::Document(Document::B)));
    }


    #[test]
    fn unresolved_conflicts_show_in_d_and_the_summary() {
        let mirror = conflict_mirror();
        assert_eq!(mirror.unresolved_count(), 1);
        assert_eq!(mirror.conflict_counts(), Some(ConflictCounts { unresolved: 1, total: 1 }));
        assert_eq!(mirror.difference_summary(), "1 difference; 1 unresolved");
        let rows = Rows::Mapped(mirror.output_rows());
        assert_eq!(
            rows.row(0),
            Some(PaneRow {
                region: Some(0),
                boxed: true,
                block: Some(Block {
                    index: 0,
                    origin: None,
                    style: None
                }),
                conflict_marker: true,
                ..PaneRow::default()
            })
        );
        assert_eq!(
            rows.row(1),
            Some(PaneRow {
                line: Some(0),
                ..PaneRow::default()
            })
        );
    }

    #[test]
    fn char_diffs_belong_to_their_documents() {
        let runs = |same_in: &[bool]| LineCharRuns {
            lines: vec![vec![kdiff4_protocol::engine::CharRun {
                columns: 0..1,
                same_in: same_in.to_vec(),
            }]],
        };
        let char_diffs = vec![runs(&[true, false]), runs(&[false, true])];
        let mirror = Mirror::from_snapshot(
            Snapshot {
                revision: Revision(1),
                documents: vec![source(Document::A, "a"), source(Document::B, "b")],
                output: None,
                comparison: Comparison::default(),
                engine: EngineCapabilities::default(),
                merge: Vec::new(),
                options: ComparisonOptions::default(),
                current_region: None,
                char_diffs: char_diffs.clone(),
                alignments: Alignments::default(),
            },
            false,
        );
        assert_eq!(mirror.char_diffs(Document::A), Some(&char_diffs[0]));
        assert_eq!(mirror.char_diffs(Document::B), Some(&char_diffs[1]));
        assert_eq!(mirror.char_diffs(Document::C), None);
        assert_eq!(mirror.char_diffs(Document::B).unwrap().line(5), []);
    }

    #[test]
    fn aligned_d_shares_the_source_rows() {
        let mut mirror = conflict_mirror();
        assert!(!mirror.is_aligned());
        mirror.set_aligned(true);
        assert!(mirror.is_aligned());
        assert_eq!(
            Rows::Mapped(mirror.output_rows()).region_start(0),
            Rows::Mapped(mirror.rows(Document::A).unwrap()).region_start(0)
        );
        assert_eq!(Rows::Mapped(mirror.output_rows()).row(0).unwrap().conflict_marker, true);
        let diff = Mirror::from_snapshot(
            Snapshot {
                revision: Revision(1),
                documents: vec![source(Document::A, "a")],
                output: None,
                comparison: Comparison::default(),
                engine: EngineCapabilities::default(),
                merge: Vec::new(),
                options: ComparisonOptions::default(),
                current_region: None,
                char_diffs: Vec::new(),
                alignments: Alignments::default(),
            },
            true,
        );
        assert!(!diff.is_aligned());
    }

    #[test]
    fn alignment_marks_follow_the_snapshot_and_their_deltas() {
        let mut mirror = conflict_mirror();
        assert_eq!(mirror.alignments(), &Alignments::default());
        let marked = Alignments {
            marks: vec![kdiff4_protocol::AlignmentMark {
                document: Document::A,
                number: 1,
                lines: 0..1,
                status: kdiff4_protocol::MarkStatus::Pending,
                replaced_by: None,
                heuristic: false,
            }],
            changed: false,
            ..Alignments::default()
        };
        assert_eq!(mirror.apply_delta(delta(2, None, Change::AlignmentsChanged(marked.clone()))), DeltaOutcome::Applied);
        assert_eq!(mirror.alignments(), &marked);
        assert_eq!(mirror.revision(), Revision(2));
        assert_eq!(
            mirror.apply_delta(delta(4, None, Change::AlignmentsChanged(Alignments::default()))),
            DeltaOutcome::NeedsResync
        );
        assert_eq!(mirror.alignments(), &marked);
    }

    #[test]
    fn current_region_follows_navigation_and_changes() {
        let mut mirror = conflict_mirror();
        assert_eq!(mirror.current_region(), None);
        assert!(mirror.apply_current_region(&CurrentRegion {
            revision: Revision(1),
            region: Some(0),
            cause: None
        }));
        assert_eq!(mirror.current_region(), Some(0));
        assert!(!mirror.apply_current_region(&CurrentRegion {
            revision: Revision(2),
            region: None,
            cause: None
        }));
        assert!(!mirror.apply_current_region(&CurrentRegion {
            revision: Revision(1),
            region: Some(5),
            cause: None
        }));
        assert_eq!(mirror.current_region(), Some(0));
        assert!(mirror.apply_current_region(&CurrentRegion {
            revision: Revision(1),
            region: None,
            cause: None
        }));
        let update = MergeUpdate {
            at: 0,
            removed: 1,
            regions: comparison(&[RegionStatus::Diff(HunkChange::Modified)]).regions,
            merge: vec![RegionMerge {
                choice: Choice::Hunks(vec![Document::B]),
                edited: false,
                output_lines: 1,
            }],
        };
        mirror.apply_delta(delta(
            2,
            None,
            Change::OutputEdited {
                range: 0..0,
                text: "b\n".to_string(),
                modified: true,
                merge: update,
            },
        ));
        assert_eq!(mirror.current_region(), Some(0));
    }

    #[test]
    fn choice_updates_regions_text_and_rows() {
        let mut mirror = conflict_mirror();
        let update = MergeUpdate {
            at: 0,
            removed: 1,
            regions: comparison(&[RegionStatus::Diff(HunkChange::Modified)]).regions,
            merge: vec![RegionMerge {
                choice: Choice::Hunks(vec![Document::B]),
                edited: false,
                output_lines: 1,
            }],
        };
        assert_eq!(
            mirror.apply_delta(delta(
                2,
                Some(3),
                Change::OutputEdited {
                    range: 0..0,
                    text: "b\n".to_string(),
                    modified: true,
                    merge: update
                }
            )),
            DeltaOutcome::Applied
        );
        assert_eq!(mirror.output().unwrap().text, "b\n");
        assert_eq!(mirror.unresolved_count(), 0);
        assert_eq!(mirror.difference_summary(), "1 difference; nothing unresolved");
        assert_eq!(
            Rows::Mapped(mirror.output_rows()).row(0),
            Some(PaneRow {
                line: Some(0),
                region: Some(0),
                boxed: true,
                block: Some(Block {
                    index: 0,
                    origin: None,
                    style: None
                }),
                origin: Some(Mark::Source(Document::B)),
                ..PaneRow::default()
            })
        );
        assert_eq!(Rows::Mapped(mirror.output_rows()).region_of_row(1), None);
    }

    #[test]
    fn merge_update_that_does_not_fit_needs_resync() {
        let mut mirror = conflict_mirror();
        let update = MergeUpdate {
            at: 3,
            removed: 1,
            regions: Vec::new(),
            merge: Vec::new(),
        };
        assert_eq!(
            mirror.apply_delta(delta(
                2,
                None,
                Change::OutputEdited {
                    range: 0..0,
                    text: String::new(),
                    modified: true,
                    merge: update
                }
            )),
            DeltaOutcome::NeedsResync
        );
    }

    #[test]
    fn regions_are_found_by_line() {
        let mirror = conflict_mirror();
        assert_eq!(Rows::Mapped(mirror.rows(Document::A).unwrap()).region_of_row(0), Some(0));
        assert_eq!(Rows::Mapped(mirror.rows(Document::B).unwrap()).region_of_row(0), Some(0));
        assert_eq!(Rows::Mapped(mirror.rows(Document::B).unwrap()).region_of_row(5), None);
        assert!(mirror.rows(Document::C).is_none());
    }

    #[test]
    fn toggling_adds_and_removes_hunks() {
        let mut mirror = conflict_mirror();
        assert_eq!(mirror.toggled_choice(0, Document::B), Some(Choice::Hunks(vec![Document::B])));
        mirror.merge[0] = RegionMerge {
            choice: Choice::Hunks(vec![Document::B]),
            edited: false,
            output_lines: 1,
        };
        assert_eq!(mirror.toggled_choice(0, Document::A), Some(Choice::Hunks(vec![Document::B, Document::A])));
        mirror.merge[0] = RegionMerge {
            choice: Choice::Hunks(vec![Document::B]),
            edited: false,
            output_lines: 1,
        };
        assert_eq!(mirror.toggled_choice(0, Document::B), Some(Choice::Unresolved));
        mirror.merge[0].edited = true;
        assert_eq!(mirror.toggled_choice(0, Document::B), Some(Choice::Hunks(vec![Document::B])));
        assert_eq!(mirror.toggled_choice(4, Document::B), None);
    }

    #[test]
    fn choices_wait_for_unconfirmed_edits() {
        let mut mirror = conflict_mirror();
        assert_eq!(mirror.choice_base(), Some(Revision(1)));
        mirror.edit_sent(RequestId(9));
        assert_eq!(mirror.choice_base(), None);
        assert_eq!(merge_mirror("x").choice_base(), None);
    }

    /// One single-row region per status; only the statuses matter here.
    fn comparison(statuses: &[RegionStatus]) -> Comparison {
        let regions = statuses
            .iter()
            .enumerate()
            .map(|(index, status)| Region {
                hunks: vec![index..index + 1, index..index + 1],
                status: status.clone(),
                rows: vec![Row {
                    lines: vec![Some(index), Some(index)],
                }],
            })
            .collect();
        Comparison { regions }
    }

    fn summary(statuses: &[RegionStatus]) -> String {
        let mut mirror = merge_mirror("");
        mirror.comparison = comparison(statuses);
        mirror.difference_summary()
    }

    #[test]
    fn snapshot_comparison_is_kept() {
        let snapshot = Snapshot {
            revision: Revision(1),
            documents: vec![source(Document::A, "a"), source(Document::B, "b")],
            output: None,
            comparison: comparison(&[RegionStatus::Diff(HunkChange::Modified)]),
            engine: EngineCapabilities::default(),
            merge: Vec::new(),
            options: ComparisonOptions::default(),
            current_region: None,
            char_diffs: Vec::new(),
            alignments: Alignments::default(),
        };
        let mirror = Mirror::from_snapshot(snapshot, false);
        assert_eq!(mirror.comparison(), &comparison(&[RegionStatus::Diff(HunkChange::Modified)]));
        assert_eq!(
            Rows::Mapped(mirror.rows(Document::A).unwrap()).row(0),
            Some(PaneRow {
                line: Some(0),
                region: Some(0),
                boxed: true,
                block: Some(Block {
                    index: 0,
                    origin: Some(Mark::Source(Document::A)),
                    style: Some(CharStyle::Document(Document::B))
                }),
                ..PaneRow::default()
            })
        );
        assert!(mirror.rows(Document::C).is_none());
    }

    #[test]
    fn difference_summaries() {
        let unchanged = RegionStatus::Merge(MergeStatus::Unchanged);
        let conflict = RegionStatus::Merge(MergeStatus::Conflict);
        let whitespace = RegionStatus::Merge(MergeStatus::WhitespaceConflict);
        assert_eq!(summary(&[]), "No differences");
        assert_eq!(summary(&[RegionStatus::Diff(HunkChange::Unchanged)]), "No differences");
        assert_eq!(summary(&[RegionStatus::Diff(HunkChange::Added), RegionStatus::Diff(HunkChange::Unchanged)]), "1 difference");
        assert_eq!(
            summary(&[RegionStatus::Merge(MergeStatus::ChangedInB), unchanged, RegionStatus::Merge(MergeStatus::ChangedAlike)]),
            "2 differences"
        );
        assert_eq!(summary(&[conflict, unchanged, RegionStatus::Merge(MergeStatus::ChangedInC)]), "2 differences, 1 conflict");
        assert_eq!(summary(&[conflict, conflict, whitespace]), "3 differences, 3 conflicts (1 whitespace only)");
        assert_eq!(summary(&[whitespace]), "1 difference, 1 conflict (whitespace only)");
    }

    #[test]
    fn typing_on_the_void_row_of_an_empty_region_goes_to_it() {
        let mut mirror = conflict_mirror();
        assert_eq!(mirror.empty_region_at_row(0), None, "an unresolved conflict's placeholder");
        mirror.merge[0] = RegionMerge {
            choice: Choice::Hunks(vec![Document::A]),
            edited: false,
            output_lines: 0,
        };
        mirror.refresh_output_rows();
        assert_eq!(
            Rows::Mapped(mirror.output_rows()).row(0),
            Some(PaneRow {
                region: Some(0),
                boxed: true,
                block: Some(Block {
                    index: 0,
                    origin: None,
                    style: None
                }),
                void_source: Some(Document::A),
                ..PaneRow::default()
            })
        );
        assert_eq!(mirror.empty_region_at_row(0), Some(0));
        assert_eq!(mirror.empty_region_at_row(1), None);
        let command = mirror
            .apply_local_edit(
                &Edit {
                    range: 0..0,
                    text: "x\n".to_string(),
                },
                Some(0),
            )
            .unwrap();
        assert_eq!(command.region, Some(0));
        assert_eq!(
            mirror.merge[0],
            RegionMerge {
                choice: Choice::Hunks(vec![Document::A]),
                edited: true,
                output_lines: 1
            }
        );
        assert_eq!(Rows::Mapped(mirror.output_rows()).row(0).and_then(|row| row.line), Some(0));
        mirror.edit_sent(RequestId(5));
        mirror.merge[0].output_lines = 0;
        mirror.refresh_output_rows();
        assert_eq!(mirror.empty_region_at_row(0), None, "unconfirmed edits may have moved regions");
    }

    fn local_edit(mirror: &mut Mirror, range: Range<usize>, text: &str, id: u64) -> OutputEdit {
        let command = mirror.apply_local_edit(&Edit { range, text: text.to_string() }, None).unwrap();
        mirror.edit_sent(RequestId(id));
        command
    }

    #[test]
    fn snapshot_builds_editor_lines_for_output() {
        let mirror = merge_mirror("x\ny\n");
        let output = mirror.output().unwrap();
        assert_eq!(output.lines, [0..1, 2..3, 4..4]);
        assert_eq!(mirror.document(Document::B).unwrap().text, "b");
        assert!(mirror.document(Document::C).is_none());
        assert!(mirror.is_merge());
    }

    #[test]
    fn local_edits_apply_at_once_with_increasing_base_revisions() {
        let mut mirror = merge_mirror("");
        let first = local_edit(&mut mirror, 0..0, "ab", 10);
        let second = local_edit(&mut mirror, 2..2, "\nc", 11);
        assert_eq!(first.base, Revision(1));
        assert_eq!(second.base, Revision(2));
        let output = mirror.output().unwrap();
        assert_eq!(output.text, "ab\nc");
        assert_eq!(output.lines, [0..2, 3..4]);
        assert!(output.modified);
    }

    #[test]
    fn own_edit_deltas_are_not_applied_twice() {
        let mut mirror = merge_mirror("");
        local_edit(&mut mirror, 0..0, "ab", 10);
        let outcome = mirror.apply_delta(delta(
            2,
            Some(10),
            Change::OutputEdited {
                range: 0..0,
                text: "ab".to_string(),
                modified: true,
                merge: no_merge(),
            },
        ));
        assert_eq!(outcome, DeltaOutcome::Applied);
        assert_eq!(mirror.output().unwrap().text, "ab");
        assert_eq!(mirror.revision(), Revision(2));
    }

    #[test]
    fn foreign_changes_are_applied() {
        let mut mirror = merge_mirror("x");
        assert_eq!(
            mirror.apply_delta(delta(
                2,
                None,
                Change::OutputEdited {
                    range: 1..1,
                    text: "y".to_string(),
                    modified: true,
                    merge: no_merge()
                }
            )),
            DeltaOutcome::Applied
        );
        assert_eq!(mirror.output().unwrap().text, "xy");
        assert_eq!(mirror.apply_delta(delta(3, Some(6), Change::OutputPathChanged(None))), DeltaOutcome::Applied);
        assert_eq!(mirror.output().unwrap().path, None);
        assert_eq!(mirror.apply_delta(delta(4, Some(7), Change::OutputSaved(PathBuf::from("s.txt")))), DeltaOutcome::Applied);
        let output = mirror.output().unwrap();
        assert_eq!(output.path, Some(PathBuf::from("s.txt")));
        assert!(!output.modified);
        assert_eq!(mirror.revision(), Revision(4));
    }

    #[test]
    fn a_new_output_line_ending_is_applied() {
        let mut mirror = merge_mirror("x");
        assert_eq!(mirror.output().unwrap().eol, EolChoice::Style(EolStyle::Lf));
        assert_eq!(mirror.apply_delta(delta(2, None, Change::OutputEolChanged(EolChoice::Conflict))), DeltaOutcome::Applied);
        assert_eq!(mirror.output().unwrap().eol, EolChoice::Conflict);
        assert_eq!(mirror.apply_delta(delta(3, Some(4), Change::OutputEolChanged(EolChoice::Style(EolStyle::Cr)))), DeltaOutcome::Applied);
        assert_eq!(mirror.output().unwrap().eol, EolChoice::Style(EolStyle::Cr));
        assert_eq!(mirror.revision(), Revision(3));
    }

    #[test]
    fn undo_back_to_saved_text_clears_modified() {
        let mut mirror = merge_mirror("x");
        assert_eq!(
            mirror.apply_delta(delta(
                2,
                Some(3),
                Change::OutputEdited {
                    range: 1..1,
                    text: "y".to_string(),
                    modified: true,
                    merge: no_merge()
                }
            )),
            DeltaOutcome::Applied
        );
        assert!(mirror.output().unwrap().modified);
        assert_eq!(
            mirror.apply_delta(delta(
                3,
                Some(4),
                Change::OutputEdited {
                    range: 1..2,
                    text: String::new(),
                    modified: false,
                    merge: no_merge()
                }
            )),
            DeltaOutcome::Applied
        );
        let output = mirror.output().unwrap();
        assert_eq!(output.text, "x");
        assert!(!output.modified);
    }

    #[test]
    fn save_with_unconfirmed_edits_stays_modified() {
        let mut mirror = merge_mirror("");
        local_edit(&mut mirror, 0..0, "a", 10);
        assert_eq!(
            mirror.apply_delta(delta(
                2,
                Some(10),
                Change::OutputEdited {
                    range: 0..0,
                    text: "a".to_string(),
                    modified: true,
                    merge: no_merge()
                }
            )),
            DeltaOutcome::Applied
        );
        local_edit(&mut mirror, 1..1, "b", 12);
        assert_eq!(mirror.apply_delta(delta(3, Some(11), Change::OutputSaved(PathBuf::from("out.txt")))), DeltaOutcome::Applied);
        assert!(mirror.output().unwrap().modified);
    }

    #[test]
    fn revision_gap_needs_resync_and_later_deltas_are_ignored() {
        let mut mirror = merge_mirror("");
        assert_eq!(mirror.apply_delta(delta(3, None, Change::OutputPathChanged(None))), DeltaOutcome::NeedsResync);
        mirror.resync_requested();
        assert_eq!(mirror.apply_delta(delta(2, None, Change::OutputPathChanged(None))), DeltaOutcome::Ignored);
        assert_eq!(mirror.output().unwrap().path, Some(PathBuf::from("out.txt")));
    }

    #[test]
    fn foreign_output_edit_during_pending_edits_needs_resync() {
        let mut mirror = merge_mirror("");
        local_edit(&mut mirror, 0..0, "a", 10);
        assert_eq!(
            mirror.apply_delta(delta(
                2,
                None,
                Change::OutputEdited {
                    range: 0..0,
                    text: "z".to_string(),
                    modified: true,
                    merge: no_merge()
                }
            )),
            DeltaOutcome::NeedsResync
        );
    }

    #[test]
    fn rejected_pending_edit_is_reported() {
        let mut mirror = merge_mirror("");
        local_edit(&mut mirror, 0..0, "a", 10);
        assert!(mirror.edit_rejected(&RequestId(10)));
        assert!(!mirror.edit_rejected(&RequestId(11)));
    }

    #[test]
    fn diff_mirror_has_no_output_and_rejects_edits() {
        let mut mirror = Mirror::from_snapshot(
            Snapshot {
                revision: Revision(1),
                documents: vec![source(Document::A, "a")],
                output: None,
                comparison: Comparison::default(),
                engine: EngineCapabilities::default(),
                merge: Vec::new(),
                options: ComparisonOptions::default(),
                current_region: None,
                char_diffs: Vec::new(),
                alignments: Alignments::default(),
            },
            false,
        );
        assert!(!mirror.is_merge());
        assert!(
            mirror
                .apply_local_edit(
                    &Edit {
                        range: 0..0,
                        text: "x".to_string()
                    },
                    None
                )
                .is_none()
        );
        assert_eq!(mirror.apply_delta(delta(2, None, Change::OutputPathChanged(None))), DeltaOutcome::NeedsResync);
    }

    #[test]
    fn local_edit_with_bad_range_is_refused() {
        let mut mirror = merge_mirror("é");
        assert!(
            mirror
                .apply_local_edit(
                    &Edit {
                        range: 1..1,
                        text: "x".to_string()
                    },
                    None
                )
                .is_none()
        );
        assert!(
            mirror
                .apply_local_edit(
                    &Edit {
                        range: 0..9,
                        text: "x".to_string()
                    },
                    None
                )
                .is_none()
        );
        assert_eq!(mirror.output().unwrap().text, "é");
    }
}
