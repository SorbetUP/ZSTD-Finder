use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("index serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("invalid archive: {0}")]
    InvalidFormat(String),

    #[error("unsupported archive version {0}")]
    UnsupportedVersion(u16),

    #[error("archive path not found: {0}")]
    NotFound(String),

    #[error("archive entry is not a regular file: {0}")]
    NotAFile(String),

    #[error("unsafe or unsupported path: {0}")]
    InvalidPath(String),

    #[error("corrupt chunk {chunk} in {path}: {reason}")]
    CorruptChunk {
        path: String,
        chunk: usize,
        reason: String,
    },
}

pub type Result<T> = std::result::Result<T, Error>;
