use std::ops::Range;

use kdiff4_protocol::engine::{EngineError, ManualAlignment, validate_alignments};
use kdiff4_protocol::{AlignmentMark, Alignments, Document, MarkStatus};

use crate::error::CoreError;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Mark {
    /// Index into the session's source documents.
    document: usize,
    number: u32,
    /// An empty range is an anchor: the position before that line.
    lines: Range<usize>,
}

/// The marks the user made, and the alignments the comparison was last computed with. A number marked in every source document is an alignment; the others wait. Marking never touches the comparison: only applying does.
#[derive(Debug, Clone, Default)]
pub struct AlignmentMarks {
    /// Ordered by document, then number. At most one mark per document and number.
    marks: Vec<Mark>,
    applied: Vec<ManualAlignment>,
}

impl AlignmentMarks {
    /// Marks `lines` of `document` under the lowest number that document has not used, and returns it. `line_counts` has one entry per source document.
    pub fn mark(&mut self, document: Document, lines: Range<usize>, line_counts: &[usize]) -> Result<u32, CoreError> {
        let index = document_index(&document, line_counts.len())?;
        let count = line_counts[index];
        if lines.start > lines.end || lines.end > count {
            return Err(invalid(format!(
                "lines {}..{} do not fit the {count} lines of {}",
                lines.start,
                lines.end,
                document.letter()
            )));
        }
        if let Some(other) = self.of(index).find(|mark| collide(&mark.lines, &lines)) {
            return Err(invalid(format!("alignment {} of {} is in the way of {}", other.number, document.letter(), subject(&lines))));
        }
        let number = (1..)
            .find(|number| self.get(index, *number).is_none())
            .ok_or_else(|| invalid("no alignment number is free".to_string()))?;
        self.marks.push(Mark { document: index, number, lines });
        self.sort();
        Ok(number)
    }

    pub fn remove(&mut self, document: Document, number: u32) -> Result<(), CoreError> {
        let index = self.index_of(&document, number)?;
        self.marks.remove(index);
        Ok(())
    }

    /// Gives the mark `number` of `document` the number `to`; a mark that has `to` already takes `number`.
    pub fn renumber(&mut self, document: Document, number: u32, to: u32) -> Result<(), CoreError> {
        if to == 0 {
            return Err(invalid("alignment numbers start at 1".to_string()));
        }
        self.index_of(&document, number)?;
        if to == number {
            return Ok(());
        }
        let document = document_index(&document, usize::MAX)?;
        for mark in self.marks.iter_mut().filter(|mark| mark.document == document) {
            if mark.number == number {
                mark.number = to;
            } else if mark.number == to {
                mark.number = number;
            }
        }
        self.sort();
        Ok(())
    }

    /// Removes every mark. The applied alignments stay, since the comparison still has them.
    pub fn clear(&mut self) {
        self.marks.clear();
    }

    /// Removes the marks and the applied alignments, for a session whose source lines changed.
    pub fn reset(&mut self) {
        self.marks.clear();
        self.applied.clear();
    }

    pub fn applied(&self) -> &[ManualAlignment] {
        &self.applied
    }

    pub fn set_applied(&mut self, alignments: Vec<ManualAlignment>) {
        self.applied = alignments;
    }

    /// The alignments the marks give: every number marked in all source documents, in the order of their positions. Checked like the engine checks them, but errors name the user's numbers instead of list positions.
    pub fn to_apply(&self, line_counts: &[usize]) -> Result<Vec<ManualAlignment>, EngineError> {
        let numbered = self.complete(line_counts.len());
        let alignments: Vec<ManualAlignment> = numbered.iter().map(|(_, alignment)| alignment.clone()).collect();
        let error = match validate_alignments(&alignments, line_counts) {
            Ok(()) => return Ok(alignments),
            Err(error) => error,
        };
        for (number, alignment) in numbered.iter() {
            if let Err(EngineError::InvalidAlignment(message)) = validate_alignments(std::slice::from_ref(alignment), line_counts) {
                return Err(EngineError::InvalidAlignment(message.replacen("alignment 1", &format!("alignment {number}"), 1)));
            }
        }
        for pair in numbered.windows(2) {
            let (previous, next) = (&pair[0], &pair[1]);
            if let Err(EngineError::InvalidAlignment(message)) = validate_alignments(&[previous.1.clone(), next.1.clone()], line_counts) {
                return Err(EngineError::InvalidAlignment(message.replacen(
                    "alignments 1 and 2",
                    &format!("alignments {} and {}", previous.0, next.0),
                    1,
                )));
            }
        }
        Err(error)
    }

