use std::ops::Range;

use kdiff4_protocol::engine::{ComparisonEngine, HunkChange, MergeStatus, Region, RegionStatus, Row, SourceLines};
use kdiff4_protocol::{Choice, Document, MergeUpdate, RegionMerge};

use crate::error::CoreError;
use crate::normalizer;
use crate::undo::StepData;

/// A region's merge state, with the size of its text in D.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionState {
    pub merge: RegionMerge,
    pub text_len: usize,
}

/// What one merge op did to the regions: `before`, at index `at`, were replaced by `after`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeStep {
    pub at: usize,
    pub before: Vec<(Region, RegionState)>,
    pub after: Vec<(Region, RegionState)>,
    /// Whether D's final line break is explicit, before and after a whole-source choice.
    pub explicit_final_eol: Option<(bool, bool)>,
}

impl StepData for MergeStep {
    /// Typing joins while it stays inside one region.
    fn joined(&self, next: &MergeStep) -> Option<MergeStep> {
        let same_region = next.at == self.at && self.after.len() == 1 && next.before.len() == 1 && next.after.len() == 1;
        (same_region && self.explicit_final_eol.is_none() && next.explicit_final_eol.is_none()).then(|| MergeStep {
            at: self.at.clone(),
            before: self.before.clone(),
            after: next.after.clone(),
            explicit_final_eol: None,
        })
    }
}

/// The regions of a session and, when it has D, their merge states. D's text is the concatenation of the regions' texts, in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Merge {
    regions: Vec<Region>,
    /// One per region for a session with D; empty otherwise.
    states: Vec<RegionState>,
}

impl Merge {
    pub fn without_output(regions: Vec<Region>) -> Merge {
        Merge { regions, states: Vec::new() }
    }

    /// Auto-resolves every region and builds D from the result.
    pub fn with_output(regions: Vec<Region>, sources: &[Vec<&str>]) -> (Merge, String) {
        let mut text = String::new();
        let mut states = Vec::with_capacity(regions.len());
        for region in &regions {
            let choice = auto_choice(&region.status);
            let rendered = render(region, &choice, sources);
            states.push(RegionState {
                merge: RegionMerge {
                    choice,
                    edited: false,
                    output_lines: line_count(&rendered),
                },
                text_len: rendered.len(),
            });
            text.push_str(&rendered);
        }
        (Merge { regions, states }, text)
    }

    pub fn regions(&self) -> &[Region] {
        &self.regions
    }

    pub fn states(&self) -> &[RegionState] {
        &self.states
    }

    pub fn region_merges(&self) -> Vec<RegionMerge> {
        self.states.iter().map(|state| state.merge.clone()).collect()
    }

    /// Whether D differs from what auto-resolve builds: a region was edited by hand, or its choice is not the one auto-resolve makes for its status.
    pub fn has_user_work(&self) -> bool {
        self.regions
            .iter()
            .zip(&self.states)
            .any(|(region, state)| state.merge.edited || state.merge.choice != auto_choice(&region.status))
    }

    pub fn unresolved_count(&self) -> usize {
        self.states.iter().filter(|state| state.merge.is_unresolved()).count()
    }

    /// Byte offset in D where each region's text starts, and one more for the end of D.
    pub fn offsets(&self) -> Vec<usize> {
        let mut offsets = Vec::with_capacity(self.states.len() + 1);
        let mut offset = 0;
        offsets.push(0);
        for state in &self.states {
            offset += state.text_len;
            offsets.push(offset.clone());
        }
        offsets
    }

    pub fn entries(&self, range: Range<usize>) -> Vec<(Region, RegionState)> {
        self.regions[range.clone()].iter().cloned().zip(self.states[range].iter().cloned()).collect()
    }

    /// Replaces `removed` regions at `at` by `entries`.
    pub fn splice(&mut self, at: usize, removed: usize, entries: Vec<(Region, RegionState)>) {
        let (regions, states): (Vec<Region>, Vec<RegionState>) = entries.into_iter().unzip();
        self.regions.splice(at..at + removed, regions);
        self.states.splice(at..at + removed, states);
    }

