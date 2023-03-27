//! Error handling for this crate
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A Result containing a SystemError with its accompanying source
pub type TypedResult<T> = Result<T, TypedError>;

/// A low-level error issued by the operating system
///
/// This implementation is custom. Do not confuse it with the traditional unix errnos.
// TODO: Why can't we just use traditional unix errnos? The anyhow messages should be
// concrete enough.
#[derive(Error, Debug, Serialize, Deserialize, Clone, Copy)]
pub enum SystemError {
    #[error("Configuration error")]
    Config,
    #[error("Module config error")]
    ModuleConfig,
    #[error("Partition config error")]
    PartitionConfig,
    #[error("Error during Partition initialization")]
    PartitionInit,
    #[error("Segmentation error occured")]
    Segmentation,
    #[error("Time duration was exceeded by periodic process")]
    TimeDurationExceeded,
    #[error("Application error raised in partition")]
    ApplicationError,
    #[error("Unrecoverable errors")]
    Panic,
    #[error("Floating point error occurred")]
    FloatingPoint,
    #[error("cgroup related error")]
    CGroup,
}

/// Combination of a SystemError with an anyhow error
#[derive(Error, Debug)]
#[error("{err:?}: {source:?}")]
pub struct TypedError {
    err: SystemError,
    source: anyhow::Error,
}

impl TypedError {
    /// Creates a new TypedError
    pub fn new(err: SystemError, source: anyhow::Error) -> Self {
        Self { err, source }
    }
    /// Returns the SystemError of this TypedError
    pub fn err(&self) -> SystemError {
        self.err
    }
    /// Returns the anyhow error of this TypedError
    pub fn source(&self) -> &anyhow::Error {
        &self.source
    }
}

/// Converts a Result into one of our own Result types
pub trait ResultExt<T> {
    /// Converts a Result to a TypedResult
    fn typ(self, err: SystemError) -> TypedResult<T>;
}

impl<T, E: Into<anyhow::Error>> ResultExt<T> for Result<T, E> {
    fn typ(self, err: SystemError) -> TypedResult<T> {
        self.map_err(|e| TypedError {
            err,
            source: e.into(),
        })
    }
}
