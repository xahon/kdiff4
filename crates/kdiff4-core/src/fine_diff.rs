use std::ops::Range;

/// How many characters of the second line kdiff3 looks at for a match of one character of the first line.
const MAX_SEARCH_RANGE: usize = 500;

/// Runs of equal characters shorter than this are merged into the change next to them, unless the lines have a longer run.
const SHORT_MATCH: usize = 4;

/// `equal` characters that are the same in both lines, followed by `first` characters of the first line and `second` characters of the second line that are not.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Step {
    equal: usize,
    first: usize,
    second: usize,
}

/// The changed columns of two lines, found the way kdiff3 does: per change, its columns in `first` and in `second`, either of which can be empty. The result depends on which line comes first. Unlike kdiff3, when the lines have a long match, a change starts at the first character that differs.
/// kdiff3: diff.cpp, `DiffList::calcDiff` and `DiffList::optimize`
pub fn changed_ranges(first: &[char], second: &[char]) -> Vec<(Range<usize>, Range<usize>)> {
    let mut steps = match_steps(first, second, 2 * (first.len() + second.len()) + 2);
    let long_match = has_long_match(&steps);
    merge_short_matches(&mut steps);
    let (mut at_first, mut at_second) = (0, 0);
    let mut ranges: Vec<(Range<usize>, Range<usize>)> = Vec::new();
    for step in steps {
        at_first += step.equal;
        at_second += step.equal;
        if step.first > 0 || step.second > 0 {
            match ranges.last_mut() {
                Some((first_columns, second_columns)) if first_columns.end == at_first && second_columns.end == at_second => {
                    first_columns.end += step.first;
                    second_columns.end += step.second;
                }
                _ => ranges.push((at_first..at_first + step.first, at_second..at_second + step.second)),
            }
        }
        at_first += step.first;
        at_second += step.second;
    }
    if long_match {
        skip_shared_starts(first, second, &mut ranges);
    }
    ranges
}

/// Matching can pull a character that both lines have at the same place into the start of a change, like the space after the `=` of `let total = cost` and `let total = price * count`; this moves each change past such characters and drops a change that nothing is left of.
fn skip_shared_starts(first: &[char], second: &[char], ranges: &mut Vec<(Range<usize>, Range<usize>)>) {
    for (first_columns, second_columns) in ranges.iter_mut() {
        while first_columns.start < first_columns.end && second_columns.start < second_columns.end && first[first_columns.start] == second[second_columns.start] {
            first_columns.start += 1;
            second_columns.start += 1;
        }
    }
    ranges.retain(|(first_columns, second_columns)| !first_columns.is_empty() || !second_columns.is_empty());
}

fn has_long_match(steps: &[Step]) -> bool {
    steps.iter().any(|step| step.equal >= SHORT_MATCH)
}

