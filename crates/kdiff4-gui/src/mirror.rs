use std::collections::VecDeque;
use std::ops::Range;
use std::path::PathBuf;

use kdiff4_protocol::engine::{Comparison, HunkChange, MergeStatus, RegionStatus};
use kdiff4_protocol::{Change, Choice, Delta, Document, DocumentText, MergeUpdate, OutputEdit, RegionMerge, RequestId, Revision, Snapshot};

use crate::editing::{self, Edit, Lines};
use crate::rows::RowMap;


/// The client's copy of D.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputMirror {
    pub path: Option<PathBuf>,
    pub text: String,
    pub lines: Vec<Range<usize>>,
    pub modified: bool,
}


impl OutputMirror {
    pub fn lines(&self) -> Lines<'_> {
        Lines { text: &self.text, ranges: &self.lines }
    }


    fn replace(&mut self, range: Range<usize>, text: &str) -> bool {
        let fits = range.start <= range.end && range.end <= self.text.len() && self.text.is_char_boundary(range.start) && self.text.is_char_boundary(range.end);
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


/// The client's copy of the session, built from a snapshot and the deltas after it. D edits are applied here at once and confirmed by the host later.
#[derive(Debug, Clone)]
pub struct Mirror {
    revision: Revision,
    documents: Vec<DocumentText>,
    output: Option<OutputMirror>,
    comparison: Comparison,
    /// Per document, in `documents` order: its rows in the synchronized view.
    row_maps: Vec<RowMap>,
    /// Per region, its merge state; empty without D.
    merge: Vec<RegionMerge>,
    /// D's rows: its lines and the placeholders of unresolved conflicts.
    output_rows: RowMap,
    /// Requests of edits applied locally that the host has not confirmed yet, oldest first.
    pending_edits: VecDeque<RequestId>,
    awaiting_resync: bool,
}


impl Mirror {
    pub fn from_snapshot(snapshot: Snapshot) -> Mirror {
        let output = snapshot.output.map(|output| {
            let lines = editing::editor_lines(&output.text);
            OutputMirror { path: output.path, text: output.text, lines, modified: output.modified }
        });
        let row_maps = row_maps(&snapshot.documents, &snapshot.comparison);
        let output_rows = RowMap::for_output(&snapshot.merge, output.as_ref().map_or(0, |output| output.lines.len()));
        Mirror { revision: snapshot.revision, documents: snapshot.documents, output, comparison: snapshot.comparison, row_maps, merge: snapshot.merge, output_rows, pending_edits: VecDeque::new(), awaiting_resync: false }
    }


    #[cfg(test)]
    pub fn revision(&self) -> Revision {
        self.revision.clone()
    }


    pub fn documents(&self) -> &[DocumentText] {
        &self.documents
    }


    pub fn document(&self, document: Document) -> Option<&DocumentText> {
        self.documents
            .iter()
            .find(|text| text.document == document)
    }


    pub fn rows(&self, document: Document) -> Option<&RowMap> {
        self.documents
            .iter()
            .position(|text| text.document == document)
            .and_then(|index| self.row_maps.get(index))
    }


    pub fn output(&self) -> Option<&OutputMirror> {
        self.output.as_ref()
    }


    pub fn is_merge(&self) -> bool {
        self.output.is_some()
    }


    #[cfg(test)]
    pub fn comparison(&self) -> &Comparison {
        &self.comparison
    }


    pub fn output_rows(&self) -> &RowMap {
        &self.output_rows
    }


    pub fn unresolved_count(&self) -> usize {
        self.merge
            .iter()
            .filter(|state| state.is_unresolved())
            .count()
    }


    /// The revision a choice command is based on, or `None` while local edits are unconfirmed: their deltas may join regions, so region indices aren't known yet.
    pub fn choice_base(&self) -> Option<Revision> {
        (self.pending_edits.is_empty() && !self.awaiting_resync && !self.merge.is_empty()).then(|| self.revision.clone())
    }


    /// The region holding `line` of `document`: for A, B and C by its hunks, for D by the lines each region contributes.
    pub fn region_at(&self, document: Document, line: usize) -> Option<usize> {
        if document == Document::D {
            let mut end = 0;
            return self.merge.iter().position(|state| {
                end += state.output_lines;
                line < end
            });
        }
        let index = self.documents.iter().position(|text| text.document == document)?;
        self.comparison
            .regions
            .iter()
            .position(|region| region.hunks.get(index).is_some_and(|hunk| hunk.contains(&line)))
    }


    /// The choice after toggling `document` in region `region`: added at the end, or removed if it is already chosen. An unresolved or hand-edited region starts from no choice.
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
        Some(Choice::Hunks(documents))
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
        self.row_maps = row_maps(&self.documents, &self.comparison);
        self.refresh_output_rows();
        true
    }


    fn refresh_output_rows(&mut self) {
        let line_count = self.output.as_ref().map_or(0, |output| output.lines.len());
        self.output_rows = RowMap::for_output(&self.merge, line_count);
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
        let changed = self.comparison
            .regions
            .iter()
            .filter(|region| !matches!(region.status, RegionStatus::Diff(HunkChange::Unchanged) | RegionStatus::Merge(MergeStatus::Unchanged)))
            .count();
        let conflicts = self.comparison
            .regions
            .iter()
            .filter(|region| matches!(region.status, RegionStatus::Merge(MergeStatus::Conflict | MergeStatus::WhitespaceConflict)))
            .count();
        let whitespace = self.comparison
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


    /// Applies an edit to D at once. Returns the command to send, based on the revision the host will be at when it gets there.
    pub fn apply_local_edit(&mut self, edit: &Edit) -> Option<OutputEdit> {
        let base = Revision(self.revision.0 + self.pending_edits.len() as u64);
        let output = self.output.as_mut()?;
        if !output.replace(edit.range.clone(), &edit.text) {
            return None;
        }
        output.modified = true;
        self.refresh_output_rows();
        Some(OutputEdit { base, range: edit.range.clone(), text: edit.text.clone() })
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
        }
        self.revision = delta.revision;
        DeltaOutcome::Applied
    }
}


