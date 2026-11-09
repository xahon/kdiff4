use std::fs;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use kdiff4_protocol::engine::{ComparisonOptions, MergeStatus, RegionStatus};
use kdiff4_protocol::{
    Alignments, Change, Choice, Client, Command, CurrentRegion, Delta, Document, EolChoice, EolStyle, ErrorKind, Event, HostMessage, Labels, MarkStatus, Navigation, OpenFiles, OutputEdit, Response,
    Revision, Snapshot, in_process,
};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kdiff4-core-host-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, text).unwrap();
    path
}

fn start() -> (Client, JoinHandle<()>) {
    let (client, endpoint) = in_process();
    let host = kdiff4_core::spawn_host(endpoint).unwrap();
    (client, host)
}

fn next_message(client: &Client) -> HostMessage {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(message) = client.try_recv().unwrap() {
            return message;
        }
        assert!(Instant::now() < deadline, "no message from the host");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn expect_snapshot(client: &Client) -> Snapshot {
    match next_message(client) {
        HostMessage::Event(Event::Snapshot(snapshot)) => snapshot,
        other => panic!("expected a snapshot, got {other:?}"),
    }
}

fn expect_delta(client: &Client) -> Delta {
    match next_message(client) {
        HostMessage::Event(Event::Delta(delta)) => delta,
        other => panic!("expected a delta, got {other:?}"),
    }
}

fn expect_ok(client: &Client, id: kdiff4_protocol::RequestId) {
    assert_eq!(next_message(client), HostMessage::Response(Response { id, result: Ok(()) }));
}

#[test]
fn open_sends_snapshot_then_ok_response() {
    let dir = temp_dir("open");
    let a = write(&dir, "a.txt", "base\n");
    let b = write(&dir, "b.txt", "left\n");
    let c = write(&dir, "c.txt", "right\n");
    let output = write(&dir, "out.txt", "merged\n");
    let (mut client, host) = start();

    let labels = Labels {
        a: Some("Base".to_string()),
        b: None,
        c: None,
    };
    let id = client
        .send(Command::Open(OpenFiles {
            a,
            b,
            c: Some(c),
            output: Some(output.clone()),
            labels,
        }))
        .unwrap();
    let snapshot = expect_snapshot(&client);
    let lines: Vec<(Document, &str)> = snapshot.documents.iter().map(|text| (text.document.clone(), text.line(0))).collect();
    assert_eq!(lines, [(Document::A, "base"), (Document::B, "left"), (Document::C, "right")]);
    assert_eq!(snapshot.documents[0].label.as_deref(), Some("Base"));
    let d = snapshot.output.unwrap();
    assert_eq!(d.path, Some(output));
    assert_eq!(d.text, "", "an existing output file is not loaded; D is the merge, here one unresolved conflict");
    assert!(snapshot.merge[0].is_unresolved());
    assert_eq!(snapshot.engine.id, kdiff4_core::BUILTIN_ENGINE_ID);
    let statuses: Vec<RegionStatus> = snapshot.comparison.regions.iter().map(|region| region.status.clone()).collect();
    assert_eq!(statuses, [RegionStatus::Merge(MergeStatus::Conflict)]);
    expect_ok(&client, id);

    drop(client);
    host.join().unwrap();
}

#[test]
fn open_missing_file_sends_error_response_only() {
    let dir = temp_dir("missing");
    let a = write(&dir, "a.txt", "a");
    let (mut client, host) = start();

    let id = client
        .send(Command::Open(OpenFiles {
            a,
            b: dir.join("missing.txt"),
            ..OpenFiles::default()
        }))
        .unwrap();
    let HostMessage::Response(response) = next_message(&client) else {
        panic!("expected a response");
    };
    assert_eq!(response.id, id);
    let error = response.result.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Io);
    assert!(error.message.contains("missing.txt"), "{}", error.message);
    assert_eq!(client.try_recv(), Ok(None));

    drop(client);
    host.join().unwrap();
}