    /// The regions a hand edit replacing `range` of D's `text` with `inserted` touches. The run is widened to the right until its new text ends with a line break, or no text follows it, so every region keeps whole lines.
    pub fn touched(&self, range: Range<usize>, inserted: &str, text: &str) -> Range<usize> {
        let count = self.states.len();
        if count == 0 {
            return 0..0;
        }
        let offsets = self.offsets();
        let has_text = |index: usize| offsets[index] < offsets[index + 1];
        let total = offsets[count];
        let first = if range.start < total {
            (0..count).find(|&index| has_text(index) && offsets[index] <= range.start && range.start < offsets[index + 1])
        } else {
            (0..count).rev().find(|&index| has_text(index))
        };
        let first = first.unwrap_or(count - 1);
        let mut last = (first..count).filter(|&index| has_text(index) && offsets[index] < range.end).last().unwrap_or(first);
        // The byte at `position` of D once the edit is made.
        let new_byte = |position: usize| {
            if position < range.start {
                text.as_bytes().get(position).cloned()
            } else if position < range.start + inserted.len() {
                inserted.as_bytes().get(position - range.start).cloned()
            } else {
                text.as_bytes().get(position - inserted.len() + range.len()).cloned()
            }
        };
        while last + 1 < count && offsets[last + 1] < total {
            let start = offsets[first];
            let end = offsets[last + 1] + inserted.len() - range.len();
            if end == start || new_byte(end - 1) == Some(b'\n') {
                break;
            }
            last += 1;
        }
        first..last + 1
    }

    /// Region `region` as the only one an edit touches, when it has no text and the edit inserts whole lines at its place; `None` otherwise.
    pub fn empty_target(&self, region: usize, range: Range<usize>, inserted: &str) -> Option<Range<usize>> {
        let offsets = self.offsets();
        let start = offsets.get(region).cloned()?;
        let end = offsets.get(region + 1).cloned()?;
        let fits = start == end && range == (start..start) && inserted.ends_with('\n');
        fits.then_some(region..region + 1)
    }

    pub fn update(&self, at: usize, removed: usize, added: usize) -> MergeUpdate {
        let range = at..at + added;
        MergeUpdate {
            at,
            removed,
            regions: self.regions[range.clone()].to_vec(),
            merge: self.states[range].iter().map(|state| state.merge.clone()).collect(),
        }
    }
}

/// A hand edit replacing `range` of `text`, the D text of an unedited region whose hunks are equal, with `inserted`. The region is split so each line the edit touches becomes an edited region of its own, spanning that line in every source; new lines beyond the touched ones get regions with no lines in the sources, and touched lines left without new text get regions with no text in D; the lines before and after stay as they were. `None` when the region doesn't qualify.
pub fn split_stable(region: &Region, state: &RegionState, text: &str, range: Range<usize>, inserted: &str) -> Option<Vec<(Region, RegionState)>> {
    let count = line_count(text);
    let qualifies = !is_difference(&region.status) && !state.merge.edited && region.rows.len() == count && region.hunks.iter().all(|hunk| hunk.len() == count);
    if !qualifies || range.start > range.end || range.end > text.len() {
        return None;
    }
    let first = text[..range.start].bytes().filter(|byte| *byte == b'\n').count();
    let line_start = text[..range.start].rfind('\n').map_or(0, |position| position + 1);
    let mut end = range.end;
    let mut middle = format!("{}{}{}", &text[line_start..range.start], inserted, &text[range.end..end]);
    let at_line_start = |position: usize| position == 0 || text.as_bytes()[position - 1] == b'\n';
    while end < text.len() && (!at_line_start(end) || (!middle.is_empty() && !middle.ends_with('\n'))) {
        end = text[end..].find('\n').map_or(text.len(), |position| end + position + 1);
        middle = format!("{}{}{}", &text[line_start..range.start], inserted, &text[range.end..end]);
    }
    let last = line_count(&text[..end]).max(first);
    let piece = |lines: Range<usize>, merge: RegionMerge, text_len: usize| {
        let hunks = region.hunks.iter().map(|hunk| hunk.start + lines.start..hunk.start + lines.end).collect();
        (
            Region {
                hunks,
                status: region.status.clone(),
                rows: region.rows[lines].to_vec(),
            },
            RegionState { merge, text_len },
        )
    };
    let unedited = |lines: usize| RegionMerge {
        choice: state.merge.choice.clone(),
        edited: false,
        output_lines: lines,
    };
    let mut pieces = Vec::with_capacity(3);
    if first > 0 {
        pieces.push(piece(0..first, unedited(first), line_start));
    }
    let new_lines: Vec<&str> = middle.split_inclusive('\n').collect();
    for index in 0..new_lines.len().max(last - first) {
        let line = new_lines.get(index).cloned().unwrap_or("");
        let start = (first + index).min(last);
        let end = if index < last - first { start + 1 } else { start };
        pieces.push(piece(
            start..end,
            RegionMerge {
                choice: state.merge.choice.clone(),
                edited: true,
                output_lines: line_count(line),
            },
            line.len(),
        ));
    }
    if last < count {
        pieces.push(piece(last..count, unedited(count - last), text.len() - end));
    }
    Some(pieces)
}