fn row_maps(documents: &[DocumentText], comparison: &Comparison) -> Vec<RowMap> {
    documents
        .iter()
        .enumerate()
        .map(|(index, text)| RowMap::for_source(comparison, index, text.line_count()))
        .collect()
}


#[cfg(test)]
mod tests {
    use kdiff4_protocol::engine::{EngineCapabilities, Region, Row};

    use crate::rows::{PaneRow, Rows, Shade};
    use kdiff4_protocol::{EolStyle, OutputDocument};

    use super::*;


    fn source(document: Document, text: &str) -> DocumentText {
        let lines = if text.is_empty() { Vec::new() } else { vec![0..text.len()] };
        DocumentText { document, path: PathBuf::from(format!("{}.txt", document.letter())), label: None, eol: EolStyle::Lf, text: text.to_string(), lines }
    }


    fn merge_mirror(text: &str) -> Mirror {
        let output = OutputDocument { path: Some(PathBuf::from("out.txt")), text: text.to_string(), modified: false };
        Mirror::from_snapshot(Snapshot { revision: Revision(1), documents: vec![source(Document::A, "a"), source(Document::B, "b")], output: Some(output), comparison: Comparison::default(), engine: EngineCapabilities::default(), merge: Vec::new() })
    }


    fn delta(revision: u64, cause: Option<u64>, change: Change) -> Delta {
        Delta { revision: Revision(revision), cause: cause.map(RequestId), change }
    }


    fn no_merge() -> MergeUpdate {
        MergeUpdate { at: 0, removed: 0, regions: Vec::new(), merge: Vec::new() }
    }


    fn unresolved() -> RegionMerge {
        RegionMerge { choice: Choice::Unresolved, edited: false, output_lines: 0 }
    }