#[test]
fn commands_before_open_are_rejected() {
    let (mut client, host) = start();
    let id = client.send(Command::Save).unwrap();
    let HostMessage::Response(response) = next_message(&client) else {
        panic!("expected a response");
    };
    assert_eq!(response.id, id);
    assert_eq!(response.result.unwrap_err().kind, ErrorKind::InvalidRequest);
    drop(client);
    host.join().unwrap();
}

#[test]
fn reload_asks_before_dropping_work_then_sends_a_snapshot() {
    let dir = temp_dir("reload-all");
    let a = write(&dir, "a.txt", "a\n");
    let b = write(&dir, "b.txt", "b\n");
    let c = write(&dir, "c.txt", "c\n");
    let (mut client, host) = start();

    let id = client
        .send(Command::Open(OpenFiles {
            a,
            b,
            c: Some(c),
            ..OpenFiles::default()
        }))
        .unwrap();
    let snapshot = expect_snapshot(&client);
    expect_ok(&client, id);
    let id = client
        .send(Command::SetChoice {
            base: snapshot.revision.clone(),
            region: 0,
            choice: Choice::Hunks(vec![Document::B]),
        })
        .unwrap();
    expect_delta(&client);
    expect_ok(&client, id);

    write(&dir, "c.txt", "a\n");
    let id = client.send(Command::Reload { confirmed: false }).unwrap();
    let HostMessage::Response(response) = next_message(&client) else {
        panic!("expected a response");
    };
    assert_eq!(response.id, id);
    assert_eq!(response.result.unwrap_err().kind, ErrorKind::NeedsConfirmation);

    let id = client.send(Command::Reload { confirmed: true }).unwrap();
    let snapshot = expect_snapshot(&client);
    expect_ok(&client, id);
    assert_eq!(snapshot.documents[2].line(0), "a");
    assert_eq!(snapshot.output.unwrap().text, "b\n", "C now equals A, so B is taken without a choice");

    drop(client);
    host.join().unwrap();
}

#[test]
fn edit_save_and_reload_go_through_deltas() {
    let dir = temp_dir("round-trip");
    let a = write(&dir, "a.txt", "a\r\n");
    let b = write(&dir, "b.txt", "b\r\n");
    let c = write(&dir, "c.txt", "c\r\n");
    let (mut client, host) = start();

    let id = client
        .send(Command::Open(OpenFiles {
            a,
            b,
            c: Some(c),
            ..OpenFiles::default()
        }))
        .unwrap();
    let snapshot = expect_snapshot(&client);
    expect_ok(&client, id);
    assert_eq!(snapshot.output.as_ref().unwrap().path, None);
    assert!(snapshot.merge[0].is_unresolved());

    let target = dir.join("saved.txt");
    let id = client.send(Command::SaveAs(target.clone())).unwrap();
    let HostMessage::Response(response) = next_message(&client) else {
        panic!("expected a response");
    };
    assert_eq!(response.id, id);
    assert!(response.result.unwrap_err().message.contains("unresolved"));

    let id = client
        .send(Command::EditOutput(OutputEdit {
            base: snapshot.revision.clone(),
            range: 0..0,
            text: "x\n".to_string(),
            region: None,
        }))
        .unwrap();
    let delta = expect_delta(&client);
    assert_eq!((delta.revision.clone(), delta.cause.clone()), (Revision(2), Some(id.clone())));
    let Change::OutputEdited { range, text, modified, merge } = delta.change else {
        panic!("expected an output change");
    };
    assert_eq!((range, text.as_str(), modified), (0..0, "x\n", true));
    assert!(merge.merge[0].edited);
    expect_ok(&client, id);

    let stale = client
        .send(Command::EditOutput(OutputEdit {
            base: Revision(1),
            range: 0..0,
            text: "y".to_string(),
            region: None,
        }))
        .unwrap();
    let HostMessage::Response(response) = next_message(&client) else {
        panic!("expected a response");
    };
    assert_eq!(response.id, stale);
    assert!(response.result.is_err());

    let id = client.send(Command::SaveAs(target.clone())).unwrap();
    assert_eq!(expect_delta(&client).change, Change::OutputSaved(target.clone()));
    expect_ok(&client, id);
    assert_eq!(fs::read(&target).unwrap(), b"x\r\n");

    let id = client.send(Command::Undo).unwrap();
    let Change::OutputEdited { range, text, modified, .. } = expect_delta(&client).change else {
        panic!("expected an output change");
    };
    assert_eq!((range, text.as_str(), modified), (0..2, "", true));
    expect_ok(&client, id);
    let id = client.send(Command::Redo).unwrap();
    let Change::OutputEdited { modified, .. } = expect_delta(&client).change else {
        panic!("expected an output change");
    };
    assert!(!modified);
    expect_ok(&client, id);

    let id = client
        .send(Command::SetChoice {
            base: Revision(5),
            region: 0,
            choice: Choice::Hunks(vec![Document::C]),
        })
        .unwrap();
    let delta = expect_delta(&client);
    assert!(matches!(delta.change, Change::OutputEdited { ref text, .. } if text == "c\n"));
    expect_ok(&client, id);
    let id = client
        .send(Command::ChooseForUnresolved {
            base: Revision(6),
            document: Document::B,
        })
        .unwrap();
    expect_ok(&client, id);

    let new_b = write(&dir, "b2.txt", "b2\n");
    let id = client
        .send(Command::LoadSource {
            document: Document::B,
            path: new_b,
        })
        .unwrap();
    let snapshot = expect_snapshot(&client);
    assert_eq!(snapshot.revision, Revision(7));
    assert_eq!(snapshot.documents[1].line(0), "b2");
    assert_eq!(snapshot.comparison.regions.len(), 1);
    assert_eq!(snapshot.comparison.regions[0].status, RegionStatus::Merge(MergeStatus::Conflict));
    assert_eq!(snapshot.output.as_ref().unwrap().text, "");
    expect_ok(&client, id);

    let id = client.send(Command::Undo).unwrap();
    expect_ok(&client, id);

    let id = client.send(Command::Resync).unwrap();
    let snapshot = expect_snapshot(&client);
    assert_eq!(snapshot.revision, Revision(7));
    expect_ok(&client, id);

    drop(client);
    host.join().unwrap();
}