    /// Every mark with its status, and whether applying would change the comparison.
    pub fn summary(&self, source_count: usize) -> Alignments {
        let complete = self.complete(source_count);
        let marks = self
            .marks
            .iter()
            .filter_map(|mark| {
                let document = Document::ALL.get(mark.document)?.clone();
                let status = match complete.iter().find(|(number, _)| *number == mark.number) {
                    None => MarkStatus::Unmatched,
                    Some((_, alignment)) if self.applied.contains(alignment) => MarkStatus::Applied,
                    Some(_) => MarkStatus::Pending,
                };
                Some(AlignmentMark {
                    document,
                    number: mark.number,
                    lines: mark.lines.clone(),
                    status,
                })
            })
            .collect();
        let wanted: Vec<ManualAlignment> = complete.into_iter().map(|(_, alignment)| alignment).collect();
        Alignments {
            marks,
            changed: wanted != self.applied,
        }
    }

    fn of(&self, document: usize) -> impl Iterator<Item = &Mark> {
        self.marks.iter().filter(move |mark| mark.document == document)
    }

    fn get(&self, document: usize, number: u32) -> Option<&Mark> {
        self.of(document).find(|mark| mark.number == number)
    }

    fn index_of(&self, document: &Document, number: u32) -> Result<usize, CoreError> {
        let index = document_index(document, usize::MAX)?;
        self.marks
            .iter()
            .position(|mark| mark.document == index && mark.number == number)
            .ok_or_else(|| invalid(format!("{} has no alignment {number}", document.letter())))
    }

    fn sort(&mut self) {
        self.marks.sort_by_key(|mark| (mark.document, mark.number));
    }

    /// The numbers marked in every one of `source_count` documents, with their alignments, in the order of their positions: compared document by document, each by where its part starts and ends. Alignments at the same positions everywhere keep the order of their numbers.
    fn complete(&self, source_count: usize) -> Vec<(u32, ManualAlignment)> {
        let mut numbers: Vec<u32> = self.marks.iter().map(|mark| mark.number.clone()).collect();
        numbers.sort_unstable();
        numbers.dedup();
        let mut complete: Vec<(u32, ManualAlignment)> = numbers
            .into_iter()
            .filter_map(|number| {
                let parts = (0..source_count)
                    .map(|document| self.get(document, number).map(|mark| mark.lines.clone()))
                    .collect::<Option<Vec<_>>>()?;
                Some((number, ManualAlignment { parts }))
            })
            .collect();
        complete.sort_by_key(|(_, alignment)| alignment.parts.iter().map(|part| (part.start.clone(), part.end.clone())).collect::<Vec<_>>());
        complete
    }
}

/// The index of a source document among `source_count`; D has none.
fn document_index(document: &Document, source_count: usize) -> Result<usize, CoreError> {
    match Document::ALL.iter().position(|candidate| candidate == document) {
        Some(index) if index < 3 && index < source_count => Ok(index),
        _ => Err(CoreError::NoSuchDocument { document: document.clone() }),
    }
}

/// Whether `new` can not be marked next to `existing`: marked ranges must not overlap and an anchor must not sit inside a range. Anchors may share a position, and ranges and anchors may touch.
fn collide(existing: &Range<usize>, new: &Range<usize>) -> bool {
    match (existing.is_empty(), new.is_empty()) {
        (true, true) => false,
        (true, false) => new.start < existing.start && existing.start < new.end,
        (false, true) => existing.start < new.start && new.start < existing.end,
        (false, false) => existing.start < new.end && new.start < existing.end,
    }
}

fn subject(lines: &Range<usize>) -> String {
    if lines.is_empty() {
        format!("an anchor before line {}", lines.start + 1)
    } else {
        format!("lines {} to {}", lines.start + 1, lines.end)
    }
}

