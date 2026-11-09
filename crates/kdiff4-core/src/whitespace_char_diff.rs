use kdiff4_protocol::engine::{CharRun, ComparisonEngine, ComparisonOptions, EngineError};

/// The character-level diff of one chunk with whitespace ignored: per source document, the lines of its part of the chunk (a region's hunk, or a row's line). Every document's lines are joined without their whitespace, so text reflowed over other lines or reindented compares equal, and diffed as one line by `engine`. Whitespace is plain: the same in every document. Returns, per document and per line of its part, the runs that cover the line.
pub fn chunk_runs(engine: &dyn ComparisonEngine, chunk: &[Vec<&str>], options: &ComparisonOptions) -> Result<Vec<Vec<Vec<CharRun>>>, EngineError> {
    let documents = chunk.len();
    let stripped: Vec<(String, Vec<(usize, usize)>)> = chunk.iter().map(|lines| strip(lines)).collect();
    let row: Vec<Option<&str>> = stripped.iter().map(|(text, _)| Some(text.as_str())).collect();
    let diff = engine.char_diff(&row, options)?;
    if diff.runs.len() != documents {
        return Err(EngineError::InvalidInput(format!("{} lines for a row of {documents}", diff.runs.len())));
    }
    let mut result = Vec::with_capacity(documents);
    for (document, (lines, runs)) in chunk.iter().zip(&diff.runs).enumerate() {
        let mut flags: Vec<Vec<Vec<bool>>> = lines.iter().map(|line| vec![vec![true; documents]; line.chars().count()]).collect();
        let places = &stripped[document].1;
        for run in runs {
            for column in run.columns.clone() {
                let Some((line, at)) = places.get(column).cloned() else { continue };
                if let Some(slot) = flags.get_mut(line).and_then(|line| line.get_mut(at)) {
                    *slot = run.same_in.clone();
                }
            }
        }
        result.push(flags.into_iter().map(runs_of).collect());
    }
    Ok(result)
}


/// The non-whitespace characters of `lines` joined, and for each of them its line and column.
fn strip(lines: &[&str]) -> (String, Vec<(usize, usize)>) {
    let mut text = String::new();
    let mut places = Vec::new();
    for (line, chars) in lines.iter().enumerate() {
        for (column, char) in chars.chars().enumerate() {
            if !char.is_whitespace() {
                text.push(char);
                places.push((line, column));
            }
        }
    }
    (text, places)
}


/// Joins consecutive characters with the same flags into runs.
fn runs_of(flags: Vec<Vec<bool>>) -> Vec<CharRun> {
    let mut runs: Vec<CharRun> = Vec::new();
    for (column, same_in) in flags.into_iter().enumerate() {
        match runs.last_mut() {
            Some(run) if run.same_in == same_in => run.columns.end = column + 1,
            _ => runs.push(CharRun {
                columns: column..column + 1,
                same_in,
            }),
        }
    }
    runs
}


/// Whether the joined non-whitespace text of `chunk` is longer than the char-diff line limit in any document.
pub fn exceeds_limit(chunk: &[Vec<&str>], options: &ComparisonOptions) -> bool {
    chunk
        .iter()
        .map(|lines| lines.iter().map(|line| line.chars().filter(|char| !char.is_whitespace()).count()).sum::<usize>())
        .any(|count| count > options.char_diff_line_limit)
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin_engine::BuiltinEngine;

    fn run(columns: std::ops::Range<usize>, same_in: &[bool]) -> CharRun {
        CharRun { columns, same_in: same_in.to_vec() }
    }

    fn diff(chunk: &[Vec<&str>]) -> Vec<Vec<Vec<CharRun>>> {
        chunk_runs(&BuiltinEngine, chunk, &ComparisonOptions::default()).unwrap()
    }

    #[test]
    fn chopped_down_parameters_are_the_same() {
        let runs = diff(&[vec!["call(a, b, c);"], vec!["call(", "    a,", "    b,", "    c);"]]);
        assert_eq!(runs[0], [vec![run(0..14, &[true, true])]]);
        assert_eq!(runs[1], [vec![run(0..5, &[true, true])], vec![run(0..6, &[true, true])], vec![run(0..6, &[true, true])], vec![run(0..7, &[true, true])]]);
    }

    #[test]
    fn a_real_change_among_reflowed_lines_is_marked() {
        let runs = diff(&[vec!["call(alpha, beta);"], vec!["call(", "    alpha,", "    four);"]]);
        let marked_a: Vec<usize> = runs[0][0].iter().filter(|run| !run.same_in[1]).flat_map(|run| run.columns.clone()).collect();
        assert_eq!(marked_a, (12..16).collect::<Vec<_>>());
        assert!(runs[1][1].iter().all(|run| run.same_in[0]));
        let marked_b: Vec<usize> = runs[1][2].iter().filter(|run| !run.same_in[0]).flat_map(|run| run.columns.clone()).collect();
        assert_eq!(marked_b, (4..8).collect::<Vec<_>>());
    }

    #[test]
    fn indentation_and_blank_lines_are_plain() {
        let runs = diff(&[vec!["\tx = 1;", ""], vec!["x  =  1;"], vec![]]);
        assert!(runs[0].iter().flatten().all(|run| run.same_in[1]));
        assert_eq!(runs[0][1], Vec::new());
        assert!(runs[1].iter().flatten().all(|run| run.same_in[0]));
        assert!(runs[0][0].iter().any(|run| !run.same_in[2]));
    }

    #[test]
    fn whitespace_is_the_same_in_every_document() {
        let runs = diff(&[vec!["a b"], vec!["xy"], vec!["a\tb"]]);
        assert_eq!(runs[0][0][1], run(1..2, &[true, true, true]));
        assert!(runs[1][0].iter().all(|run| !run.same_in[0] && !run.same_in[2]));
        assert_eq!(runs[2][0], [run(0..1, &[true, false, true]), run(1..2, &[true, true, true]), run(2..3, &[true, false, true])]);
    }

    #[test]
    fn whitespace_only_hunk_against_an_empty_one_is_plain() {
        let runs = diff(&[vec!["  ", "\t"], vec![]]);
        assert!(runs[0].iter().flatten().all(|run| run.same_in == [true, true]));
        assert!(runs[1].is_empty());
    }

    #[test]
    fn limit_counts_only_non_whitespace() {
        let options = ComparisonOptions {
            char_diff_line_limit: 3,
            ..ComparisonOptions::default()
        };
        assert!(!exceeds_limit(&[vec!["a b", "  c"]], &options));
        assert!(exceeds_limit(&[vec!["ab", "cd"]], &options));
    }
}
