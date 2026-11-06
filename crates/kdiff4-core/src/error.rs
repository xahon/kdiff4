use std::io;
use std::path::PathBuf;

use kdiff4_protocol::engine::EngineError;
use kdiff4_protocol::{Document, ErrorKind, ProtocolError, Revision};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("cannot read {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("cannot write {}: {source}", path.display())]
    Write { path: PathBuf, source: io::Error },
    #[error("{} is a directory, not a file", path.display())]
    Directory { path: PathBuf },
    #[error("no session is open")]
    NoSession,
    #[error("this session has no {} document", document.letter())]
    NoSuchDocument { document: Document },
    #[error("this session has no output: it is not a merge")]
    NotAMerge,
    #[error("the output has no file path yet")]
    NoOutputPath,
    #[error("the edit was made on revision {}, but the session is at revision {}", base.0, current.0)]
    StaleRevision { base: Revision, current: Revision },
    #[error("the edit range {start}..{end} does not fit the output text")]
    InvalidRange { start: usize, end: usize },
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error("there is no region {index}; the session has {count}")]
    NoSuchRegion { index: usize, count: usize },
    #[error("the edit can't go to region {region}: it must insert whole lines where that region, which has no lines, sits")]
    InvalidEditTarget { region: usize },
    #[error("invalid choice: {reason}")]
    InvalidChoice { reason: String },
    #[error("{count} unresolved {} left; resolve them before saving", if *count == 1 { "conflict is" } else { "conflicts are" })]
    UnresolvedConflicts { count: usize },
    #[error("invalid alignment mark: {reason}")]
    InvalidMark { reason: String },
    #[error("applying the alignments compares everything again and discards the choices and edits in D")]
    NeedsConfirmation,
}

impl From<CoreError> for ProtocolError {
    fn from(error: CoreError) -> Self {
        if let CoreError::Engine(engine_error) = &error {
            return ProtocolError::from(engine_error.clone());
        }
        let kind = match &error {
            CoreError::Read { .. } | CoreError::Write { .. } => ErrorKind::Io,
            CoreError::Directory { .. }
            | CoreError::NoSession
            | CoreError::NoSuchDocument { .. }
            | CoreError::NotAMerge
            | CoreError::NoOutputPath
            | CoreError::StaleRevision { .. }
            | CoreError::InvalidRange { .. }
            | CoreError::NoSuchRegion { .. }
            | CoreError::InvalidChoice { .. }
            | CoreError::InvalidEditTarget { .. }
            | CoreError::UnresolvedConflicts { .. }
            | CoreError::InvalidMark { .. } => ErrorKind::InvalidRequest,
            CoreError::NeedsConfirmation => ErrorKind::NeedsConfirmation,
            CoreError::Engine(_) => ErrorKind::Engine,
        };
        ProtocolError { kind, message: error.to_string() }
    }
}