/// Whether `next` continues `previous` as one unedited region whose hunks are equal: both take a single hunk, so D holds the same text whichever one, and their hunks are adjacent in every source.
pub fn joins_stable(previous: &(Region, RegionState), next: &(Region, RegionState)) -> bool {
    let stable = |(region, state): &(Region, RegionState)| {
        !is_difference(&region.status) && !state.merge.edited && matches!(&state.merge.choice, Choice::Hunks(documents) if documents.len() == 1)
    };
    let adjacent = previous.0.hunks.len() == next.0.hunks.len() && previous.0.hunks.iter().zip(&next.0.hunks).all(|(left, right)| left.end == right.start);
    stable(previous) && stable(next) && previous.0.status == next.0.status && adjacent
}

/// One unedited region spanning `previous` and `next`, which must pass `joins_stable`. It keeps the choice of `previous`.
pub fn join_stable(previous: &(Region, RegionState), next: &(Region, RegionState)) -> (Region, RegionState) {
    let hunks = previous.0.hunks.iter().zip(&next.0.hunks).map(|(left, right)| left.start..right.end).collect();
    let rows = previous.0.rows.iter().chain(&next.0.rows).cloned().collect();
    let merge = RegionMerge {
        choice: previous.1.merge.choice.clone(),
        edited: false,
        output_lines: previous.1.merge.output_lines + next.1.merge.output_lines,
    };
    (
        Region {
            hunks,
            status: previous.0.status.clone(),
            rows,
        },
        RegionState {
            merge,
            text_len: previous.1.text_len + next.1.text_len,
        },
    )
}

/// The choice auto-resolve makes for a region with this status.
pub fn auto_choice(status: &RegionStatus) -> Choice {
    if status.is_conflict() {
        return Choice::Unresolved;
    }
    match status {
        RegionStatus::Merge(MergeStatus::ChangedInB | MergeStatus::ChangedAlike) => Choice::Hunks(vec![Document::B]),
        RegionStatus::Merge(MergeStatus::ChangedInC) => Choice::Hunks(vec![Document::C]),
        _ => Choice::Hunks(vec![Document::A]),
    }
}

/// A region whose hunks are not all equal.
pub fn is_difference(status: &RegionStatus) -> bool {
    !matches!(status, RegionStatus::Diff(HunkChange::Unchanged) | RegionStatus::Merge(MergeStatus::Unchanged))
}

/// Rejects choices of D, of documents the session doesn't have, and of one hunk twice.
pub fn validate_choice(choice: &Choice, source_count: usize) -> Result<(), CoreError> {
    let Choice::Hunks(documents) = choice else {
        return Ok(());
    };
    for (index, document) in documents.iter().enumerate() {
        let position = source_index(document.clone());
        if position.is_none_or(|position| position >= source_count) {
            return Err(CoreError::NoSuchDocument { document: document.clone() });
        }
        if documents[..index].contains(document) {
            return Err(CoreError::InvalidChoice {
                reason: format!("{} is chosen twice", document.letter()),
            });
        }
    }
    Ok(())
}

