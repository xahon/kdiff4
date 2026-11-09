use std::io;
use std::thread::{self, JoinHandle};

use kdiff4_protocol::{Change, Command, CurrentRegion, Delta, Event, HeuristicProposal, HostEndpoint, HostMessage, ProtocolError, RequestId, Response};

use crate::engine_registry::EngineRegistry;
use crate::error::CoreError;
use crate::session::Session;

pub fn spawn_host(endpoint: HostEndpoint) -> io::Result<JoinHandle<()>> {
    thread::Builder::new().name("kdiff4-host".to_string()).spawn(move || run_host(endpoint))
}

/// Handles requests in arrival order until the client disconnects.
pub fn run_host(endpoint: HostEndpoint) {
    let registry = EngineRegistry::default();
    let mut session: Option<Session> = None;
    while let Some(request) = endpoint.recv() {
        log::debug!("host: request {:?}", request.id);
        let result = handle(&endpoint, &registry, &mut session, request.id.clone(), request.command);
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
fn handle(endpoint: &HostEndpoint, registry: &EngineRegistry, session: &mut Option<Session>, id: RequestId, command: Command) -> Result<(), ProtocolError> {
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
            let opened = Session::open(&files, registry.default_engine())?;
            log::info!(
                "host: opened A {}, B {}, C {}, output {}",
                files.a.display(),
                files.b.display(),
                describe(files.c.as_deref()),
                describe(files.output.as_deref())
            );
            let _ = endpoint.send(HostMessage::Event(Event::Snapshot(opened.snapshot())));
            *session = Some(opened);
            return Ok(());
        }
        Command::Resync => {
            let _ = endpoint.send(HostMessage::Event(Event::Snapshot(active(session)?.snapshot())));
            return Ok(());
        }
        Command::LoadSource { document, path } => {
            let active = active(session)?;
            active.load_source(document.clone(), &path, registry.default_engine())?;
            log::info!("host: reloaded {} from {}", document.letter(), path.display());
            let _ = endpoint.send(HostMessage::Event(Event::Snapshot(active.snapshot())));
            return Ok(());
        }
        Command::Reload { confirmed } => {
            let active = active(session)?;
            active.reload(confirmed, registry.default_engine())?;
            log::info!("host: reloaded all source documents");
            let _ = endpoint.send(HostMessage::Event(Event::Snapshot(active.snapshot())));
            return Ok(());
        }
        Command::SetComparisonOptions(options) => {
            let active = active(session)?;
            active.set_comparison_options(options.clone(), registry.default_engine())?;
            log::info!("host: comparison options set to {options:?}");
            let _ = endpoint.send(HostMessage::Event(Event::Snapshot(active.snapshot())));
            return Ok(());
        }
        Command::Navigate {
            base,
            from,
            to,
            include_alignments,
        } => {
            let active = active(session)?;
            if let Some(region) = active.navigate(&base, from, to, include_alignments)? {
                log::debug!("host: navigated {to:?} to region {region}");
                send_current(endpoint, active, id);
            }
            return Ok(());
        }
        Command::SetCurrentRegion { base, region } => {
            let active = active(session)?;
            active.set_current_region(&base, region)?;
            send_current(endpoint, active, id);
            return Ok(());
        }
        Command::ApplyAlignments { confirmed } => {
            let active = active(session)?;
            if active.apply_alignments(confirmed, registry.default_engine())? {
                log::info!("host: alignments applied");
                let _ = endpoint.send(HostMessage::Event(Event::Snapshot(active.snapshot())));
            }
            return Ok(());
        }
        Command::ProposeHeuristicAlignments { language, options, within } => {
            let proposal = active(session)?.propose_heuristic_alignments(&language, &options, within)?;
            log::info!("host: Heuristics Alignment proposes {} alignments for {language}", proposal.matches.len());
            let _ = endpoint.send(HostMessage::Event(Event::HeuristicProposal(HeuristicProposal { cause: Some(id), ..proposal })));
            return Ok(());
        }
        Command::ApplyHeuristicAlignments { alignments, confirmed, within } => {
            let active = active(session)?;
            if active.apply_heuristic_alignments(&alignments, within, confirmed, registry.default_engine())? {
                log::info!("host: {} heuristic alignments applied", alignments.len());
                let _ = endpoint.send(HostMessage::Event(Event::Snapshot(active.snapshot())));
                return Ok(());
            }
            active.alignment_marks_change()
        }
        Command::MarkAlignment { document, lines } => active(session)?.mark_alignment(document, lines)?,
        Command::RemoveAlignmentMark { document, number } => active(session)?.remove_alignment_mark(document, number)?,
        Command::SetAlignmentMarkLines { document, number, lines } => active(session)?.set_alignment_mark_lines(document, number, lines)?,
        Command::RenumberAlignmentMark { document, number, to } => active(session)?.renumber_alignment_mark(document, number, to)?,
        Command::PlaceAlignment { number, slot } => active(session)?.place_alignment(number, slot)?,
        Command::ClearAlignmentMarks => match active(session)?.clear_alignment_marks() {
            Some(change) => change,
            None => return Ok(()),
        },
        Command::EditOutput(edit) => active(session)?.edit_output(&edit, registry.default_engine())?,
        Command::SetChoice { base, region, choice } => active(session)?.set_choice(&base, region, choice)?,
        Command::ChooseForUnresolved { base, document } => match active(session)?.choose_for_unresolved(&base, document)? {
            Some(change) => change,
            None => return Ok(()),
        },
        Command::SetOutputPath(path) => active(session)?.set_output_path(path)?,
        Command::SetOutputEol(eol) => active(session)?.set_output_eol(eol)?,
        Command::Save => active(session)?.save()?,
        Command::SaveAs(path) => active(session)?.save_as(&path)?,
    };
    let revision = active(session)?.revision();
    match &change {
        Change::OutputPathChanged(path) => log::info!("host: output path set to {}", describe(path.as_deref())),
        Change::OutputSaved(path) => log::info!("host: saved output to {}", path.display()),
        Change::OutputEolChanged(eol) => log::info!("host: output line ending set to {eol:?}"),
        Change::AlignmentsChanged(alignments) => log::debug!("host: {} alignment marks, changed {}, revision {}", alignments.marks.len(), alignments.changed, revision.0),
        Change::OutputEdited { range, text, merge, .. } => log::debug!(
            "host: output bytes {range:?} replaced by {} bytes, regions {}..{} by {}, revision {}",
            text.len(),
            merge.at,
            merge.at + merge.removed,
            merge.regions.len(),
            revision.0
        ),
    }
    let _ = endpoint.send(HostMessage::Event(Event::Delta(Delta {
        revision,
        cause: Some(id),
        change,
    })));
    Ok(())
}

fn send_current(endpoint: &HostEndpoint, session: &Session, id: RequestId) {
    let current = CurrentRegion {
        revision: session.revision(),
        region: session.current_region(),
        cause: Some(id),
    };
    let _ = endpoint.send(HostMessage::Event(Event::CurrentRegion(current)));
}

fn describe(path: Option<&std::path::Path>) -> String {
    path.map(|path| path.display().to_string()).unwrap_or_else(|| "none".to_string())
}

fn active(session: &mut Option<Session>) -> Result<&mut Session, CoreError> {
    session.as_mut().ok_or(CoreError::NoSession)
}
