use std::fs;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use kdiff4_protocol::{Change, Client, Command, Delta, Document, ErrorKind, Event, HostMessage, Labels, OpenFiles, OutputEdit, Response, Revision, Snapshot, in_process};


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

    let labels = Labels { a: Some("Base".to_string()), b: None, c: None };
    let id = client.send(Command::Open(OpenFiles { a, b, c: Some(c), output: Some(output.clone()), labels })).unwrap();
    let snapshot = expect_snapshot(&client);
    let lines: Vec<(Document, &str)> = snapshot.documents
        .iter()
        .map(|text| (text.document.clone(), text.line(0)))
        .collect();
    assert_eq!(lines, [(Document::A, "base"), (Document::B, "left"), (Document::C, "right")]);
    assert_eq!(snapshot.documents[0].label.as_deref(), Some("Base"));
    let d = snapshot.output.unwrap();
    assert_eq!(d.path, Some(output));
    assert_eq!(d.text, "merged\n");
    expect_ok(&client, id);

    drop(client);
    host.join().unwrap();
}


#[test]
fn open_missing_file_sends_error_response_only() {
    let dir = temp_dir("missing");
    let a = write(&dir, "a.txt", "a");
    let (mut client, host) = start();

    let id = client.send(Command::Open(OpenFiles { a, b: dir.join("missing.txt"), ..OpenFiles::default() })).unwrap();
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
fn edit_save_and_reload_go_through_deltas() {
    let dir = temp_dir("round-trip");
    let a = write(&dir, "a.txt", "a\r\n");
    let b = write(&dir, "b.txt", "b\n");
    let c = write(&dir, "c.txt", "c\n");
    let (mut client, host) = start();

    let id = client.send(Command::Open(OpenFiles { a, b, c: Some(c), ..OpenFiles::default() })).unwrap();
    let snapshot = expect_snapshot(&client);
    expect_ok(&client, id);
    assert_eq!(snapshot.output.as_ref().unwrap().path, None);

    let id = client.send(Command::EditOutput(OutputEdit { base: snapshot.revision.clone(), range: 0..0, text: "x\n".to_string() })).unwrap();
    let delta = expect_delta(&client);
    assert_eq!(delta, Delta { revision: Revision(2), cause: Some(id.clone()), change: Change::OutputEdited { range: 0..0, text: "x\n".to_string(), modified: true } });
    expect_ok(&client, id);

    let stale = client.send(Command::EditOutput(OutputEdit { base: Revision(1), range: 0..0, text: "y".to_string() })).unwrap();
    let HostMessage::Response(response) = next_message(&client) else {
        panic!("expected a response");
    };
    assert_eq!(response.id, stale);
    assert!(response.result.is_err());

    let target = dir.join("saved.txt");
    let id = client.send(Command::SaveAs(target.clone())).unwrap();
    assert_eq!(expect_delta(&client).change, Change::OutputSaved(target.clone()));
    expect_ok(&client, id);
    assert_eq!(fs::read(&target).unwrap(), b"x\r\n");

    let new_b = write(&dir, "b2.txt", "b2\n");
    let id = client.send(Command::LoadSource { document: Document::B, path: new_b }).unwrap();
    let delta = expect_delta(&client);
    assert_eq!(delta.revision, Revision(4));
    assert!(matches!(delta.change, Change::SourceReplaced(ref text) if text.line(0) == "b2"));
    expect_ok(&client, id);

    let id = client.send(Command::Undo).unwrap();
    let delta = expect_delta(&client);
    assert_eq!(delta.change, Change::OutputEdited { range: 0..2, text: String::new(), modified: true });
    expect_ok(&client, id);
    let id = client.send(Command::Redo).unwrap();
    assert_eq!(expect_delta(&client).change, Change::OutputEdited { range: 0..0, text: "x\n".to_string(), modified: false });
    expect_ok(&client, id);
    let id = client.send(Command::Redo).unwrap();
    expect_ok(&client, id);

    let id = client.send(Command::Resync).unwrap();
    let snapshot = expect_snapshot(&client);
    assert_eq!(snapshot.revision, Revision(6));
    assert_eq!(snapshot.output.unwrap().text, "x\n");
    expect_ok(&client, id);

    drop(client);
    host.join().unwrap();
}