fn source_index(document: Document) -> Option<usize> {
    match document {
        Document::A => Some(0),
        Document::B => Some(1),
        Document::C => Some(2),
        Document::D => None,
    }
}

/// D's text for a region with `choice`: the lines of the chosen hunks in order, each with a line break.
pub fn render(region: &Region, choice: &Choice, sources: &[Vec<&str>]) -> String {
    let Choice::Hunks(documents) = choice else {
        return String::new();
    };
    let mut text = String::new();
    for document in documents {
        let Some(index) = source_index(document.clone()) else { continue };
        let (Some(hunk), Some(lines)) = (region.hunks.get(index), sources.get(index)) else {
            continue;
        };
        for line in lines.get(hunk.clone()).unwrap_or(&[]) {
            text.push_str(line);
            text.push('\n');
        }
    }
    text
}

/// Lines of a piece of D: one per line break, plus a last line without one.
pub fn line_count(text: &str) -> usize {
    let breaks = text.bytes().filter(|byte| *byte == b'\n').count();
    if text.is_empty() || text.ends_with('\n') { breaks } else { breaks + 1 }
}

/// One region spanning `regions`, which must be consecutive: their hunks and rows joined, its status classified again by `engine`. Where the regions don't follow each other in a document, because an inset exchanged the hunks there, the hunk covers all of them and the status is classified from the lines of the rows.
pub fn join(regions: &[Region], sources: &[Vec<&str>], engine: &dyn ComparisonEngine) -> Result<Region, CoreError> {
    let (Some(first), Some(last)) = (regions.first(), regions.last()) else {
        return Err(CoreError::InvalidChoice {
            reason: "nothing to join".to_string(),
        });
    };
    let follows = |document: usize| regions.windows(2).all(|pair| pair[0].hunks[document].end == pair[1].hunks[document].start);
    let hunks: Vec<Range<usize>> = (0..first.hunks.len())
        .map(|document| {
            if follows(document) {
                return first.hunks[document].start..last.hunks[document].end;
            }
            let start = regions.iter().map(|region| region.hunks[document].start).min().unwrap_or(0);
            let end = regions.iter().map(|region| region.hunks[document].end).max().unwrap_or(0);
            start..end
        })
        .collect();
    let rows: Vec<Row> = regions.iter().flat_map(|region| region.rows.iter().cloned()).collect();
    let lines: Vec<Vec<&str>> = (0..hunks.len())
        .map(|document| {
            let source = sources.get(document).map(Vec::as_slice).unwrap_or(&[]);
            if follows(document) {
                return source.get(hunks[document].clone()).unwrap_or(&[]).to_vec();
            }
            rows.iter().filter_map(|row| row.lines[document]).filter_map(|line| source.get(line).copied()).collect()
        })
        .collect();
    let tokens = normalizer::tokenize(&lines);
    let hunk_lines: Vec<SourceLines<'_>> = lines.iter().zip(&tokens).map(|(lines, tokens)| SourceLines { lines, tokens }).collect();
    let status = engine.classify(&hunk_lines)?;
    Ok(Region { hunks, status, rows })
}

#[cfg(test)]
mod tests {
    use kdiff4_protocol::engine::Row;

    use super::*;
    use crate::builtin_engine::BuiltinEngine;

    fn region(hunks: &[Range<usize>], status: RegionStatus) -> Region {
        Region {
            hunks: hunks.to_vec(),
            status,
            rows: vec![Row {
                lines: hunks.iter().map(|hunk| (!hunk.is_empty()).then(|| hunk.start)).collect(),
            }],
        }
    }

