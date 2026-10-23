// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::PathBuf;

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub(crate) type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no git repository found at {path}")]
    NotARepository {
        path: PathBuf,
        #[source]
        source: BoxError,
    },
    #[error("{context}: {source}")]
    Git {
        context: &'static str,
        #[source]
        source: BoxError,
    },
    #[error("`git {command}` failed: {stderr}")]
    GitCommand { command: String, stderr: String },
    #[error("{0}")]
    GitUnavailable(String),
    #[error("invalid object id `{0}`")]
    InvalidObjectId(String),
    #[error("{0} not found")]
    NotFound(String),
    #[error("the repository has no working tree")]
    BareRepository,
    #[error("the operation was cancelled")]
    Cancelled,
    #[error("{0}")]
    Unsupported(String),
    /// The repository changed since the data an operation refers to was read.
    #[error("{0}")]
    OutOfDate(String),
    #[error("{context}: {source}")]
    Io {
        context: &'static str,
        #[source]
        source: std::io::Error,
    },
}

/// Attaches a short description of what was being attempted to a gitoxide error.
pub(crate) trait Context<T> {
    fn context(self, context: &'static str) -> Result<T>;
}

impl<T, E: Into<BoxError>> Context<T> for std::result::Result<T, E> {
    fn context(self, context: &'static str) -> Result<T> {
        self.map_err(|err| Error::Git {
            context,
            source: err.into(),
        })
    }
}
