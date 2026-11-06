// Merge cases in `test_data/merge/`, run through the host the way the GUI drives it. A case folder holds `base`, `left` and `right` (3-way) or `left` and `right` (2-way), plus `expected`: the auto-merge with B chosen for every conflict left unresolved.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use kdiff4_protocol::engine::{HunkChange, MergeStatus, RegionStatus};
use kdiff4_protocol::{Choice, Client, Command, Document, DocumentText, Event, HostMessage, OpenFiles, Snapshot, in_process};

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
            "conflict_txt",
            "edges_txt",
            "long_txt",
            "manual_alignment_rs",
            "moved_and_changed_rs",
            "moved_functions_rs",
            "swapped_rs",
            "two_way_txt",
            "whitespace_rs"
        ]
    );
}