    fn sources() -> Vec<Vec<&'static str>> {
        vec![vec!["a0", "a1"], vec!["b0", "b1"], vec!["c0", "c1"]]
    }

    #[test]
    fn auto_resolve_rules() {
        assert_eq!(auto_choice(&RegionStatus::Merge(MergeStatus::Unchanged)), Choice::Hunks(vec![Document::A]));
        assert_eq!(auto_choice(&RegionStatus::Merge(MergeStatus::ChangedInB)), Choice::Hunks(vec![Document::B]));
        assert_eq!(auto_choice(&RegionStatus::Merge(MergeStatus::ChangedInC)), Choice::Hunks(vec![Document::C]));
        assert_eq!(auto_choice(&RegionStatus::Merge(MergeStatus::ChangedAlike)), Choice::Hunks(vec![Document::B]));
        assert_eq!(auto_choice(&RegionStatus::Merge(MergeStatus::Conflict)), Choice::Unresolved);
        assert_eq!(auto_choice(&RegionStatus::Merge(MergeStatus::WhitespaceConflict)), Choice::Unresolved);
        assert_eq!(auto_choice(&RegionStatus::Diff(HunkChange::Unchanged)), Choice::Hunks(vec![Document::A]));
        assert_eq!(auto_choice(&RegionStatus::Diff(HunkChange::Modified)), Choice::Unresolved);
        assert_eq!(auto_choice(&RegionStatus::Diff(HunkChange::Added)), Choice::Unresolved);
    }

    #[test]
    fn render_takes_chosen_hunks_in_order() {
        let region = region(&[0..1, 0..2, 1..2], RegionStatus::Merge(MergeStatus::Conflict));
        assert_eq!(render(&region, &Choice::Hunks(vec![Document::C, Document::B]), &sources()), "c1\nb0\nb1\n");
        assert_eq!(render(&region, &Choice::Hunks(Vec::new()), &sources()), "");
        assert_eq!(render(&region, &Choice::Unresolved, &sources()), "");
    }

    #[test]
    fn choices_are_validated() {
        assert!(validate_choice(&Choice::Hunks(vec![Document::A, Document::C]), 3).is_ok());
        assert!(validate_choice(&Choice::Unresolved, 2).is_ok());
        assert!(matches!(
            validate_choice(&Choice::Hunks(vec![Document::C]), 2),
            Err(CoreError::NoSuchDocument { document: Document::C })
        ));
        assert!(matches!(
            validate_choice(&Choice::Hunks(vec![Document::D]), 3),
            Err(CoreError::NoSuchDocument { document: Document::D })
        ));
        assert!(matches!(
            validate_choice(&Choice::Hunks(vec![Document::B, Document::B]), 3),
            Err(CoreError::InvalidChoice { .. })
        ));
    }

    #[test]
    fn line_counts() {
        assert_eq!(line_count(""), 0);
        assert_eq!(line_count("a\n"), 1);
        assert_eq!(line_count("a\nb"), 2);
        assert_eq!(line_count("\n\n"), 2);
    }

    fn merge_of(texts: &[&str]) -> Merge {
        let regions = texts
            .iter()
            .enumerate()
            .map(|(index, _)| region(&[index..index + 1, index..index + 1], RegionStatus::Diff(HunkChange::Modified)))
            .collect();
        let states = texts
            .iter()
            .map(|text| RegionState {
                merge: RegionMerge {
                    choice: Choice::Unresolved,
                    edited: true,
                    output_lines: line_count(text),
                },
                text_len: text.len(),
            })
            .collect();
        Merge { regions, states }
    }

    fn touched(texts: &[&str], range: Range<usize>, inserted: &str) -> Range<usize> {
        merge_of(texts).touched(range, inserted, &texts.concat())
    }

    #[test]
    fn edits_inside_one_region_touch_only_it() {
        assert_eq!(touched(&["ab\n", "cd\n"], 1..1, "x"), 0..1);
        assert_eq!(touched(&["ab\n", "cd\n"], 3..3, "x"), 1..2);
        assert_eq!(touched(&["ab\n", "cd\n"], 4..5, ""), 1..2);
        assert_eq!(touched(&["ab\n", "cd\n"], 6..6, "e\n"), 1..2);
    }

    #[test]
    fn edits_across_a_boundary_touch_both() {
        assert_eq!(touched(&["ab\n", "cd\n", "ef\n"], 1..4, ""), 0..2);
        assert_eq!(touched(&["ab\n", "cd\n", "ef\n"], 0..9, ""), 0..3);
    }

    #[test]
    fn removing_a_line_break_takes_in_the_next_region() {
        assert_eq!(touched(&["ab\n", "cd\n"], 2..3, ""), 0..2);
        assert_eq!(touched(&["ab\n", "", "cd\n"], 2..3, ""), 0..3);
        assert_eq!(touched(&["ab\n", "cd\n"], 2..2, "x"), 0..1);
        assert_eq!(touched(&["ab\n", "cd\n", ""], 5..6, ""), 1..2);
    }

    #[test]
    fn deleting_a_whole_region_stays_in_it() {
        assert_eq!(touched(&["ab\n", "cd\n"], 0..3, ""), 0..1);
    }

    #[test]
    fn edits_skip_regions_without_text() {
        assert_eq!(touched(&["ab\n", "", "cd\n"], 3..3, "x"), 2..3);
        assert_eq!(touched(&["ab\n", ""], 3..3, "x"), 0..1);
        assert_eq!(touched(&["", ""], 0..0, "x"), 1..2);
        assert_eq!(touched(&[], 0..0, "x"), 0..0);
    }

    #[test]
    fn offsets_and_splice() {
        let mut merge = merge_of(&["ab\n", "c\n", "def\n"]);
        assert_eq!(merge.offsets(), [0, 3, 5, 9]);
        let entries = merge.entries(0..2);
        merge.splice(0, 2, vec![entries[1].clone()]);
        assert_eq!(merge.offsets(), [0, 2, 6]);
        assert_eq!(merge.update(0, 2, 1).removed, 2);
        assert_eq!(merge.update(0, 2, 1).regions.len(), 1);
    }

    #[test]
    fn join_spans_both_regions_and_reclassifies() {
        let sources = sources();
        let first = region(&[0..1, 0..1, 0..1], RegionStatus::Merge(MergeStatus::Unchanged));
        let second = region(&[1..2, 1..2, 1..2], RegionStatus::Merge(MergeStatus::ChangedInB));
        let joined = join(&[first, second], &sources, &BuiltinEngine).unwrap();
        assert_eq!(joined.hunks, [0..2, 0..2, 0..2]);
        assert_eq!(joined.rows.len(), 2);
        assert_eq!(joined.status, RegionStatus::Merge(MergeStatus::Conflict));
    }

    #[test]
    fn join_reaches_over_an_inset_and_classifies_its_lines() {
        let sources = vec![vec!["a0", "a1", "a2"], vec!["b0", "a1", "a2"], vec!["c0", "a1", "a2"]];
        let first = region(&[2..3, 2..3, 2..3], RegionStatus::Merge(MergeStatus::Unchanged));
        let second = region(&[0..1, 0..1, 0..1], RegionStatus::Merge(MergeStatus::Conflict));
        let joined = join(&[first, second], &sources, &BuiltinEngine).unwrap();
        assert_eq!(joined.hunks, [0..3, 0..3, 0..3]);
        assert_eq!(joined.rows.len(), 2);
        assert_eq!(joined.status, RegionStatus::Merge(MergeStatus::Conflict));
    }

    fn stable() -> (Region, RegionState) {
        let region = Region {
            hunks: vec![2..5, 0..3, 4..7],
            status: RegionStatus::Merge(MergeStatus::Unchanged),
            rows: (0..3)
                .map(|line| Row {
                    lines: vec![Some(2 + line), Some(line), Some(4 + line)],
                })
                .collect(),
        };
        (
            region,
            RegionState {
                merge: RegionMerge {
                    choice: Choice::Hunks(vec![Document::A]),
                    edited: false,
                    output_lines: 3,
                },
                text_len: 9,
            },
        )
    }

    fn split_shape(range: Range<usize>, inserted: &str) -> Vec<(Vec<Range<usize>>, bool, usize)> {
        let (region, state) = stable();
        split_stable(&region, &state, "aa\nbb\ncc\n", range, inserted)
            .unwrap()
            .into_iter()
            .map(|(region, state)| (region.hunks, state.merge.edited, state.text_len))
            .collect()
    }

    #[test]
    fn editing_a_stable_line_splits_it_out() {
        assert_eq!(
            split_shape(4..4, "x"),
            [(vec![2..3, 0..1, 4..5], false, 3), (vec![3..4, 1..2, 5..6], true, 4), (vec![4..5, 2..3, 6..7], false, 3)]
        );
        assert_eq!(split_shape(0..1, ""), [(vec![2..3, 0..1, 4..5], true, 2), (vec![3..5, 1..3, 5..7], false, 6)]);
        assert_eq!(
            split_shape(
                1..7,
                "1
2
"
            ),
            [(vec![2..3, 0..1, 4..5], true, 3), (vec![3..4, 1..2, 5..6], true, 2), (vec![4..5, 2..3, 6..7], true, 2)]
        );
        assert_eq!(split_shape(7..8, "z"), [(vec![2..4, 0..2, 4..6], false, 6), (vec![4..5, 2..3, 6..7], true, 3)]);
    }

    #[test]
    fn inserted_lines_get_empty_hunks() {
        assert_eq!(
            split_shape(3..3, "new\n"),
            [(vec![2..3, 0..1, 4..5], false, 3), (vec![3..3, 1..1, 5..5], true, 4), (vec![3..5, 1..3, 5..7], false, 6)]
        );
    }

    #[test]
    fn removing_a_line_break_takes_in_the_next_line() {
        assert_eq!(
            split_shape(2..3, ""),
            [(vec![2..3, 0..1, 4..5], true, 5), (vec![3..4, 1..2, 5..6], true, 0), (vec![4..5, 2..3, 6..7], false, 3)]
        );
    }

    #[test]
    fn extra_new_lines_get_empty_hunks() {
        assert_eq!(
            split_shape(
                0..2,
                "x
y
z"
            ),
            [
                (vec![2..3, 0..1, 4..5], true, 2),
                (vec![3..3, 1..1, 5..5], true, 2),
                (vec![3..3, 1..1, 5..5], true, 2),
                (vec![3..5, 1..3, 5..7], false, 6)
            ]
        );
    }

    #[test]
    fn edited_or_differing_regions_dont_split() {
        let (region, mut state) = stable();
        state.merge.edited = true;
        assert!(split_stable(&region, &state, "aa\nbb\ncc\n", 0..0, "x").is_none());
        let (mut region, state) = stable();
        region.status = RegionStatus::Merge(MergeStatus::Conflict);
        assert!(split_stable(&region, &state, "aa\nbb\ncc\n", 0..0, "x").is_none());
    }

    #[test]
    fn typing_steps_join_inside_one_region_only() {
        let entry = |text_len| {
            (
                region(&[0..1, 0..1], RegionStatus::Diff(HunkChange::Modified)),
                RegionState {
                    merge: RegionMerge {
                        choice: Choice::Unresolved,
                        edited: true,
                        output_lines: 1,
                    },
                    text_len,
                },
            )
        };
        let first = MergeStep {
            at: 2,
            before: vec![entry(3)],
            after: vec![entry(4)],
            explicit_final_eol: None,
        };
        let second = MergeStep {
            at: 2,
            before: vec![entry(4)],
            after: vec![entry(5)],
            explicit_final_eol: None,
        };
        assert_eq!(
            first.joined(&second),
            Some(MergeStep {
                at: 2,
                before: vec![entry(3)],
                after: vec![entry(5)],
                explicit_final_eol: None,
            })
        );
        let elsewhere = MergeStep { at: 3, ..second.clone() };
        assert_eq!(first.joined(&elsewhere), None);
        let joining = MergeStep {
            at: 2,
            before: vec![entry(4), entry(1)],
            after: vec![entry(6)],
            explicit_final_eol: None,
        };
        assert_eq!(first.joined(&joining), None);
    }
}
