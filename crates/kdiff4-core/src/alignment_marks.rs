use std::ops::Range;

use kdiff4_protocol::engine::{EngineError, ManualAlignment, document_order, moved_parts, validate_alignments};
use kdiff4_protocol::{AlignmentMark, Alignments, Document, InsetChoice, InsetOption, MarkStatus};

use crate::error::CoreError;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Mark {
    /// Index into the session's source documents.
    document: usize,
    number: u32,
    /// An empty range is an anchor: the position before that line.
    lines: Range<usize>,
}

/// A complete alignment at its place in the display order.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Placed {
    number: u32,
    alignment: ManualAlignment,
}


/// Where an alignment could be shown among the others: the documents that don't keep their order there move their hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate {
    slot: usize,
    moved: Vec<bool>,
    /// How many alignments it would be shown on the wrong side of by their numbers. Of several places that move the same documents, the least disordered one is the candidate.
    disorder: usize,
}


/// The marks the user made, and the alignments the comparison was last computed with. A number marked in every source document is an alignment; the others wait. Marking never touches the comparison: only applying does.
#[derive(Debug, Clone, Default)]
pub struct AlignmentMarks {
    /// Ordered by document, then number. At most one mark per document and number.
    marks: Vec<Mark>,
    /// The number of source documents the marks were last changed for.
    documents: usize,
    /// The complete alignments in the order they are shown. A place the user chose stays until the marks of its alignment change.
    placed: Vec<Placed>,
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
        self.documents = line_counts.len();
        self.settle();
        Ok(number)
    }

    pub fn remove(&mut self, document: Document, number: u32) -> Result<(), CoreError> {
        let index = self.index_of(&document, number)?;
        self.marks.remove(index);
        self.settle();
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
        self.settle();
        Ok(())
    }

    /// Shows the alignment `number` at `slot` among the other alignments, counting from 0 at the top. The documents whose order disagrees with that place move its hunk.
    pub fn place(&mut self, number: u32, slot: usize) -> Result<(), CoreError> {
        let mut list = self.arranged(self.documents);
        let Some(index) = list.iter().position(|placed| placed.number == number) else {
            return Err(invalid(format!("alignment {number} is not marked in every document")));
        };
        let placed = list.remove(index);
        if slot > list.len() {
            return Err(invalid(format!("there is no place {slot} among the {} other alignments", list.len())));
        }
        let moved = moved_at(&list, &placed.alignment.parts, slot);
        if moved.iter().all(|moved| *moved) {
            return Err(invalid(format!("no document keeps its order when alignment {number} is shown there")));
        }
        list.insert(slot, placed);
        self.placed = list;
        Ok(())
    }

    /// Whether some document has the alignment `number` in another place than the others, so the user has a place to choose.
    pub fn is_inset(&self, number: u32, source_count: usize) -> bool {
        let placed = self.arranged(source_count);
        let orders = document_orders(&placed, source_count);
        placed.iter().position(|placed| placed.number == number).is_some_and(|index| disagrees(&orders, index))
    }

    /// Removes every mark. The applied alignments stay, since the comparison still has them.
    pub fn clear(&mut self) {
        self.marks.clear();
        self.settle();
    }

    /// Removes the marks and the applied alignments, for a session whose source lines changed.
    pub fn reset(&mut self) {
        self.marks.clear();
        self.placed.clear();
        self.applied.clear();
    }

    pub fn applied(&self) -> &[ManualAlignment] {
        &self.applied
    }

    pub fn set_applied(&mut self, alignments: Vec<ManualAlignment>) {
        self.applied = alignments;
    }

    /// The alignments the marks give: every number marked in all source documents, in the order they are shown. Checked like the engine checks them, but errors name the user's numbers instead of list positions.
    pub fn to_apply(&self, line_counts: &[usize]) -> Result<Vec<ManualAlignment>, EngineError> {
        let numbered: Vec<(u32, ManualAlignment)> = self.arranged(line_counts.len()).into_iter().map(|placed| (placed.number, placed.alignment)).collect();
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

    /// Every mark with its status, the alignments that are insets with the places they could be shown at, and whether applying would change the comparison.
    pub fn summary(&self, source_count: usize) -> Alignments {
        let placed = self.arranged(source_count);
        let wanted: Vec<ManualAlignment> = placed.iter().map(|placed| placed.alignment.clone()).collect();
        let moved_wanted: Vec<Vec<Option<usize>>> = (0..source_count).map(|document| moved_parts(&wanted, document)).collect();
        let moved_applied: Vec<Vec<Option<usize>>> = (0..source_count).map(|document| moved_parts(&self.applied, document)).collect();
        let marks = self
            .marks
            .iter()
            .filter_map(|mark| {
                let document = Document::ALL.get(mark.document)?.clone();
                let entry = placed.iter().find(|placed| placed.number == mark.number);
                let status = match entry {
                    None => MarkStatus::Unmatched,
                    Some(entry) if self.applied.contains(&entry.alignment) => MarkStatus::Applied,
                    Some(_) => MarkStatus::Pending,
                };
                let replaced_by = entry.and_then(|entry| {
                    let (alignments, moved) = match status {
                        MarkStatus::Applied => (&self.applied, &moved_applied),
                        _ => (&wanted, &moved_wanted),
                    };
                    let index = alignments.iter().position(|alignment| *alignment == entry.alignment)?;
                    let occupant = alignments.get(moved.get(mark.document)?.get(index).cloned()??)?;
                    placed.iter().find(|placed| placed.alignment == *occupant).map(|placed| placed.number)
                });
                Some(AlignmentMark {
                    document,
                    number: mark.number,
                    lines: mark.lines.clone(),
                    status,
                    replaced_by,
                })
            })
            .collect();
        let orders = document_orders(&placed, source_count);
        let mut insets: Vec<InsetChoice> = (0..placed.len()).filter(|index| disagrees(&orders, *index)).map(|index| inset_choice(&placed, &placed[index])).collect();
        insets.sort_by_key(|choice| choice.number);
        Alignments {
            marks,
            changed: wanted != self.applied,
            insets,
            ask: None,
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

    /// Keeps the places in step with the marks.
    fn settle(&mut self) {
        self.placed = self.arranged(self.documents);
    }

    /// The numbers marked in every one of `source_count` documents, with their parts, ordered by number.
    fn complete(&self, source_count: usize) -> Vec<(u32, Vec<Range<usize>>)> {
        let mut numbers: Vec<u32> = self.marks.iter().map(|mark| mark.number.clone()).collect();
        numbers.sort_unstable();
        numbers.dedup();
        numbers
            .into_iter()
            .filter_map(|number| {
                let parts = (0..source_count)
                    .map(|document| self.get(document, number).map(|mark| mark.lines.clone()))
                    .collect::<Option<Vec<_>>>()?;
                Some((number, parts))
            })
            .collect()
    }

    /// Every complete alignment of `source_count` documents in the order they are shown. Alignments keep the place they have as long as their marks are the same. A new one is shown where every document agrees with the order, among alignments with the same positions by number. Where the documents disagree it is shown where the fewest documents keep their order, which is the user's likely intent: the document that differs from the others is the one that moved the code on purpose. A tie goes to the first document.
    fn arranged(&self, source_count: usize) -> Vec<Placed> {
        let complete = self.complete(source_count);
        let kept = |placed: &&Placed| complete.iter().any(|(number, parts)| *number == placed.number && *parts == placed.alignment.parts);
        let mut list: Vec<Placed> = self.placed.iter().filter(kept).cloned().collect();
        for (number, parts) in complete.iter() {
            if list.iter().any(|placed| placed.number == *number) {
                continue;
            }
            let candidates = candidates(&list, *number, parts);
            let slot = preferred(&candidates).map_or(list.len(), |index| candidates[index].slot);
            list.insert(
                slot,
                Placed {
                    number: *number,
                    alignment: ManualAlignment::new(parts.clone()),
                },
            );
        }
        list
    }
}


/// The order of the parts of each of `source_count` documents, for the alignments in the order they are shown.
fn document_orders(placed: &[Placed], source_count: usize) -> Vec<Vec<usize>> {
    let alignments: Vec<ManualAlignment> = placed.iter().map(|placed| placed.alignment.clone()).collect();
    (0..source_count).map(|document| document_order(&alignments, document)).collect()
}


/// Whether the alignment at `index` of the list is not at the place in some document's order that it has in the list.
fn disagrees(orders: &[Vec<usize>], index: usize) -> bool {
    orders.iter().any(|order| order.get(index).is_some_and(|alignment| *alignment != index))
}


/// For each document, whether alignment parts `parts` would be in another place of the document than their place in the list when shown at `slot` among `others`.
fn moved_at(others: &[Placed], parts: &[Range<usize>], slot: usize) -> Vec<bool> {
    let mut alignments: Vec<ManualAlignment> = others.iter().map(|other| other.alignment.clone()).collect();
    alignments.insert(slot, ManualAlignment::new(parts.to_vec()));
    (0..parts.len()).map(|document| document_order(&alignments, document)[slot] != slot).collect()
}


/// The places the alignment `number` with `parts` could be shown at among `others`, one per set of documents that would move it, in the order of their slots. A place where every document moves it is never one.
fn candidates(others: &[Placed], number: u32, parts: &[Range<usize>]) -> Vec<Candidate> {
    let mut candidates: Vec<Candidate> = Vec::new();
    for slot in 0..=others.len() {
        let moved = moved_at(others, parts, slot);
        if moved.iter().all(|moved| *moved) {
            continue;
        }
        let disorder = others[..slot].iter().filter(|other| other.number > number).count() + others[slot..].iter().filter(|other| other.number < number).count();
        match candidates.iter_mut().find(|candidate| candidate.moved == moved) {
            Some(candidate) if disorder < candidate.disorder => {
                candidate.slot = slot;
                candidate.disorder = disorder;
            }
            Some(_) => {}
            None => candidates.push(Candidate { slot, moved, disorder }),
        }
    }
    candidates
}


/// The index of the candidate to show an alignment at when the user has not chosen: the one that moves nothing, else the one the fewest documents keep their order at, the first documents winning ties.
fn preferred(candidates: &[Candidate]) -> Option<usize> {
    let key = |candidate: &Candidate| {
        let natural: Vec<usize> = (0..candidate.moved.len()).filter(|document| !candidate.moved[*document]).collect();
        (natural.len() < candidate.moved.len(), natural.len(), natural)
    };
    (0..candidates.len()).min_by_key(|index| key(&candidates[*index]))
}


/// The places `entry` could be shown at, and the one it is shown at, for the summary. `placed` is the whole list.
fn inset_choice(placed: &[Placed], entry: &Placed) -> InsetChoice {
    let others: Vec<Placed> = placed.iter().filter(|other| other.number != entry.number).cloned().collect();
    let options = candidates(&others, entry.number, &entry.alignment.parts);
    let slot = placed.iter().position(|other| other.number == entry.number).unwrap_or(0);
    let moved_now = moved_at(&others, &entry.alignment.parts, slot);
    let current = options.iter().position(|option| option.moved == moved_now).unwrap_or(0);
    let options = options
        .iter()
        .map(|option| InsetOption {
            slot: option.slot,
            above: option.slot.checked_sub(1).and_then(|index| others.get(index)).map(|other| other.number),
            below: others.get(option.slot).map(|other| other.number),
            moved: option.moved.iter().enumerate().filter(|(_, moved)| **moved).filter_map(|(document, _)| Document::ALL.get(document).cloned()).collect(),
        })
        .collect();
    InsetChoice {
        number: entry.number,
        current,
        options,
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
        assert_eq!(alignments, [ManualAlignment::new(vec![0..1, 2..3, 1..2])]);
    }

    #[test]
    fn two_way_sessions_need_two_documents() {
        let marks = marks(&[(Document::A, 0..1), (Document::B, 2..3)]);
        assert_eq!(marks.to_apply(&[10, 10]).unwrap(), [ManualAlignment::new(vec![0..1, 2..3])]);
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
            [ManualAlignment::new(vec![1..2, 2..3, 0..1]), ManualAlignment::new(vec![5..6, 6..7, 4..5])]
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
            [ManualAlignment::new(vec![1..1, 1..2, 1..2]), ManualAlignment::new(vec![1..1, 2..3, 2..3])]
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
            [ManualAlignment::new(vec![1..1, 1..2, 1..1]), ManualAlignment::new(vec![1..1, 2..2, 1..2])]
        );
    }

    // Alignment 1 is the first function and 2 the second; B has them in the other order.
    fn functions_with_b_reversed() -> AlignmentMarks {
        marks(&[
            (Document::A, 0..1),
            (Document::B, 4..5),
            (Document::C, 0..1),
            (Document::A, 2..3),
            (Document::B, 1..2),
            (Document::C, 2..3),
        ])
    }

    /// The documents that don't have the alignment at `index` of the list at the place it has in the list.
    fn displaced_in(alignments: &[ManualAlignment], index: usize) -> Vec<usize> {
        (0..alignments[0].parts.len()).filter(|document| document_order(alignments, *document)[index] != index).collect()
    }

    #[test]
    fn a_document_that_differs_in_order_keeps_its_order_and_the_others_move() {
        let marks = functions_with_b_reversed();
        let alignments = marks.to_apply(&COUNTS).unwrap();
        assert_eq!(alignments.len(), 2);
        assert_eq!(alignments[0].parts, [2..3, 1..2, 2..3]);
        assert_eq!(alignments[1].parts, [0..1, 4..5, 0..1]);
        assert_eq!(displaced_in(&alignments, 0), [0, 2]);
        assert_eq!(displaced_in(&alignments, 1), [0, 2]);
        assert_eq!(moved_parts(&alignments, 0), [None, Some(0)]);
        assert_eq!(moved_parts(&alignments, 1), [None, None]);
    }

    #[test]
    fn two_way_insets_keep_the_order_of_the_first_document() {
        let marks = marks(&[(Document::A, 0..1), (Document::B, 4..5), (Document::A, 2..3), (Document::B, 1..2)]);
        let alignments = marks.to_apply(&[10, 10]).unwrap();
        assert_eq!(alignments[0].parts, [0..1, 4..5]);
        assert_eq!(alignments[1].parts, [2..3, 1..2]);
        assert_eq!(displaced_in(&alignments, 1), [1]);
    }

    #[test]
    fn the_summary_lists_where_an_inset_could_be_shown() {
        let summary = functions_with_b_reversed().summary(3);
        let moved: Vec<(Document, u32, Option<u32>)> = summary.marks.iter().filter(|mark| mark.is_moved()).map(|mark| (mark.document.clone(), mark.number, mark.replaced_by)).collect();
        assert_eq!(moved, [(Document::A, 1, Some(2)), (Document::C, 1, Some(2))]);
        let option = |slot, above, below, moved: &[Document]| InsetOption {
            slot,
            above,
            below,
            moved: moved.to_vec(),
        };
        let first = InsetChoice {
            number: 1,
            current: 1,
            options: vec![option(0, None, Some(2), &[Document::B]), option(1, Some(2), None, &[Document::A, Document::C])],
        };
        let second = InsetChoice {
            number: 2,
            current: 0,
            options: vec![option(0, None, Some(1), &[Document::A, Document::C]), option(1, Some(1), None, &[Document::B])],
        };
        assert_eq!(summary.insets, [first, second]);
        assert_eq!(summary.ask, None);
    }

    #[test]
    fn applied_marks_keep_naming_the_hunk_that_took_their_place_until_the_new_place_is_applied() {
        let mut marks = functions_with_b_reversed();
        marks.set_applied(marks.to_apply(&COUNTS).unwrap());
        let replaced = |marks: &AlignmentMarks| -> Vec<(Document, u32, Option<u32>)> {
            marks.summary(3).marks.iter().filter(|mark| mark.is_moved()).map(|mark| (mark.document.clone(), mark.number, mark.replaced_by)).collect()
        };
        marks.place(2, 1).unwrap();
        assert!(marks.summary(3).changed);
        assert_eq!(replaced(&marks), [(Document::A, 1, Some(2)), (Document::C, 1, Some(2))]);
        marks.set_applied(marks.to_apply(&COUNTS).unwrap());
        assert_eq!(replaced(&marks), [(Document::B, 2, Some(1))]);
    }

    #[test]
    fn placing_an_alignment_elsewhere_moves_the_other_documents() {
        let mut marks = functions_with_b_reversed();
        marks.place(2, 1).unwrap();
        let alignments = marks.to_apply(&COUNTS).unwrap();
        assert_eq!(alignments[0].parts, [0..1, 4..5, 0..1]);
        assert_eq!(alignments[1].parts, [2..3, 1..2, 2..3]);
        assert_eq!(displaced_in(&alignments, 1), [1]);
        assert_eq!(marks.summary(3).insets[1].current, 1);
        marks.place(2, 0).unwrap();
        assert_eq!(displaced_in(&marks.to_apply(&COUNTS).unwrap(), 0), [0, 2]);
    }

    #[test]
    fn a_chosen_place_stays_when_other_marks_change() {
        let mut marks = functions_with_b_reversed();
        marks.place(2, 1).unwrap();
        marks.mark(Document::A, 6..7, &COUNTS).unwrap();
        marks.mark(Document::B, 7..8, &COUNTS).unwrap();
        marks.mark(Document::C, 6..7, &COUNTS).unwrap();
        let alignments = marks.to_apply(&COUNTS).unwrap();
        assert_eq!(alignments.iter().map(|alignment| alignment.parts[0].clone()).collect::<Vec<_>>(), [0..1, 2..3, 6..7]);
        assert_eq!(displaced_in(&alignments, 1), [1]);
        marks.remove(Document::B, 3).unwrap();
        assert_eq!(marks.to_apply(&COUNTS).unwrap().len(), 2);
    }

    #[test]
    fn an_alignment_whose_marks_change_is_placed_again() {
        let mut marks = functions_with_b_reversed();
        marks.place(2, 1).unwrap();
        marks.remove(Document::B, 2).unwrap();
        marks.mark(Document::B, 1..2, &COUNTS).unwrap();
        assert_eq!(displaced_in(&marks.to_apply(&COUNTS).unwrap(), 0), [0, 2]);
    }

    #[test]
    fn insets_can_be_shown_between_other_alignments() {
        let mut marks = marks(&[
            (Document::A, 0..1),
            (Document::B, 0..1),
            (Document::C, 0..1),
            (Document::A, 2..3),
            (Document::B, 2..3),
            (Document::C, 2..3),
            (Document::A, 4..5),
            (Document::B, 4..5),
            (Document::C, 4..5),
        ]);
        marks.mark(Document::A, 6..7, &COUNTS).unwrap();
        marks.mark(Document::B, 3..4, &COUNTS).unwrap();
        marks.mark(Document::C, 6..7, &COUNTS).unwrap();
        let summary = marks.summary(3);
        let choice = summary.insets.iter().find(|choice| choice.number == 4).unwrap();
        assert_eq!(choice.options.iter().map(|option| (option.slot, option.above, option.below)).collect::<Vec<_>>(), [(2, Some(2), Some(3)), (3, Some(3), None)]);
        assert_eq!(choice.current, 0);
        marks.place(4, 3).unwrap();
        let summary = marks.summary(3);
        assert_eq!(summary.insets.iter().find(|choice| choice.number == 4).unwrap().current, 1);
    }

    #[test]
    fn placing_rejects_unknown_alignments_and_places() {
        let mut marks = functions_with_b_reversed();
        marks.mark(Document::A, 8..9, &COUNTS).unwrap();
        assert_eq!(reason(marks.place(3, 0)), "alignment 3 is not marked in every document");
        assert_eq!(reason(marks.place(9, 0)), "alignment 9 is not marked in every document");
        assert_eq!(reason(marks.place(2, 2)), "there is no place 2 among the 1 other alignments");
    }

    #[test]
    fn only_alignments_that_disagree_are_insets() {
        let marks = marks(&[(Document::A, 0..1), (Document::B, 0..1), (Document::A, 2..3), (Document::B, 2..3)]);
        assert!(!marks.is_inset(1, 2) && !marks.is_inset(2, 2));
        assert!(marks.summary(2).insets.is_empty());
        let reversed = functions_with_b_reversed();
        assert!(reversed.is_inset(2, 3) && reversed.is_inset(1, 3));
        assert!(!reversed.is_inset(3, 3));
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