/// Not an LCS: after each run of equal characters it takes the nearest plausible match, then goes back over equal characters that the match skipped. After `max_rounds` rounds it gives up and reports the lines as different as a whole.
fn match_steps(first: &[char], second: &[char], max_rounds: usize) -> Vec<Step> {
    let mut steps: Vec<Step> = Vec::new();
    let (mut at_first, mut at_second) = (0, 0);
    for _ in 0..max_rounds {
        let mut equal = 0;
        while at_first < first.len() && at_second < second.len() && first[at_first] == second[at_second] {
            at_first += 1;
            at_second += 1;
            equal += 1;
        }
        let mut end_reached = false;
        let mut step = match nearest_match(&first[at_first..], &second[at_second..]) {
            Some((mut skip_first, mut skip_second)) => {
                while skip_first >= 1 && skip_second >= 1 && first[at_first + skip_first - 1] == second[at_second + skip_second - 1] {
                    skip_first -= 1;
                    skip_second -= 1;
                }
                at_first += skip_first;
                at_second += skip_second;
                Step {
                    equal,
                    first: skip_first.clone(),
                    second: skip_second.clone(),
                }
            }
            None => {
                end_reached = true;
                Step {
                    equal,
                    first: first.len() - at_first,
                    second: second.len() - at_second,
                }
            }
        };
        let matched_before = (1..=at_first.min(at_second)).take_while(|back| first[at_first - back] == second[at_second - back]).count();
        if matched_before > 0 {
            let original = step.clone();
            let mut remaining = matched_before.clone();
            while remaining > 0 {
                if step.first > 0 && step.second > 0 {
                    step.first -= 1;
                    step.second -= 1;
                    remaining -= 1;
                } else if step.equal > 0 {
                    step.equal -= 1;
                    remaining -= 1;
                }
                if step.equal == 0 && (step.first == 0 || step.second == 0) && remaining > 0 {
                    let Some(previous) = steps.pop() else { break };
                    step.equal += previous.equal;
                    step.first += previous.first;
                    step.second += previous.second;
                    end_reached = false;
                }
            }
            if end_reached {
                step = original;
            } else {
                at_first = at_first - matched_before + remaining;
                at_second = at_second - matched_before + remaining;
            }
        }
        steps.push(step);
        if end_reached {
            return steps;
        }
    }
    vec![Step {
        equal: 0,
        first: first.len(),
        second: second.len(),
    }]
}

/// How many characters to skip in `first` and in `second` to reach the match with the fewest skipped characters. A match counts when its characters are at most two columns apart, when the next characters match too, or when both are the last ones.
fn nearest_match(first: &[char], second: &[char]) -> Option<(usize, usize)> {
    let mut best = None;
    let mut fewest_skipped = usize::MAX;
    for skip_first in 0..first.len() {
        if skip_first >= fewest_skipped {
            break;
        }
        for skip_second in 0..MAX_SEARCH_RANGE.min(second.len()) {
            if skip_first + skip_second >= fewest_skipped {
                break;
            }
            if first[skip_first] == second[skip_second] && is_plausible(first, second, skip_first, skip_second) {
                best = Some((skip_first, skip_second));
                fewest_skipped = skip_first + skip_second;
                break;
            }
        }
    }
    best
}

fn is_plausible(first: &[char], second: &[char], skip_first: usize, skip_second: usize) -> bool {
    let first_is_last = skip_first + 1 == first.len();
    let second_is_last = skip_second + 1 == second.len();
    skip_first.abs_diff(skip_second) < 3 || (first_is_last && second_is_last) || (!first_is_last && !second_is_last && first[skip_first + 1] == second[skip_second + 1])
}

