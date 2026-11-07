//! Pairing the lines of a changed chunk by how alike they are, instead of by position, so a line that changed a little shares its row even when lines were added or removed before it.

/// Lines at least this alike, in percent, may share a row.
const MIN_SIMILARITY: u32 = 50;
/// Chunks with more line pairs than this are paired by position, which keeps the cost bounded.
const MAX_CELLS: usize = 250_000;


/// Rows of a changed chunk: per row the line of `before` and of `after` it shows, counting from the start of each. The lines are paired in order, so that the pairs of lines at least `MIN_SIMILARITY` alike are as alike as possible in total; the lines between those pairs are paired by position, and the rest of the longer side gets rows of its own, like a chunk with no similar lines.
pub fn chunk_rows(before: &[&str], after: &[&str]) -> Vec<(Option<usize>, Option<usize>)> {
    if before.is_empty() || after.is_empty() || before.len().saturating_mul(after.len()) > MAX_CELLS {
        return by_position(0..before.len(), 0..after.len());
    }
    let before_grams: Vec<Vec<u32>> = before.iter().map(|line| bigrams(line)).collect();
    let after_grams: Vec<Vec<u32>> = after.iter().map(|line| bigrams(line)).collect();
    let similarity = |i: usize, j: usize| similarity(before[i], &before_grams[i], after[j], &after_grams[j]);
    let columns = after.len() + 1;
    let mut best = vec![0u32; (before.len() + 1) * columns];
    for i in 1..=before.len() {
        for j in 1..=after.len() {
            let skip = best[(i - 1) * columns + j].max(best[i * columns + j - 1]);
            let alike = similarity(i - 1, j - 1);
            let pair = if alike >= MIN_SIMILARITY { best[(i - 1) * columns + j - 1] + alike } else { 0 };
            best[i * columns + j] = skip.max(pair);
        }
    }
    let mut anchors = Vec::new();
    let (mut i, mut j) = (before.len(), after.len());
    while i > 0 && j > 0 {
        let here = best[i * columns + j];
        let alike = similarity(i - 1, j - 1);
        if alike >= MIN_SIMILARITY && here == best[(i - 1) * columns + j - 1] + alike {
            anchors.push((i - 1, j - 1));
            i -= 1;
            j -= 1;
        } else if here == best[(i - 1) * columns + j] {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    anchors.reverse();
    let mut rows = Vec::with_capacity(before.len().max(after.len()));
    let (mut next_before, mut next_after) = (0, 0);
    for (line_before, line_after) in anchors {
        rows.extend(by_position(next_before..line_before, next_after..line_after));
        rows.push((Some(line_before.clone()), Some(line_after.clone())));
        next_before = line_before + 1;
        next_after = line_after + 1;
    }
    rows.extend(by_position(next_before..before.len(), next_after..after.len()));
    rows
}


/// How good a row is for a reader, as the sum over each two lines it shows: 1000 for equal lines, their similarity for lines at least `MIN_SIMILARITY` alike, and nothing otherwise. Equal lines outweigh any number of similar ones, so rows that keep equal lines of two documents together win, like kdiff3's.
pub fn row_score(lines: &[Option<&str>]) -> u32 {
    let present: Vec<&str> = lines.iter().flatten().copied().collect();
    let mut score = 0;
    for (index, first) in present.iter().enumerate() {
        for second in present[index + 1..].iter() {
            score += if first == second {
                1000
            } else {
                let alike = similarity(first, &bigrams(first), second, &bigrams(second));
                if alike >= MIN_SIMILARITY { alike } else { 0 }
            };
        }
    }
    score
}


/// The lines of `before` and `after` paired by position, then the rest of the longer side alone.
fn by_position(before: std::ops::Range<usize>, after: std::ops::Range<usize>) -> Vec<(Option<usize>, Option<usize>)> {
    let paired = before.len().min(after.len());
    let mut rows: Vec<(Option<usize>, Option<usize>)> = (0..paired).map(|offset| (Some(before.start + offset), Some(after.start + offset))).collect();
    rows.extend((before.start + paired..before.end).map(|line| (Some(line), None)));
    rows.extend((after.start + paired..after.end).map(|line| (None, Some(line))));
    rows
}


/// The sorted pairs of adjacent characters of a line without its leading and trailing whitespace, each packed into one number.
fn bigrams(line: &str) -> Vec<u32> {
    let chars: Vec<char> = line.trim().chars().collect();
    let mut grams: Vec<u32> = chars
        .windows(2)
        .map(|pair| (u32::from(pair[0]) & 0xFFFF) << 16 | (u32::from(pair[1]) & 0xFFFF))
        .collect();
    grams.sort_unstable();
    grams
}


/// How alike two lines are, in percent: twice their common character pairs over all of them. Lines too short to have a pair are alike only when equal.
fn similarity(first: &str, first_grams: &[u32], second: &str, second_grams: &[u32]) -> u32 {
    if first_grams.is_empty() || second_grams.is_empty() {
        return if first.trim() == second.trim() { 100 } else { 0 };
    }
    let (mut i, mut j, mut common) = (0, 0, 0);
    while i < first_grams.len() && j < second_grams.len() {
        match first_grams[i].cmp(&second_grams[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                common += 1;
                i += 1;
                j += 1;
            }
        }
    }
    u32::try_from(common * 200 / (first_grams.len() + second_grams.len())).unwrap_or(0)
}


#[cfg(test)]
mod tests {
    use super::*;

    fn rows(before: &[&str], after: &[&str]) -> Vec<(Option<usize>, Option<usize>)> {
        chunk_rows(before, after)
    }


    #[test]
    fn a_changed_line_after_an_insertion_keeps_its_partner() {
        let before = ["    total += value;"];
        let after = ["    log(value);", "    total += value * 2;"];
        assert_eq!(rows(&before, &after), [(None, Some(0)), (Some(0), Some(1))]);
    }


    #[test]
    fn lines_with_nothing_alike_are_paired_by_position() {
        let before = ["alpha", "beta"];
        let after = ["xyz", "uvw", "rst"];
        assert_eq!(rows(&before, &after), [(Some(0), Some(0)), (Some(1), Some(1)), (None, Some(2))]);
    }


    #[test]
    fn lines_between_similar_pairs_are_paired_by_position() {
        let before = ["int first = 1;", "aaaa", "bbbb", "int last = 9;"];
        let after = ["int first = 2;", "cccc", "int last = 8;"];
        assert_eq!(rows(&before, &after), [(Some(0), Some(0)), (Some(1), Some(1)), (Some(2), None), (Some(3), Some(2))]);
    }


    #[test]
    fn a_removed_line_before_a_changed_one_gets_its_own_row() {
        let before = ["// old note", "return compute(x, y);"];
        let after = ["return compute(x, y, z);"];
        assert_eq!(rows(&before, &after), [(Some(0), None), (Some(1), Some(0))]);
    }


    #[test]
    fn similarity_ignores_indentation_and_needs_pairs_or_equality() {
        let (a, b) = ("    foo(bar);", "foo(bar);");
        assert_eq!(similarity(a, &bigrams(a), b, &bigrams(b)), 100);
        assert_eq!(similarity("{", &bigrams("{"), "{", &bigrams("{")), 100);
        assert_eq!(similarity("{", &bigrams("{"), "}", &bigrams("}")), 0);
        assert_eq!(similarity("", &bigrams(""), "  ", &bigrams("  ")), 100);
    }


    #[test]
    fn equal_lines_outweigh_similar_ones() {
        assert_eq!(row_score(&[Some("aaa"), None, Some("aaa")]), 1000);
        assert_eq!(row_score(&[Some("x = 1;"), Some("x = 2;")]), 60);
        assert_eq!(row_score(&[Some("alpha"), Some("omega")]), 0);
        assert_eq!(row_score(&[Some("aaa")]), 0);
    }


    #[test]
    fn huge_chunks_are_paired_by_position() {
        let before = vec!["x"; 600];
        let after = vec!["y"; 600];
        assert_eq!(rows(&before, &after).len(), 600);
        assert!(rows(&before, &after).iter().all(|(first, second)| first == second));
    }
}