    /// A 2-way merge of "a" and "b" with one modified region, unresolved, and D empty.
    fn conflict_mirror() -> Mirror {
        let output = OutputDocument { path: None, text: String::new(), modified: false };
        let snapshot = Snapshot { revision: Revision(1), documents: vec![source(Document::A, "a"), source(Document::B, "b")], output: Some(output), comparison: comparison(&[RegionStatus::Diff(HunkChange::Modified)]), engine: EngineCapabilities::default(), merge: vec![unresolved()] };
        Mirror::from_snapshot(snapshot)
    }


    #[test]
    fn unresolved_conflicts_show_in_d_and_the_summary() {
        let mirror = conflict_mirror();
        assert_eq!(mirror.unresolved_count(), 1);
        assert_eq!(mirror.difference_summary(), "1 difference; 1 unresolved");
        let rows = Rows::Mapped(mirror.output_rows());
        assert_eq!(rows.row(0), Some(PaneRow { line: None, shade: Some(Shade::Conflict) }));
        assert_eq!(rows.row(1), Some(PaneRow { line: Some(0), shade: None }));
    }


    #[test]
    fn choice_updates_regions_text_and_rows() {
        let mut mirror = conflict_mirror();
        let update = MergeUpdate { at: 0, removed: 1, regions: comparison(&[RegionStatus::Diff(HunkChange::Modified)]).regions, merge: vec![RegionMerge { choice: Choice::Hunks(vec![Document::B]), edited: false, output_lines: 1 }] };
        assert_eq!(mirror.apply_delta(delta(2, Some(3), Change::OutputEdited { range: 0..0, text: "b\n".to_string(), modified: true, merge: update })), DeltaOutcome::Applied);
        assert_eq!(mirror.output().unwrap().text, "b\n");
        assert_eq!(mirror.unresolved_count(), 0);
        assert_eq!(mirror.difference_summary(), "1 difference; nothing unresolved");
        assert_eq!(Rows::Mapped(mirror.output_rows()).row(0), Some(PaneRow { line: Some(0), shade: None }));
        assert_eq!(mirror.region_at(Document::D, 0), Some(0));
        assert_eq!(mirror.region_at(Document::D, 1), None);
    }


    #[test]
    fn merge_update_that_does_not_fit_needs_resync() {
        let mut mirror = conflict_mirror();
        let update = MergeUpdate { at: 3, removed: 1, regions: Vec::new(), merge: Vec::new() };
        assert_eq!(mirror.apply_delta(delta(2, None, Change::OutputEdited { range: 0..0, text: String::new(), modified: true, merge: update })), DeltaOutcome::NeedsResync);
    }


    #[test]
    fn regions_are_found_by_line() {
        let mirror = conflict_mirror();
        assert_eq!(mirror.region_at(Document::A, 0), Some(0));
        assert_eq!(mirror.region_at(Document::B, 0), Some(0));
        assert_eq!(mirror.region_at(Document::B, 5), None);
        assert_eq!(mirror.region_at(Document::C, 0), None);
    }


    #[test]
    fn toggling_adds_and_removes_hunks() {
        let mut mirror = conflict_mirror();
        assert_eq!(mirror.toggled_choice(0, Document::B), Some(Choice::Hunks(vec![Document::B])));
        mirror.merge[0] = RegionMerge { choice: Choice::Hunks(vec![Document::B]), edited: false, output_lines: 1 };
        assert_eq!(mirror.toggled_choice(0, Document::A), Some(Choice::Hunks(vec![Document::B, Document::A])));
        assert_eq!(mirror.toggled_choice(0, Document::B), Some(Choice::Hunks(Vec::new())));
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
            .map(|(index, status)| Region { hunks: vec![index..index + 1, index..index + 1], status: status.clone(), rows: vec![Row { lines: vec![Some(index), Some(index)] }] })
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
        let snapshot = Snapshot { revision: Revision(1), documents: vec![source(Document::A, "a"), source(Document::B, "b")], output: None, comparison: comparison(&[RegionStatus::Diff(HunkChange::Modified)]), engine: EngineCapabilities::default(), merge: Vec::new() };
        let mirror = Mirror::from_snapshot(snapshot);
        assert_eq!(mirror.comparison(), &comparison(&[RegionStatus::Diff(HunkChange::Modified)]));
        assert_eq!(Rows::Mapped(mirror.rows(Document::A).unwrap()).row(0), Some(PaneRow { line: Some(0), shade: Some(Shade::Modified) }));
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
        assert_eq!(summary(&[RegionStatus::Merge(MergeStatus::ChangedInB), unchanged, RegionStatus::Merge(MergeStatus::ChangedAlike)]), "2 differences");
        assert_eq!(summary(&[conflict, unchanged, RegionStatus::Merge(MergeStatus::ChangedInC)]), "2 differences, 1 conflict");
        assert_eq!(summary(&[conflict, conflict, whitespace]), "3 differences, 3 conflicts (1 whitespace only)");
        assert_eq!(summary(&[whitespace]), "1 difference, 1 conflict (whitespace only)");
    }


