use std::io;
use std::path::PathBuf;

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
}


impl From<CoreError> for ProtocolError {
    fn from(error: CoreError) -> Self {
        let kind = match &error {
            CoreError::Read { .. } | CoreError::Write { .. } => ErrorKind::Io,
            CoreError::Directory { .. } | CoreError::NoSession | CoreError::NoSuchDocument { .. } | CoreError::NotAMerge | CoreError::NoOutputPath | CoreError::StaleRevision { .. } | CoreError::InvalidRange { .. } => ErrorKind::InvalidRequest,
        };
        ProtocolError { kind, message: error.to_string() }
    }
}
