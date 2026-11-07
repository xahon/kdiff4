// Merge cases in `test_data/merge/`, run through the host the way the GUI drives it. A case folder holds `base`, `left` and `right` (3-way) or `left` and `right` (2-way), plus `expected`: the auto-merge with B chosen for every conflict left unresolved.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use kdiff4_protocol::engine::{HunkChange, MergeStatus, RegionStatus};
use kdiff4_protocol::{Choice, Client, Command, Document, DocumentText, EntityKind, ErrorKind, Event, HeuristicOptions, HeuristicProposal, HostMessage, MatchStage, OpenFiles, Snapshot, in_process};

fn case_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test_data/merge").join(name)
}

fn extension(name: &str) -> &str {
    name.rsplit('_').next().unwrap_or("txt")
}

fn start() -> (Client, JoinHandle<()>) {
    let (client, endpoint) = in_process();
    let host = kdiff4_core::spawn_host(endpoint).unwrap();
    (client, host)
}

/// Sends `command` and waits for its response, skipping events. Returns the last snapshot seen on the way, if any.
fn run(client: &mut Client, command: Command) -> Option<Snapshot> {
    let id = client.send(command).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut snapshot = None;
    loop {
        match client.try_recv().unwrap() {
            Some(HostMessage::Response(response)) if response.id == id => {
                response.result.unwrap();
                return snapshot;
            }
            Some(HostMessage::Event(Event::Snapshot(received))) => snapshot = Some(received),
            Some(_) => {}
            None => {
                assert!(Instant::now() < deadline, "no response from the host");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}

/// Sends `ProposeHeuristicAlignments` and returns the proposal the host answers with.
fn propose(client: &mut Client, language: &str, options: HeuristicOptions) -> HeuristicProposal {
    propose_within(client, language, options, None)
}

fn propose_within(client: &mut Client, language: &str, options: HeuristicOptions, within: Option<u32>) -> HeuristicProposal {
    let id = client
        .send(Command::ProposeHeuristicAlignments {
            language: language.to_string(),
            options,
            within,
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut proposal = None;
    loop {
        match client.try_recv().unwrap() {
            Some(HostMessage::Response(response)) if response.id == id => {
                response.result.unwrap();
                return proposal.expect("Test: the host answers a proposal before the response");
            }
            Some(HostMessage::Event(Event::HeuristicProposal(received))) => {
                assert_eq!(received.cause, Some(id.clone()));
                proposal = Some(received);
            }
            Some(_) => {}
            None => {
                assert!(Instant::now() < deadline, "no response from the host");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}


fn resync(client: &mut Client) -> Snapshot {
    run(client, Command::Resync).expect("Test: resync answers with a snapshot")
}

fn open(client: &mut Client, name: &str) -> Snapshot {
    let dir = case_dir(name);
    let extension = extension(name);
    let path = |part: &str| dir.join(format!("{part}.{extension}"));
    let output = Some(std::env::temp_dir().join(format!("kdiff4-merge-data-{name}-{}.{extension}", std::process::id())));
    let files = match path("base").exists() {
        true => OpenFiles {
            a: path("base"),
            b: path("left"),
            c: Some(path("right")),
            output,
            ..OpenFiles::default()
        },
        false => OpenFiles {
            a: path("left"),
            b: path("right"),
            output,
            ..OpenFiles::default()
        },
    };
    run(client, Command::Open(files)).expect("Test: open answers with a snapshot")
}

/// The text D holds after choosing every line of `text`: its lines, each with a line break.
fn as_output(text: &DocumentText) -> String {
    (0..text.line_count()).map(|line| format!("{}\n", text.line(line))).collect()
}

#[derive(Debug, PartialEq, Eq)]
struct Counts {
    differences: usize,
    conflicts: usize,
    unresolved: usize,
}

fn counts(snapshot: &Snapshot) -> Counts {
    let statuses: Vec<&RegionStatus> = snapshot.comparison.regions.iter().map(|region| &region.status).collect();
    let differences = statuses
        .iter()
        .filter(|status| !matches!(status, RegionStatus::Diff(HunkChange::Unchanged) | RegionStatus::Merge(MergeStatus::Unchanged)))
        .count();
    let conflicts = statuses
        .iter()
        .filter(|status| matches!(status, RegionStatus::Merge(MergeStatus::Conflict | MergeStatus::WhitespaceConflict)))
        .count();
    let unresolved = snapshot.merge.iter().filter(|state| state.is_unresolved()).count();
    Counts {
        differences,
        conflicts,
        unresolved,
    }
}

fn check_case(name: &str, expected_counts: Counts) {
    let (mut client, host) = start();
    let snapshot = open(&mut client, name);
    assert_eq!(counts(&snapshot), expected_counts, "{name}");
    assert_eq!(snapshot.merge.len(), snapshot.comparison.regions.len(), "{name}");

    run(
        &mut client,
        Command::ChooseForUnresolved {
            base: snapshot.revision.clone(),
            document: Document::B,
        },
    );
    let merged = resync(&mut client);
    let expected_path = case_dir(name).join(format!("expected.{}", extension(name)));
    let expected = fs::read_to_string(&expected_path).unwrap_or_else(|error| panic!("{}: {error}", expected_path.display()));
    assert_eq!(merged.output.as_ref().unwrap().text, expected, "{name}: auto-merge, B for the rest");
    assert_eq!(counts(&merged).unresolved, 0, "{name}");

    for source in &snapshot.documents {
        let mut revision = resync(&mut client).revision;
        for region in 0..snapshot.comparison.regions.len() {
            run(
                &mut client,
                Command::SetChoice {
                    base: revision.clone(),
                    region,
                    choice: Choice::Hunks(vec![source.document.clone()]),
                },
            );
            revision = resync(&mut client).revision;
        }
        let chosen = resync(&mut client);
        assert_eq!(
            chosen.output.as_ref().unwrap().text,
            as_output(source),
            "{name}: choosing {:?} everywhere reproduces it",
            source.document
        );
    }

    drop(client);
    host.join().unwrap();
}

#[test]
fn basic_txt() {
    check_case(
        "basic_txt",
        Counts {
            differences: 5,
            conflicts: 0,
            unresolved: 0,
        },
    );
}

#[test]
fn conflict_txt() {
    check_case(
        "conflict_txt",
        Counts {
            differences: 5,
            conflicts: 2,
            unresolved: 2,
        },
    );
}

#[test]
fn two_way_txt() {
    check_case(
        "two_way_txt",
        Counts {
            differences: 4,
            conflicts: 0,
            unresolved: 4,
        },
    );
}

#[test]
fn long_txt() {
    check_case(
        "long_txt",
        Counts {
            differences: 17,
            conflicts: 4,
            unresolved: 4,
        },
    );
}

#[test]
fn moved_functions_rs() {
    check_case(
        "moved_functions_rs",
        Counts {
            differences: 6,
            conflicts: 0,
            unresolved: 0,
        },
    );
}

/// Moving a function shows as a removal and an insertion, so changing it in place on the other side conflicts with the removal.
#[test]
fn moved_and_changed_rs() {
    check_case(
        "moved_and_changed_rs",
        Counts {
            differences: 4,
            conflicts: 2,
            unresolved: 2,
        },
    );
}

#[test]
fn moved_functions_cpp() {
    check_case(
        "moved_functions_cpp",
        Counts {
            differences: 3,
            conflicts: 2,
            unresolved: 2,
        },
    );
}


/// Both sides move functions and change some of them, so the merge without alignments finds removals and insertions that conflict with the changes.
#[test]
fn moved_and_changed_cpp() {
    check_case(
        "moved_and_changed_cpp",
        Counts {
            differences: 8,
            conflicts: 2,
            unresolved: 2,
        },
    );
}


#[test]
fn swapped_rs() {
    check_case(
        "swapped_rs",
        Counts {
            differences: 5,
            conflicts: 0,
            unresolved: 5,
        },
    );
}

#[test]
fn whitespace_rs() {
    check_case(
        "whitespace_rs",
        Counts {
            differences: 4,
            conflicts: 2,
            unresolved: 2,
        },
    );
}

/// An empty base, a last line without a line break, and CRLF line ends on one side.
#[test]
fn edges_txt() {
    check_case(
        "edges_txt",
        Counts {
            differences: 1,
            conflicts: 1,
            unresolved: 1,
        },
    );
}

/// B and C each add a different function at the same place of A, which conflicts.
#[test]
fn manual_alignment_rs() {
    check_case(
        "manual_alignment_rs",
        Counts {
            differences: 1,
            conflicts: 1,
            unresolved: 1,
        },
    );
}

/// Marking B's function and C's function as the anchors and ranges of the Foo and Qux example, and applying, keeps both functions without a conflict.
#[test]
fn manual_alignment_separates_the_added_functions() {
    let (mut client, host) = start();
    open(&mut client, "manual_alignment_rs");
    let marks = [
        (Document::A, 4..4),
        (Document::A, 4..4),
        (Document::B, 4..8),
        (Document::B, 8..8),
        (Document::C, 4..4),
        (Document::C, 4..8),
    ];
    for (document, lines) in marks {
        run(&mut client, Command::MarkAlignment { document, lines });
    }
    let applied = run(&mut client, Command::ApplyAlignments { confirmed: false }).expect("Test: apply answers with a snapshot");
    assert_eq!(
        counts(&applied),
        Counts {
            differences: 2,
            conflicts: 0,
            unresolved: 0
        }
    );
    let expected_path = case_dir("manual_alignment_rs").join("aligned.rs");
    let expected = fs::read_to_string(&expected_path).unwrap_or_else(|error| panic!("{}: {error}", expected_path.display()));
    assert_eq!(applied.output.as_ref().unwrap().text, expected);

    drop(client);
    host.join().unwrap();
}

/// Heuristics Alignment finds the functions of `moved_and_changed_cpp` that the inset test below marks by hand, and applying them merges the same way.
#[test]
fn heuristics_alignment_finds_and_exchanges_the_moved_functions() {
    let (mut client, host) = start();
    open(&mut client, "moved_and_changed_cpp");
    let proposal = propose(&mut client, "C++", HeuristicOptions::default());
    let base = [11..21, 23..40, 42..53, 55..60, 62..71, 73..89, 91..101];
    let left = [18..28, 30..47, 78..89, 11..16, 49..58, 60..76, 91..101];
    let right = [93..103, 11..28, 50..61, 30..37, 39..48, 63..79, 81..91];
    let expected: Vec<Vec<std::ops::Range<usize>>> = (0..base.len())
        .map(|index| vec![base[index].clone(), left[index].clone(), right[index].clone()])
        .collect();
    let parts: Vec<Vec<std::ops::Range<usize>>> = proposal.matches.iter().map(|found| found.parts.clone()).collect();
    assert_eq!(parts, expected);
    assert_eq!(proposal.found, vec![7, 7, 7]);
    assert_eq!(proposal.blocked, 0);
    assert!(proposal.matches.iter().all(|found| found.stage == MatchStage::Signature));
    assert!(proposal.matches.iter().any(|found| found.moved));
    assert_eq!(proposal.matches[0].names, vec![Some("config::trim".to_string()); 3]);
    let applied = run(&mut client, Command::ApplyHeuristicAlignments { alignments: parts, confirmed: false, within: None }).expect("Test: apply answers with a snapshot");
    assert_eq!(
        counts(&applied),
        Counts {
            differences: 7,
            conflicts: 1,
            unresolved: 1
        }
    );
    assert!(applied.alignments.marks.iter().all(|mark| mark.heuristic));
    run(
        &mut client,
        Command::ChooseForUnresolved {
            base: applied.revision.clone(),
            document: Document::B,
        },
    );
    let merged = resync(&mut client);
    let expected_path = case_dir("moved_and_changed_cpp").join("aligned.cpp");
    let expected = fs::read_to_string(&expected_path).unwrap_or_else(|error| panic!("{}: {error}", expected_path.display()));
    assert_eq!(merged.output.as_ref().unwrap().text, expected);

    drop(client);
    host.join().unwrap();
}


/// A function the user marked is left to the user, and running again replaces only the heuristic marks.
#[test]
fn heuristics_alignment_leaves_marked_functions_alone() {
    let (mut client, host) = start();
    open(&mut client, "moved_and_changed_cpp");
    run(&mut client, Command::MarkAlignment { document: Document::A, lines: 11..21 });
    let proposal = propose(&mut client, "C++", HeuristicOptions::default());
    assert_eq!(proposal.blocked, 1);
    assert_eq!(proposal.matches.len(), 6);
    let parts: Vec<Vec<std::ops::Range<usize>>> = proposal.matches.iter().map(|found| found.parts.clone()).collect();
    run(&mut client, Command::ApplyHeuristicAlignments { alignments: parts.clone(), confirmed: false, within: None });
    let again = run(&mut client, Command::ApplyHeuristicAlignments { alignments: parts[..2].to_vec(), confirmed: true, within: None }).expect("Test: apply answers with a snapshot");
    let marks = &again.alignments.marks;
    assert_eq!(marks.iter().filter(|mark| mark.heuristic).count(), 6);
    let manual: Vec<std::ops::Range<usize>> = marks
        .iter()
        .filter(|mark| !mark.heuristic)
        .map(|mark| mark.lines.clone())
        .collect();
    assert_eq!(manual, vec![11..21]);

    drop(client);
    host.join().unwrap();
}


/// The sources of a snapshot are reproduced by choosing them everywhere. Choosing from the last region up keeps the indexes valid when a choice joins neighboring regions.
fn assert_sources_reproduced(client: &mut Client, snapshot: &Snapshot) {
    for source in &snapshot.documents {
        let before = resync(client);
        let mut revision = before.revision.clone();
        for region in (0..before.comparison.regions.len()).rev() {
            run(
                client,
                Command::SetChoice {
                    base: revision.clone(),
                    region,
                    choice: Choice::Hunks(vec![source.document.clone()]),
                },
            );
            revision = resync(client).revision;
        }
        assert_eq!(resync(client).output.as_ref().unwrap().text, as_output(source), "choosing {:?} everywhere reproduces it", source.document);
    }
}


/// B moves `helper` above `process`, B and C both add the same block to it, and C changes a loop. Without alignments the added block is merged twice.
#[test]
fn blocks_in_function_cpp() {
    check_case(
        "blocks_in_function_cpp",
        Counts {
            differences: 5,
            conflicts: 0,
            unresolved: 0,
        },
    );
}


/// With blocks on, the blocks of `process` are proposed inside it, the block B and C added is anchored in A, and applying merges the moved function with its blocks.
#[test]
fn heuristics_alignment_aligns_the_blocks_inside_the_functions() {
    let (mut client, host) = start();
    open(&mut client, "blocks_in_function_cpp");
    let without = propose(&mut client, "C++", HeuristicOptions::default());
    assert_eq!(without.matches.len(), 2);
    assert!(without.matches.iter().all(|found| found.parent.is_none()));
    let proposal = propose(&mut client, "C++", HeuristicOptions { blocks: true, ..HeuristicOptions::default() });
    let shape: Vec<(EntityKind, Option<usize>)> = proposal.matches.iter().map(|found| (found.kind.clone(), found.parent)).collect();
    assert_eq!(
        shape,
        [(EntityKind::Function, None), (EntityKind::Block, Some(0)), (EntityKind::Block, Some(0)), (EntityKind::Block, Some(0)), (EntityKind::Block, Some(0)), (EntityKind::Function, None)]
    );
    assert_eq!(proposal.found, vec![5, 6, 6]);
    assert_eq!(proposal.matches[3].names, vec![None, Some("if(total<0)".to_string()), Some("if(total<0)".to_string())]);
    assert_eq!(proposal.matches[3].parts, vec![13..13, 18..22, 13..17]);
    let parts: Vec<Vec<std::ops::Range<usize>>> = proposal.matches.iter().map(|found| found.parts.clone()).collect();
    let applied = run(&mut client, Command::ApplyHeuristicAlignments { alignments: parts, confirmed: false, within: None }).expect("Test: apply answers with a snapshot");
    assert_eq!(
        counts(&applied),
        Counts {
            differences: 3,
            conflicts: 0,
            unresolved: 0
        }
    );
    assert_eq!(applied.alignments.marks.iter().filter(|mark| mark.heuristic).count(), 18);
    run(
        &mut client,
        Command::ChooseForUnresolved {
            base: applied.revision.clone(),
            document: Document::B,
        },
    );
    let expected_path = case_dir("blocks_in_function_cpp").join("aligned.cpp");
    let expected = fs::read_to_string(&expected_path).unwrap_or_else(|error| panic!("{}: {error}", expected_path.display()));
    assert_eq!(resync(&mut client).output.as_ref().unwrap().text, expected);

    drop(client);
    host.join().unwrap();
}


/// Marking `process` by hand and aligning inside it pairs its blocks, keeps the marks outside it, and the sources are reproduced by choosing them.
#[test]
fn heuristics_alignment_inside_a_marked_function_aligns_its_blocks() {
    let (mut client, host) = start();
    open(&mut client, "blocks_in_function_cpp");
    let process = propose(&mut client, "C++", HeuristicOptions::default()).matches[0].parts.clone();
    for (document, lines) in [Document::A, Document::B, Document::C].into_iter().zip(process.iter()) {
        run(&mut client, Command::MarkAlignment { document, lines: lines.clone() });
    }
    run(&mut client, Command::ApplyAlignments { confirmed: false });

    let outside = propose_within(&mut client, "C++", HeuristicOptions::default(), Some(1));
    assert_eq!(outside.matches.len(), 4);
    assert!(outside.matches.iter().all(|found| found.kind == EntityKind::Block && found.parent.is_none()));
    assert_eq!(outside.found, vec![3, 4, 4]);
    let parts: Vec<Vec<std::ops::Range<usize>>> = outside.matches.iter().map(|found| found.parts.clone()).collect();
    assert!(parts.iter().all(|parts| parts.iter().zip(process.iter()).all(|(part, process)| process.start <= part.start && part.end <= process.end)));
    let applied = run(&mut client, Command::ApplyHeuristicAlignments { alignments: parts, confirmed: false, within: Some(1) }).expect("Test: apply answers with a snapshot");
    assert_eq!(applied.alignments.marks.iter().filter(|mark| mark.heuristic).count(), 12);
    assert_eq!(applied.alignments.marks.iter().filter(|mark| !mark.heuristic).count(), 3);
    let current = resync(&mut client);
    assert_sources_reproduced(&mut client, &current);

    let whole = propose(&mut client, "C++", HeuristicOptions::default());
    assert_eq!(whole.blocked, 1);
    assert_eq!(whole.matches.len(), 1);
    let parts: Vec<Vec<std::ops::Range<usize>>> = whole.matches.iter().map(|found| found.parts.clone()).collect();
    let replaced = run(&mut client, Command::ApplyHeuristicAlignments { alignments: parts, confirmed: true, within: None }).expect("Test: apply answers with a snapshot");
    assert_eq!(replaced.alignments.marks.iter().filter(|mark| mark.heuristic).count(), 3);

    drop(client);
    host.join().unwrap();
}


#[test]
fn heuristics_alignment_inside_an_alignment_that_is_not_applied_fails() {
    let (mut client, host) = start();
    open(&mut client, "blocks_in_function_cpp");
    let id = client
        .send(Command::ProposeHeuristicAlignments {
            language: "C++".to_string(),
            options: HeuristicOptions::default(),
            within: Some(7),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match client.try_recv().unwrap() {
            Some(HostMessage::Response(response)) if response.id == id => {
                let error = response.result.unwrap_err();
                assert_eq!(error.kind, ErrorKind::InvalidRequest);
                assert_eq!(error.message, "Alignment 7 is not applied");
                break;
            }
            Some(_) => {}
            None => {
                assert!(Instant::now() < deadline, "no response from the host");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    drop(client);
    host.join().unwrap();
}


#[test]
fn heuristics_alignment_refuses_languages_it_can_not_parse() {
    let (mut client, host) = start();
    open(&mut client, "moved_and_changed_cpp");
    let id = client
        .send(Command::ProposeHeuristicAlignments {
            language: "Plain text".to_string(),
            options: HeuristicOptions::default(),
            within: None,
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match client.try_recv().unwrap() {
            Some(HostMessage::Response(response)) if response.id == id => {
                assert_eq!(response.result.unwrap_err().message, "Heuristics Alignment can't parse Plain text yet");
                break;
            }
            Some(_) => {}
            None => {
                assert!(Instant::now() < deadline, "no response from the host");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    drop(client);
    host.join().unwrap();
}


/// The functions of `moved_and_changed_cpp`, marked in the order of the base. B and C hold them in other orders, so the alignments are insets: the functions are exchanged between the places they have in B and C, and the blank lines between them stay where they are. Every change of both sides merges except the one both made to `toLower`.
#[test]
fn insets_exchange_the_moved_functions_without_extra_blank_lines() {
    let (mut client, host) = start();
    open(&mut client, "moved_and_changed_cpp");
    let base = [11..21, 23..40, 42..53, 55..60, 62..71, 73..89, 91..101];
    let left = [18..28, 30..47, 78..89, 11..16, 49..58, 60..76, 91..101];
    let right = [93..103, 11..28, 50..61, 30..37, 39..48, 63..79, 81..91];
    for (document, functions) in [(Document::A, base), (Document::B, left), (Document::C, right)] {
        for lines in functions {
            run(&mut client, Command::MarkAlignment { document: document.clone(), lines });
        }
    }
    let applied = run(&mut client, Command::ApplyAlignments { confirmed: false }).expect("Test: apply answers with a snapshot");
    assert_eq!(
        counts(&applied),
        Counts {
            differences: 7,
            conflicts: 1,
            unresolved: 1
        }
    );
    run(
        &mut client,
        Command::ChooseForUnresolved {
            base: applied.revision.clone(),
            document: Document::B,
        },
    );
    let merged = resync(&mut client);
    let text = &merged.output.as_ref().unwrap().text;
    let expected_path = case_dir("moved_and_changed_cpp").join("aligned.cpp");
    let expected = fs::read_to_string(&expected_path).unwrap_or_else(|error| panic!("{}: {error}", expected_path.display()));
    assert_eq!(*text, expected);
    assert!(!text.contains("\n\n\n\n"), "no more than two blank lines in a row");

    drop(client);
    host.join().unwrap();
}


#[test]
fn every_case_has_a_test() {
    let mut cases: Vec<String> = fs::read_dir(case_dir(""))
        .expect("Test: merge data folder")
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().to_str().map(|name| name.to_string()))
        .collect();
    cases.sort();
    assert_eq!(
        cases,
        [
            "basic_txt",
            "blocks_in_function_cpp",
            "conflict_txt",
            "edges_txt",
            "long_txt",
            "manual_alignment_rs",
            "moved_and_changed_cpp",
            "moved_and_changed_rs",
            "moved_functions_cpp",
            "moved_functions_rs",
            "swapped_rs",
            "two_way_txt",
            "whitespace_rs"
        ]
    );
}