#[test]
fn a_line_ending_conflict_is_settled_by_choosing_a_style_before_saving() {
    let dir = temp_dir("eol-conflict");
    let a = write(&dir, "a.txt", "a\r\n");
    let b = write(&dir, "b.txt", "a\n");
    let c = write(&dir, "c.txt", "a\n");
    let target = dir.join("out.txt");
    let (mut client, host) = start();

    let id = client
        .send(Command::Open(OpenFiles {
            a,
            b,
            c: Some(c),
            output: Some(target.clone()),
            ..OpenFiles::default()
        }))
        .unwrap();
    let snapshot = expect_snapshot(&client);
    expect_ok(&client, id);
    assert_eq!(snapshot.documents.iter().map(|document| document.eol).collect::<Vec<_>>(), [Some(EolStyle::CrLf), Some(EolStyle::Lf), Some(EolStyle::Lf)]);
    assert_eq!(snapshot.output.as_ref().unwrap().eol, EolChoice::Conflict);

    let id = client.send(Command::Save).unwrap();
    let HostMessage::Response(response) = next_message(&client) else {
        panic!("expected a response");
    };
    assert_eq!(response.id, id);
    assert_eq!(response.result.unwrap_err().kind, ErrorKind::EolConflict);
    assert!(!target.exists());

    let id = client.send(Command::SetOutputEol(EolStyle::CrLf)).unwrap();
    let delta = expect_delta(&client);
    assert_eq!((delta.revision.clone(), delta.cause.clone(), delta.change), (Revision(2), Some(id.clone()), Change::OutputEolChanged(EolChoice::Style(EolStyle::CrLf))));
    expect_ok(&client, id);

    let id = client.send(Command::Save).unwrap();
    assert_eq!(expect_delta(&client).change, Change::OutputSaved(target.clone()));
    expect_ok(&client, id);
    assert_eq!(fs::read(&target).unwrap(), b"a\r\n");

    drop(client);
    host.join().unwrap();
}