    fn local_edit(mirror: &mut Mirror, range: Range<usize>, text: &str, id: u64) -> OutputEdit {
        let command = mirror.apply_local_edit(&Edit { range, text: text.to_string() }).unwrap();
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
        let outcome = mirror.apply_delta(delta(2, Some(10), Change::OutputEdited { range: 0..0, text: "ab".to_string(), modified: true, merge: no_merge() }));
        assert_eq!(outcome, DeltaOutcome::Applied);
        assert_eq!(mirror.output().unwrap().text, "ab");
        assert_eq!(mirror.revision(), Revision(2));
    }


    #[test]
    fn foreign_changes_are_applied() {
        let mut mirror = merge_mirror("x");
        assert_eq!(mirror.apply_delta(delta(2, None, Change::OutputEdited { range: 1..1, text: "y".to_string(), modified: true, merge: no_merge() })), DeltaOutcome::Applied);
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
    fn undo_back_to_saved_text_clears_modified() {
        let mut mirror = merge_mirror("x");
        assert_eq!(mirror.apply_delta(delta(2, Some(3), Change::OutputEdited { range: 1..1, text: "y".to_string(), modified: true, merge: no_merge() })), DeltaOutcome::Applied);
        assert!(mirror.output().unwrap().modified);
        assert_eq!(mirror.apply_delta(delta(3, Some(4), Change::OutputEdited { range: 1..2, text: String::new(), modified: false, merge: no_merge() })), DeltaOutcome::Applied);
        let output = mirror.output().unwrap();
        assert_eq!(output.text, "x");
        assert!(!output.modified);
    }


    #[test]
    fn save_with_unconfirmed_edits_stays_modified() {
        let mut mirror = merge_mirror("");
        local_edit(&mut mirror, 0..0, "a", 10);
        assert_eq!(mirror.apply_delta(delta(2, Some(10), Change::OutputEdited { range: 0..0, text: "a".to_string(), modified: true, merge: no_merge() })), DeltaOutcome::Applied);
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
        assert_eq!(mirror.apply_delta(delta(2, None, Change::OutputEdited { range: 0..0, text: "z".to_string(), modified: true, merge: no_merge() })), DeltaOutcome::NeedsResync);
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
        let mut mirror = Mirror::from_snapshot(Snapshot { revision: Revision(1), documents: vec![source(Document::A, "a")], output: None, comparison: Comparison::default(), engine: EngineCapabilities::default(), merge: Vec::new() });
        assert!(!mirror.is_merge());
        assert!(mirror.apply_local_edit(&Edit { range: 0..0, text: "x".to_string() }).is_none());
        assert_eq!(mirror.apply_delta(delta(2, None, Change::OutputPathChanged(None))), DeltaOutcome::NeedsResync);
    }


    #[test]
    fn local_edit_with_bad_range_is_refused() {
        let mut mirror = merge_mirror("é");
        assert!(mirror.apply_local_edit(&Edit { range: 1..1, text: "x".to_string() }).is_none());
        assert!(mirror.apply_local_edit(&Edit { range: 0..9, text: "x".to_string() }).is_none());
        assert_eq!(mirror.output().unwrap().text, "é");
    }
}
