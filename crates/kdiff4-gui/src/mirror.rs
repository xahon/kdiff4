use std::collections::VecDeque;
use std::ops::Range;
use std::path::PathBuf;

use kdiff4_protocol::{Change, Delta, Document, DocumentText, OutputEdit, RequestId, Revision, Snapshot};

use crate::editing::{self, Edit, Lines};


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
        Mirror { revision: snapshot.revision, documents: snapshot.documents, output, pending_edits: VecDeque::new(), awaiting_resync: false }
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


    pub fn output(&self) -> Option<&OutputMirror> {
        self.output.as_ref()
    }


    pub fn is_merge(&self) -> bool {
        self.output.is_some()
    }


    /// Applies an edit to D at once. Returns the command to send, based on the revision the host will be at when it gets there.
    pub fn apply_local_edit(&mut self, edit: &Edit) -> Option<OutputEdit> {
        let base = Revision(self.revision.0 + self.pending_edits.len() as u64);
        let output = self.output.as_mut()?;
        if !output.replace(edit.range.clone(), &edit.text) {
            return None;
        }
        output.modified = true;
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
            if let Change::OutputEdited { modified, .. } = delta.change
                && self.pending_edits.is_empty()
                && let Some(output) = self.output.as_mut()
            {
                output.modified = modified;
            }
            return DeltaOutcome::Applied;
        }
        let has_pending = !self.pending_edits.is_empty();
        match delta.change {
            Change::SourceReplaced(text) => match self.documents.iter_mut().find(|existing| existing.document == text.document) {
                Some(existing) => *existing = text,
                None => return DeltaOutcome::NeedsResync,
            },
            Change::OutputEdited { range, text, modified } => {
                let applied = !has_pending && self.output.as_mut().is_some_and(|output| output.replace(range, &text));
                if !applied {
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


#[cfg(test)]
mod tests {
    use kdiff4_protocol::{EolStyle, OutputDocument};

    use super::*;


    fn source(document: Document, text: &str) -> DocumentText {
        let lines = if text.is_empty() { Vec::new() } else { vec![0..text.len()] };
        DocumentText { document, path: PathBuf::from(format!("{}.txt", document.letter())), label: None, eol: EolStyle::Lf, text: text.to_string(), lines }
    }


    fn merge_mirror(text: &str) -> Mirror {
        let output = OutputDocument { path: Some(PathBuf::from("out.txt")), text: text.to_string(), modified: false };
        Mirror::from_snapshot(Snapshot { revision: Revision(1), documents: vec![source(Document::A, "a"), source(Document::B, "b")], output: Some(output) })
    }


    fn delta(revision: u64, cause: Option<u64>, change: Change) -> Delta {
        Delta { revision: Revision(revision), cause: cause.map(RequestId), change }
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
        let outcome = mirror.apply_delta(delta(2, Some(10), Change::OutputEdited { range: 0..0, text: "ab".to_string(), modified: true }));
        assert_eq!(outcome, DeltaOutcome::Applied);
        assert_eq!(mirror.output().unwrap().text, "ab");
        assert_eq!(mirror.revision(), Revision(2));
    }


    #[test]
    fn foreign_changes_are_applied() {
        let mut mirror = merge_mirror("x");
        assert_eq!(mirror.apply_delta(delta(2, None, Change::OutputEdited { range: 1..1, text: "y".to_string(), modified: true })), DeltaOutcome::Applied);
        assert_eq!(mirror.output().unwrap().text, "xy");
        assert_eq!(mirror.apply_delta(delta(3, Some(5), Change::SourceReplaced(source(Document::B, "new")))), DeltaOutcome::Applied);
        assert_eq!(mirror.document(Document::B).unwrap().text, "new");
        assert_eq!(mirror.apply_delta(delta(4, Some(6), Change::OutputPathChanged(None))), DeltaOutcome::Applied);
        assert_eq!(mirror.output().unwrap().path, None);
        assert_eq!(mirror.apply_delta(delta(5, Some(7), Change::OutputSaved(PathBuf::from("s.txt")))), DeltaOutcome::Applied);
        let output = mirror.output().unwrap();
        assert_eq!(output.path, Some(PathBuf::from("s.txt")));
        assert!(!output.modified);
        assert_eq!(mirror.revision(), Revision(5));
    }


    #[test]
    fn undo_back_to_saved_text_clears_modified() {
        let mut mirror = merge_mirror("x");
        assert_eq!(mirror.apply_delta(delta(2, Some(3), Change::OutputEdited { range: 1..1, text: "y".to_string(), modified: true })), DeltaOutcome::Applied);
        assert!(mirror.output().unwrap().modified);
        assert_eq!(mirror.apply_delta(delta(3, Some(4), Change::OutputEdited { range: 1..2, text: String::new(), modified: false })), DeltaOutcome::Applied);
        let output = mirror.output().unwrap();
        assert_eq!(output.text, "x");
        assert!(!output.modified);
    }


    #[test]
    fn save_with_unconfirmed_edits_stays_modified() {
        let mut mirror = merge_mirror("");
        local_edit(&mut mirror, 0..0, "a", 10);
        assert_eq!(mirror.apply_delta(delta(2, Some(10), Change::OutputEdited { range: 0..0, text: "a".to_string(), modified: true })), DeltaOutcome::Applied);
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
        assert_eq!(mirror.apply_delta(delta(2, None, Change::OutputEdited { range: 0..0, text: "z".to_string(), modified: true })), DeltaOutcome::NeedsResync);
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
        let mut mirror = Mirror::from_snapshot(Snapshot { revision: Revision(1), documents: vec![source(Document::A, "a")], output: None });
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