#[test]
fn navigation_broadcasts_the_current_region_without_a_new_revision() {
    let dir = temp_dir("navigate");
    let a = write(&dir, "a.txt", "s\na\n");
    let b = write(&dir, "b.txt", "s\nb\n");
    let c = write(&dir, "c.txt", "s\nc\n");
    let (mut client, host) = start();
    let id = client
        .send(Command::Open(OpenFiles {
            a,
            b,
            c: Some(c),
            ..OpenFiles::default()
        }))
        .unwrap();
    let snapshot = expect_snapshot(&client);
    expect_ok(&client, id);

    let id = client
        .send(Command::Navigate {
            base: snapshot.revision.clone(),
            from: None,
            to: Navigation::NextConflict,
            include_alignments: false,
        })
        .unwrap();
    assert_eq!(
        next_message(&client),
        HostMessage::Event(Event::CurrentRegion(CurrentRegion {
            revision: snapshot.revision.clone(),
            region: Some(1),
            cause: Some(id.clone())
        }))
    );
    expect_ok(&client, id);

    let id = client
        .send(Command::Navigate {
            base: snapshot.revision.clone(),
            from: None,
            to: Navigation::NextConflict,
            include_alignments: false,
        })
        .unwrap();
    expect_ok(&client, id);

    let options = ComparisonOptions {
        try_hard: true,
        ..ComparisonOptions::default()
    };
    let id = client.send(Command::SetComparisonOptions(options.clone())).unwrap();
    let snapshot = expect_snapshot(&client);
    assert_eq!(snapshot.options, options);
    assert_eq!(snapshot.current_region, None);
    expect_ok(&client, id);

    let options = ComparisonOptions {
        ignore_whitespace: true,
        ..options
    };
    let id = client.send(Command::SetComparisonOptions(options.clone())).unwrap();
    let reflowed = expect_snapshot(&client);
    assert_eq!(reflowed.options, options);
    assert_eq!(reflowed.comparison, snapshot.comparison);
    assert_eq!(reflowed.output, snapshot.output);
    assert_eq!(reflowed.merge, snapshot.merge);
    expect_ok(&client, id);

    drop(client);
    host.join().unwrap();
}

