use std::io;
use std::thread::{self, JoinHandle};

use kdiff4_protocol::{Command, Delta, Event, HostEndpoint, HostMessage, ProtocolError, RequestId, Response};

use crate::error::CoreError;
use crate::session::Session;


pub fn spawn_host(endpoint: HostEndpoint) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("kdiff4-host".to_string())
        .spawn(move || run_host(endpoint))
}


/// Handles requests in arrival order until the client disconnects.
pub fn run_host(endpoint: HostEndpoint) {
    let mut session: Option<Session> = None;
    while let Some(request) = endpoint.recv() {
        log::debug!("host: request {:?}", request.id);
        let result = handle(&endpoint, &mut session, request.id.clone(), request.command);
        if let Err(error) = &result {
            log::warn!("host: request {:?} failed: {error}", request.id);
        }
        if endpoint.send(HostMessage::Response(Response { id: request.id, result })).is_err() {
            break;
        }
    }
    log::debug!("host: client disconnected");
}


/// Events are sent before the response, so a client sees the new state when its request completes. A disconnected client is noticed when the response is sent.
fn handle(endpoint: &HostEndpoint, session: &mut Option<Session>, id: RequestId, command: Command) -> Result<(), ProtocolError> {
    let change = match command {
        Command::Undo => match active(session)?.undo()? {
            Some(change) => change,
            None => return Ok(()),
        },
        Command::Redo => match active(session)?.redo()? {
            Some(change) => change,
            None => return Ok(()),
        },
        Command::Open(files) => {
            let opened = Session::open(&files)?;
            let _ = endpoint.send(HostMessage::Event(Event::Snapshot(opened.snapshot())));
            *session = Some(opened);
            return Ok(());
        }
        Command::Resync => {
            let _ = endpoint.send(HostMessage::Event(Event::Snapshot(active(session)?.snapshot())));
            return Ok(());
        }
        Command::LoadSource { document, path } => active(session)?.load_source(document, &path)?,
        Command::EditOutput(edit) => active(session)?.edit_output(&edit)?,
        Command::SetOutputPath(path) => active(session)?.set_output_path(path)?,
        Command::Save => active(session)?.save()?,
        Command::SaveAs(path) => active(session)?.save_as(&path)?,
    };
    let revision = active(session)?.revision();
    let _ = endpoint.send(HostMessage::Event(Event::Delta(Delta { revision, cause: Some(id), change })));
    Ok(())
}


fn active(session: &mut Option<Session>) -> Result<&mut Session, CoreError> {
    session.as_mut().ok_or(CoreError::NoSession)
}