fn invalid(reason: String) -> CoreError {
    CoreError::InvalidMark { reason }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COUNTS: [usize; 3] = [10, 10, 10];

    fn marks(entries: &[(Document, Range<usize>)]) -> AlignmentMarks {
        let mut marks = AlignmentMarks::default();
        for (document, lines) in entries {
            marks.mark(document.clone(), lines.clone(), &COUNTS).unwrap();
        }
        marks
    }

    fn numbers(marks: &AlignmentMarks, document: Document) -> Vec<(u32, Range<usize>)> {
        marks.summary(3).of(document).map(|mark| (mark.number, mark.lines.clone())).collect()
    }

    fn reason(result: Result<impl std::fmt::Debug, CoreError>) -> String {
        match result {
            Err(CoreError::InvalidMark { reason }) => reason,
            other => panic!("expected an invalid mark, got {other:?}"),
        }
    }

    #[test]
    fn numbers_are_the_lowest_free_one_of_each_document() {
        let mut marks = marks(&[(Document::A, 0..1), (Document::A, 2..3), (Document::B, 5..6)]);
        assert_eq!(numbers(&marks, Document::A), [(1, 0..1), (2, 2..3)]);
        assert_eq!(numbers(&marks, Document::B), [(1, 5..6)]);
        marks.remove(Document::A, 1).unwrap();
        assert_eq!(marks.mark(Document::A, 4..5, &COUNTS).unwrap(), 1);
        assert_eq!(numbers(&marks, Document::A), [(1, 4..5), (2, 2..3)]);
    }

    #[test]
    fn marks_must_fit_the_document() {
        let mut marks = AlignmentMarks::default();
        assert_eq!(reason(marks.mark(Document::A, 8..11, &COUNTS)), "lines 8..11 do not fit the 10 lines of A");
        assert!(marks.mark(Document::A, 8..10, &COUNTS).is_ok());
        assert!(marks.mark(Document::B, 10..10, &COUNTS).is_ok());
        assert_eq!(reason(marks.mark(Document::B, 11..11, &COUNTS)), "lines 11..11 do not fit the 10 lines of B");
    }

    #[test]
    fn only_source_documents_of_the_session_take_marks() {
        let mut marks = AlignmentMarks::default();
        assert!(matches!(marks.mark(Document::D, 0..1, &COUNTS), Err(CoreError::NoSuchDocument { .. })));
        assert!(matches!(marks.mark(Document::C, 0..1, &[10, 10]), Err(CoreError::NoSuchDocument { .. })));
    }

    #[test]
    fn ranges_must_not_overlap_and_anchors_must_not_sit_inside_one() {
        let mut marks = marks(&[(Document::A, 2..5)]);
        assert_eq!(reason(marks.mark(Document::A, 4..6, &COUNTS)), "alignment 1 of A is in the way of lines 5 to 6");
        assert_eq!(reason(marks.mark(Document::A, 3..4, &COUNTS)), "alignment 1 of A is in the way of lines 4 to 4");
        assert_eq!(reason(marks.mark(Document::A, 3..3, &COUNTS)), "alignment 1 of A is in the way of an anchor before line 4");
        assert_eq!(reason(marks.mark(Document::A, 0..8, &COUNTS)), "alignment 1 of A is in the way of lines 1 to 8");
        assert_eq!(numbers(&marks, Document::A).len(), 1);
    }

    #[test]
    fn ranges_and_anchors_may_touch_and_anchors_may_share_a_position() {
        let mut marks = marks(&[(Document::A, 2..5)]);
        assert!(marks.mark(Document::A, 5..7, &COUNTS).is_ok());
        assert!(marks.mark(Document::A, 0..2, &COUNTS).is_ok());
        assert!(marks.mark(Document::A, 2..2, &COUNTS).is_ok());
        assert!(marks.mark(Document::A, 5..5, &COUNTS).is_ok());
        assert!(marks.mark(Document::A, 5..5, &COUNTS).is_ok());
        assert_eq!(numbers(&marks, Document::A).len(), 6);
    }

    #[test]
    fn removing_an_unknown_mark_fails() {
        let mut marks = marks(&[(Document::A, 0..1)]);
        assert_eq!(reason(marks.remove(Document::A, 2)), "A has no alignment 2");
        assert_eq!(reason(marks.remove(Document::B, 1)), "B has no alignment 1");
        assert!(marks.remove(Document::A, 1).is_ok());
        assert!(numbers(&marks, Document::A).is_empty());
    }

    #[test]
    fn renumbering_moves_a_mark_or_swaps_it_with_the_one_that_has_the_number() {
        let mut marks = marks(&[(Document::A, 0..1), (Document::A, 2..3), (Document::A, 4..5), (Document::B, 6..7)]);
        marks.renumber(Document::A, 1, 7).unwrap();
        assert_eq!(numbers(&marks, Document::A), [(2, 2..3), (3, 4..5), (7, 0..1)]);
        marks.renumber(Document::A, 7, 3).unwrap();
        assert_eq!(numbers(&marks, Document::A), [(2, 2..3), (3, 0..1), (7, 4..5)]);
        marks.renumber(Document::A, 2, 2).unwrap();
        assert_eq!(numbers(&marks, Document::B), [(1, 6..7)]);
    }

    #[test]
    fn renumbering_rejects_zero_and_unknown_marks() {
        let mut marks = marks(&[(Document::A, 0..1)]);
        assert_eq!(reason(marks.renumber(Document::A, 1, 0)), "alignment numbers start at 1");
        assert_eq!(reason(marks.renumber(Document::A, 4, 2)), "A has no alignment 4");
    }

    #[test]
    fn only_numbers_marked_everywhere_give_alignments() {
        let marks = marks(&[(Document::A, 0..1), (Document::B, 2..3), (Document::A, 4..5), (Document::B, 6..7), (Document::C, 1..2)]);
        let alignments = marks.to_apply(&COUNTS).unwrap();
        assert_eq!(alignments, [ManualAlignment { parts: vec![0..1, 2..3, 1..2] }]);
    }

    #[test]
    fn two_way_sessions_need_two_documents() {
        let marks = marks(&[(Document::A, 0..1), (Document::B, 2..3)]);
        assert_eq!(marks.to_apply(&[10, 10]).unwrap(), [ManualAlignment { parts: vec![0..1, 2..3] }]);
        assert_eq!(marks.to_apply(&COUNTS).unwrap(), []);
    }

    #[test]
    fn alignments_follow_the_positions_not_the_numbers() {
        let mut marks = marks(&[
            (Document::A, 5..6),
            (Document::A, 1..2),
            (Document::B, 6..7),
            (Document::B, 2..3),
            (Document::C, 4..5),
            (Document::C, 0..1),
        ]);
        let alignments = marks.to_apply(&COUNTS).unwrap();
        assert_eq!(
            alignments,
            [ManualAlignment { parts: vec![1..2, 2..3, 0..1] }, ManualAlignment { parts: vec![5..6, 6..7, 4..5] }]
        );
        marks.set_applied(alignments);
        assert!(!marks.summary(3).changed);
    }

    #[test]
    fn anchors_at_one_position_are_ordered_by_the_other_documents() {
        let marks = marks(&[
            (Document::A, 1..1),
            (Document::A, 1..1),
            (Document::B, 2..3),
            (Document::B, 1..2),
            (Document::C, 2..3),
            (Document::C, 1..2),
        ]);
        let alignments = marks.to_apply(&COUNTS).unwrap();
        assert_eq!(
            alignments,
            [ManualAlignment { parts: vec![1..1, 1..2, 1..2] }, ManualAlignment { parts: vec![1..1, 2..3, 2..3] }]
        );
    }

    #[test]
    fn anchors_at_one_position_keep_it() {
        let marks = marks(&[
            (Document::A, 1..1),
            (Document::A, 1..1),
            (Document::B, 1..2),
            (Document::B, 2..2),
            (Document::C, 1..1),
            (Document::C, 1..2),
        ]);
        let alignments = marks.to_apply(&COUNTS).unwrap();
        assert_eq!(
            alignments,
            [ManualAlignment { parts: vec![1..1, 1..2, 1..1] }, ManualAlignment { parts: vec![1..1, 2..2, 1..2] }]
        );
    }

    #[test]
    fn crossing_names_the_users_numbers() {
        let mut marks = marks(&[
            (Document::A, 0..1),
            (Document::A, 9..10),
            (Document::A, 4..5),
            (Document::B, 4..5),
            (Document::B, 9..10),
            (Document::B, 0..1),
            (Document::C, 0..1),
            (Document::C, 1..2),
            (Document::C, 2..3),
        ]);
        marks.remove(Document::A, 2).unwrap();
        marks.remove(Document::B, 2).unwrap();
        marks.remove(Document::C, 2).unwrap();
        marks.renumber(Document::A, 3, 5).unwrap();
        marks.renumber(Document::B, 3, 5).unwrap();
        marks.renumber(Document::C, 3, 5).unwrap();
        let Err(EngineError::InvalidAlignment(message)) = marks.to_apply(&COUNTS) else {
            panic!("expected an invalid alignment")
        };
        assert_eq!(message, "alignments 1 and 5 cross in B");
    }

    #[test]
    fn an_alignment_of_anchors_only_names_its_number() {
        let mut marks = marks(&[(Document::A, 0..1), (Document::B, 0..1), (Document::A, 3..3), (Document::B, 3..3)]);
        marks.mark(Document::C, 0..1, &COUNTS).unwrap();
        marks.mark(Document::C, 3..3, &COUNTS).unwrap();
        let Err(EngineError::InvalidAlignment(message)) = marks.to_apply(&COUNTS) else {
            panic!("expected an invalid alignment")
        };
        assert_eq!(message, "alignment 2 has only anchors");
    }

    #[test]
    fn statuses_follow_the_applied_alignments() {
        let mut marks = marks(&[(Document::A, 0..1), (Document::B, 2..3), (Document::C, 1..2), (Document::A, 5..6)]);
        let statuses = |marks: &AlignmentMarks| -> Vec<MarkStatus> { marks.summary(3).marks.iter().map(|mark| mark.status).collect() };
        assert_eq!(statuses(&marks), [MarkStatus::Pending, MarkStatus::Unmatched, MarkStatus::Pending, MarkStatus::Pending]);
        assert!(marks.summary(3).changed);
        marks.set_applied(marks.to_apply(&COUNTS).unwrap());
        assert_eq!(statuses(&marks), [MarkStatus::Applied, MarkStatus::Unmatched, MarkStatus::Applied, MarkStatus::Applied]);
        assert!(!marks.summary(3).changed);
    }

    #[test]
    fn changing_a_mark_after_applying_makes_it_pending() {
        let mut marks = marks(&[(Document::A, 0..1), (Document::B, 2..3), (Document::C, 1..2)]);
        marks.set_applied(marks.to_apply(&COUNTS).unwrap());
        marks.remove(Document::B, 1).unwrap();
        let summary = marks.summary(3);
        assert!(summary.changed);
        assert!(summary.marks.iter().all(|mark| mark.status == MarkStatus::Unmatched));
        marks.mark(Document::B, 3..4, &COUNTS).unwrap();
        let summary = marks.summary(3);
        assert!(summary.changed);
        assert!(summary.marks.iter().all(|mark| mark.status == MarkStatus::Pending));
    }

    #[test]
    fn clearing_the_marks_leaves_the_applied_alignments_and_counts_as_a_change() {
        let mut marks = marks(&[(Document::A, 0..1), (Document::B, 2..3), (Document::C, 1..2)]);
        marks.set_applied(marks.to_apply(&COUNTS).unwrap());
        marks.clear();
        assert_eq!(marks.applied().len(), 1);
        assert!(marks.summary(3).marks.is_empty());
        assert!(marks.summary(3).changed);
        marks.set_applied(marks.to_apply(&COUNTS).unwrap());
        assert!(!marks.summary(3).changed);
    }

    #[test]
    fn marks_that_give_no_alignment_change_nothing() {
        let marks = marks(&[(Document::A, 0..1), (Document::B, 2..3)]);
        assert!(!marks.summary(3).changed);
    }

    #[test]
    fn reset_forgets_marks_and_applied_alignments() {
        let mut marks = marks(&[(Document::A, 0..1), (Document::B, 2..3), (Document::C, 1..2)]);
        marks.set_applied(marks.to_apply(&COUNTS).unwrap());
        marks.reset();
        assert!(marks.applied().is_empty());
        assert!(marks.summary(3).marks.is_empty());
        assert!(!marks.summary(3).changed);
    }
}