#[test]
fn a_mark_can_be_stretched_through_the_protocol() {
    let dir = temp_dir("stretch");
    let a = write(&dir, "a.txt", "1
2
3
4
");
    let b = write(&dir, "b.txt", "1
2
3
4
");
    let (mut client, host) = start();
    let id = client.send(Command::Open(OpenFiles { a, b, ..OpenFiles::default() })).unwrap();
    expect_snapshot(&client);
    expect_ok(&client, id);
    let id = client.send(Command::MarkAlignment { document: Document::A, lines: 1..2 }).unwrap();
    expect_delta(&client);
    expect_ok(&client, id);

    let id = client.send(Command::SetAlignmentMarkLines { document: Document::A, number: 1, lines: 1..4 }).unwrap();
    let delta = expect_delta(&client);
    let Change::AlignmentsChanged(alignments) = delta.change else {
        panic!("expected the marks")
    };
    assert_eq!(alignments.marks.iter().map(|mark| mark.lines.clone()).collect::<Vec<_>>(), [1..4]);
    expect_ok(&client, id);

    let id = client.send(Command::SetAlignmentMarkLines { document: Document::A, number: 1, lines: 1..9 }).unwrap();
    let HostMessage::Response(response) = next_message(&client) else {
        panic!("expected a response")
    };
    assert_eq!(response.id, id);
    assert_eq!(response.result.unwrap_err().kind, ErrorKind::InvalidRequest);

    drop(client);
    host.join().unwrap();
}


#[test]
fn the_function_around_a_line_is_marked_through_the_protocol() {
    let dir = temp_dir("function-mark");
    let a = write(&dir, "a.cpp", "int x;

int f()
{
    return 0;
}
");
    let b = write(&dir, "b.cpp", "int x;
");
    let (mut client, host) = start();
    let id = client.send(Command::Open(OpenFiles { a, b, ..OpenFiles::default() })).unwrap();
    expect_snapshot(&client);
    expect_ok(&client, id);

    let id = client.send(Command::MarkFunctionAlignment { document: Document::A, line: 5, language: "C++".to_string() }).unwrap();
    let delta = expect_delta(&client);
    let Change::AlignmentsChanged(alignments) = delta.change else {
        panic!("expected the marks")
    };
    assert_eq!(alignments.marks.iter().map(|mark| mark.lines.clone()).collect::<Vec<_>>(), [2..6]);
    expect_ok(&client, id);

    for (line, language) in [(0, "C++"), (2, "Plain Text")] {
        let id = client.send(Command::MarkFunctionAlignment { document: Document::A, line, language: language.to_string() }).unwrap();
        let HostMessage::Response(response) = next_message(&client) else {
            panic!("expected a response")
        };
        assert_eq!(response.id, id);
        assert_eq!(response.result.unwrap_err().kind, ErrorKind::InvalidRequest);
    }

    drop(client);
    host.join().unwrap();
}


#[test]
fn alignment_marks_arrive_as_deltas_and_apply_answers_with_a_snapshot() {
    let dir = temp_dir("alignment");
    let a = write(&dir, "a.txt", "Bar\nBaz\n");
    let b = write(&dir, "b.txt", "Bar\nFoo\nBaz\n");
    let c = write(&dir, "c.txt", "Bar\nQux\nBaz\n");
    let (mut client, host) = start();
    let id = client
        .send(Command::Open(OpenFiles {
            a,
            b,
            c: Some(c),
            ..OpenFiles::default()
        }))
        .unwrap();
    let snapshot = expect_snapshot(&client);
    assert_eq!(snapshot.alignments, Alignments::default());
    expect_ok(&client, id);

    let marks = [
        (Document::A, 1..1),
        (Document::A, 1..1),
        (Document::B, 1..2),
        (Document::B, 2..2),
        (Document::C, 1..1),
        (Document::C, 1..2),
    ];
    for (index, (document, lines)) in marks.into_iter().enumerate() {
        let id = client.send(Command::MarkAlignment { document, lines }).unwrap();
        let delta = expect_delta(&client);
        assert_eq!(delta.revision, Revision(snapshot.revision.0 + index as u64 + 1));
        assert_eq!(delta.cause, Some(id.clone()));
        let Change::AlignmentsChanged(alignments) = delta.change else {
            panic!("expected the marks")
        };
        assert_eq!(alignments.marks.len(), index + 1);
        assert!(alignments.changed || index < 5);
        expect_ok(&client, id);
    }

    let id = client
        .send(Command::SetChoice {
            base: Revision(snapshot.revision.0 + 6),
            region: 1,
            choice: Choice::Hunks(vec![Document::B]),
        })
        .unwrap();
    expect_delta(&client);
    expect_ok(&client, id);

    let id = client.send(Command::ApplyAlignments { confirmed: false }).unwrap();
    let HostMessage::Response(response) = next_message(&client) else {
        panic!("expected a response")
    };
    assert_eq!(response.id, id);
    assert_eq!(response.result.unwrap_err().kind, ErrorKind::NeedsConfirmation);

    let id = client.send(Command::ApplyAlignments { confirmed: true }).unwrap();
    let snapshot = expect_snapshot(&client);
    let statuses: Vec<RegionStatus> = snapshot.comparison.regions.iter().map(|region| region.status.clone()).collect();
    assert_eq!(
        statuses,
        [
            RegionStatus::Merge(MergeStatus::Unchanged),
            RegionStatus::Merge(MergeStatus::ChangedInB),
            RegionStatus::Merge(MergeStatus::ChangedInC),
            RegionStatus::Merge(MergeStatus::Unchanged)
        ]
    );
    assert_eq!(snapshot.output.unwrap().text, "Bar\nFoo\nQux\nBaz\n");
    assert!(!snapshot.alignments.changed);
    assert!(snapshot.alignments.marks.iter().all(|mark| mark.status == MarkStatus::Applied));
    expect_ok(&client, id);

    let id = client.send(Command::ApplyAlignments { confirmed: false }).unwrap();
    expect_ok(&client, id);

    let id = client.send(Command::ClearAlignmentMarks).unwrap();
    let delta = expect_delta(&client);
    assert_eq!(delta.change, Change::AlignmentsChanged(Alignments { marks: Vec::new(), changed: true, ..Alignments::default() }));
    expect_ok(&client, id);
    let id = client.send(Command::ClearAlignmentMarks).unwrap();
    expect_ok(&client, id);

    drop(client);
    host.join().unwrap();
}


#[test]
fn insets_ask_where_to_show_them_and_can_be_placed_again() {
    let dir = temp_dir("inset");
    let a = write(&dir, "a.txt", "a\nb\n");
    let b = write(&dir, "b.txt", "b\na\n");
    let c = write(&dir, "c.txt", "a\nb\n");
    let (mut client, host) = start();
    let id = client
        .send(Command::Open(OpenFiles {
            a,
            b,
            c: Some(c),
            ..OpenFiles::default()
        }))
        .unwrap();
    expect_snapshot(&client);
    expect_ok(&client, id);

    let marks = [
        (Document::A, 0..1),
        (Document::A, 1..2),
        (Document::B, 1..2),
        (Document::B, 0..1),
        (Document::C, 0..1),
        (Document::C, 1..2),
    ];
    let mut asked = Vec::new();
    for (document, lines) in marks {
        let id = client.send(Command::MarkAlignment { document, lines }).unwrap();
        let Change::AlignmentsChanged(alignments) = expect_delta(&client).change else {
            panic!("expected the marks")
        };
        asked.push(alignments.ask);
        expect_ok(&client, id);
    }
    assert_eq!(asked, [None, None, None, None, None, Some(2)]);

    let id = client.send(Command::PlaceAlignment { number: 2, slot: 1 }).unwrap();
    let Change::AlignmentsChanged(alignments) = expect_delta(&client).change else {
        panic!("expected the marks")
    };
    assert_eq!(alignments.ask, None);
    assert_eq!(alignments.insets.len(), 2);
    assert_eq!(alignments.insets.iter().find(|choice| choice.number == 2).unwrap().current, 1);
    assert!(alignments.changed);
    expect_ok(&client, id);

    let id = client.send(Command::PlaceAlignment { number: 7, slot: 0 }).unwrap();
    let HostMessage::Response(response) = next_message(&client) else {
        panic!("expected a response")
    };
    assert_eq!(response.id, id);
    assert_eq!(response.result.unwrap_err().kind, ErrorKind::InvalidRequest);

    let id = client.send(Command::ApplyAlignments { confirmed: true }).unwrap();
    let snapshot = expect_snapshot(&client);
    assert_eq!(snapshot.output.unwrap().text, "a\nb\n");
    let hunks: Vec<_> = snapshot.comparison.regions.iter().map(|region| region.hunks.clone()).collect();
    assert_eq!(hunks, [vec![0..1, 1..2, 0..1], vec![1..2, 0..1, 1..2]]);
    assert!(snapshot.alignments.marks.iter().filter(|mark| mark.is_moved()).all(|mark| mark.document == Document::B));
    expect_ok(&client, id);

    drop(client);
    host.join().unwrap();
}

#[test]
fn fold_ranges_are_found_for_every_document_asked_without_a_new_revision() {
    let dir = temp_dir("fold-ranges");
    let a = write(&dir, "a.cpp", "int f() {\n    return 0;\n}\n");
    let b = write(&dir, "b.cpp", "int g() {\n    int x = 1;\n    return x;\n}\n");
    let (mut client, host) = start();

    let id = client
        .send(Command::Open(OpenFiles {
            a,
            b,
            output: Some(dir.join("out.cpp")),
            ..OpenFiles::default()
        }))
        .unwrap();
    let snapshot = expect_snapshot(&client);
    expect_ok(&client, id);

    let id = client
        .send(Command::FindFoldRanges {
            language: "C++".to_string(),
            documents: vec![Document::B, Document::C, Document::D],
        })
        .unwrap();
    let HostMessage::Event(Event::FoldRanges(found)) = next_message(&client) else {
        panic!("expected fold ranges");
    };
    expect_ok(&client, id.clone());
    assert_eq!(found.cause, Some(id));
    assert_eq!(found.revision, snapshot.revision);
    assert_eq!(found.documents.len(), 2);
    assert_eq!(found.documents[0], (Document::B, vec![0..4]));
    assert_eq!(found.documents[1].0, Document::D);

    drop(client);
    host.join().unwrap();
}
