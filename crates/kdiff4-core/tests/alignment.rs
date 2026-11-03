// kdiff3 alignment test data (`test/testdata/`), compared with the built-in engine.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use kdiff4_core::{BuiltinEngine, SourceText};
use kdiff4_protocol::engine::{CompareInput, ComparisonOptions, Job, Progress, SourceLines, Token, run_comparison};


fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test_data/alignment")
}


fn lines(path: &Path) -> Vec<String> {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let text = SourceText::decode(&bytes);
    text.lines
        .iter()
        .map(|range| text.text[range.clone()].to_string())
        .collect()
}


/// Rows of `A B C` line numbers, -1 for a void. Rows that hold only lines past the end of every document are dropped: kdiff3 counts an empty last line after the final line break.
fn expected_rows(path: &Path, counts: &[usize]) -> Vec<Vec<Option<usize>>> {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .lines()
        .map(|line| {
            line.split_whitespace()
                .zip(counts)
                .map(|(number, count)| number.parse::<i64>().ok().filter(|&number| number >= 0 && (number as usize) < *count).map(|number| number as usize))
                .collect::<Vec<Option<usize>>>()
        })
        .filter(|row| row.iter().any(|line| line.is_some()))
        .collect()
}


fn check_case(name: &str) {
    let dir = data_dir();
    let documents: Vec<Vec<String>> = ["base", "contrib1", "contrib2"]
        .iter()
        .map(|part| lines(&dir.join(format!("{name}_{part}.txt"))))
        .collect();
    let mut interned: HashMap<&str, Token> = HashMap::new();
    let mut tokens = Vec::new();
    for document in &documents {
        let mut document_tokens = Vec::new();
        for line in document {
            let next = Token(interned.len() as u32);
            document_tokens.push(interned.entry(line.as_str()).or_insert(next).clone());
        }
        tokens.push(document_tokens);
    }
    let line_refs: Vec<Vec<&str>> = documents
        .iter()
        .map(|document| document.iter().map(|line| line.as_str()).collect())
        .collect();
    let sources = line_refs
        .iter()
        .zip(&tokens)
        .map(|(lines, tokens)| SourceLines { lines, tokens })
        .collect();
    let options = ComparisonOptions::default();
    let cancel = AtomicBool::new(false);
    let progress = |_: Progress| {};
    let comparison = run_comparison(&BuiltinEngine, &CompareInput { sources, alignments: &[], options: &options }, &Job::new(&cancel, &progress)).unwrap();
    let rows: Vec<Vec<Option<usize>>> = comparison
        .regions
        .iter()
        .flat_map(|region| region.rows.iter().map(|row| row.lines.clone()))
        .collect();
    let counts: Vec<usize> = documents.iter().map(|document| document.len()).collect();
    assert_eq!(rows, expected_rows(&dir.join(format!("{name}_expected_result.txt")), &counts), "{name}");
}


#[test]
fn simple_test() {
    check_case("1_simpletest");
}


#[test]
fn prefer_identical_to_space_differences() {
    check_case("2_prefer_identical_to_space_differences");
}


#[test]
fn every_case_has_a_test() {
    let mut cases: Vec<String> = fs::read_dir(data_dir())
        .expect("Test: alignment data folder")
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().to_str().and_then(|name| name.strip_suffix("_base.txt")).map(|name| name.to_string()))
        .collect();
    cases.sort();
    assert_eq!(cases, ["1_simpletest", "2_prefer_identical_to_space_differences"]);
}