/// A short match is merged into the change after it. Only the first step keeps its short match, and only when another match is long.
/// kdiff3 1.14.70 inverts this and merges just the first step (a 2022 refactor, commit 92b1326e); kdiff3 0.9.98 and Kdiff4 do what is written here.
fn merge_short_matches(steps: &mut [Step]) {
    let long_match = has_long_match(steps);
    for (index, step) in steps.iter_mut().enumerate() {
        if (!long_match || index > 0) && step.equal < SHORT_MATCH && (step.first > 0 || step.second > 0) {
            step.first += step.equal;
            step.second += step.equal;
            step.equal = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn chars(text: &str) -> Vec<char> {
        text.chars().collect()
    }

    fn ranges(first: &str, second: &str) -> Vec<(Range<usize>, Range<usize>)> {
        changed_ranges(&chars(first), &chars(second))
    }

    #[test]
    fn equal_lines_have_no_changes() {
        assert_eq!(ranges("", ""), []);
        assert_eq!(ranges("let x = 1;", "let x = 1;"), []);
    }

    #[test]
    fn whole_lines_can_be_added_or_removed() {
        assert_eq!(ranges("", "a"), [(0..0, 0..1)]);
        assert_eq!(ranges("a", ""), [(0..1, 0..0)]);
        assert_eq!(ranges("abc", "xyz"), [(0..3, 0..3)]);
    }

    #[test]
    fn a_renamed_word_is_changed_as_a_whole() {
        assert_eq!(ranges("let total = price * quantity;", "let total = cost * quantity;"), [(12..17, 12..16)]);
    }

    #[test]
    fn short_matches_inside_a_change_are_merged_into_it() {
        assert_eq!(ranges("let total = price * count + tax;", "let total = price * quantity;"), [(20..31, 20..28)]);
    }

    #[test]
    fn a_change_does_not_start_with_the_space_both_lines_have_there() {
        assert_eq!(ranges("let total = cost * quantity;", "let total = price * count + tax;"), [(12..27, 12..31)]);
    }

    #[test]
    fn a_change_keeps_the_characters_both_lines_have_there_when_the_lines_have_no_long_match() {
        assert_eq!(ranges("abc", "abd"), [(0..3, 0..3)]);
        assert_eq!(ranges("ab cd", "ab ef"), [(0..5, 0..5)]);
    }

    #[test]
    fn the_direction_changes_which_characters_match() {
        assert_eq!(ranges("a a", "alpha a"), [(0..0, 0..4)]);
        assert_eq!(ranges("alpha a", "a a"), [(0..7, 0..3)]);
    }

    #[test]
    fn lines_without_a_long_match_are_changed_as_a_whole() {
        assert_eq!(ranges("abc", "abd"), [(0..3, 0..3)]);
        assert_eq!(ranges("abcdef", "abcdeg"), [(5..6, 5..6)]);
    }

    #[test]
    fn the_first_short_match_is_kept_when_another_match_is_long() {
        assert_eq!(ranges("abXdefgh", "abYdefgh"), [(2..3, 2..3)]);
    }

    #[test]
    fn gives_up_after_the_round_limit_with_the_lines_different_as_a_whole() {
        let (first, second) = (chars("let total = price * count + tax;"), chars("let total = price * quantity;"));
        assert_eq!(
            match_steps(&first, &second, 1),
            [Step {
                equal: 0,
                first: first.len(),
                second: second.len()
            }]
        );
        assert_ne!(match_steps(&first, &second, 100).len(), 1);
    }

    fn kept(chars: &[char], changes: Vec<Range<usize>>) -> Vec<char> {
        let mut changed = vec![false; chars.len()];
        for column in changes.into_iter().flatten() {
            changed[column] = true;
        }
        chars.iter().zip(changed).filter(|(_, changed)| !changed).map(|(&char, _)| char).collect()
    }

    proptest! {
        #[test]
        fn a_change_of_lines_with_a_long_match_does_not_start_with_a_character_both_lines_have_there(first in "[abc ]{0,40}", second in "[abc ]{0,40}") {
            let (first, second) = (chars(&first), chars(&second));
            let long_match = has_long_match(&match_steps(&first, &second, 2 * (first.len() + second.len()) + 2));
            for (first_columns, second_columns) in changed_ranges(&first, &second) {
                let both_start_with_a_character = first_columns.start < first_columns.end && second_columns.start < second_columns.end;
                prop_assert!(!(long_match && both_start_with_a_character && first[first_columns.start] == second[second_columns.start]));
            }
        }


        #[test]
        fn what_is_not_changed_is_the_same_in_both_lines(first in "[abc ]{0,40}", second in "[abc ]{0,40}") {
            let (first, second) = (chars(&first), chars(&second));
            let ranges = changed_ranges(&first, &second);
            let ordered = ranges.iter().zip(ranges.iter().skip(1)).all(|(before, after)| before.0.end <= after.0.start && before.1.end <= after.1.start);
            prop_assert!(ordered);
            let first_changes: Vec<Range<usize>> = ranges.iter().map(|(first, _)| first.clone()).collect();
            let second_changes: Vec<Range<usize>> = ranges.iter().map(|(_, second)| second.clone()).collect();
            prop_assert_eq!(kept(&first, first_changes), kept(&second, second_changes));
        }
    }
}
