//! Error handling for this crate
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A Result containing a SystemError with its accompanying source
pub type TypedResult<T> = Result<T, TypedError>;
/// A Result containing a SystemError with its accompanying error and time window
// TODO: Consider merging these two types by making level an Option.
pub type LeveledResult<T> = Result<T, LeveledError>;

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

/// The time window in which the error has occurred
#[derive(Debug, Clone, Copy)]
pub enum ErrorLevel {
    /// Synchronous to Partition Time Window
    Partition,
    /// During Module Init Phase
    ModuleInit,
    /// Asynchronous to Partition Time Window
    ModuleRun,
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

/// Combination of a SystemError with an anyhow error and its time window
// TODO: Consider naming "level" "source" instead, as it indicates in which
// time window the error has occurred?
#[derive(Error, Debug)]
#[error("{err:?}: {level:?}, {source:?}")]
pub struct LeveledError {
    err: SystemError,
    level: ErrorLevel,
    source: anyhow::Error,
}

impl LeveledError {
    /// Creates a new LeveledError
    pub fn new(err: SystemError, level: ErrorLevel, source: anyhow::Error) -> Self {
        Self { err, level, source }
    }
    /// Returns the SystemError of this TypedError
    pub fn err(&self) -> SystemError {
        self.err
    }
    /// Returns the ErrorLevel of this TypedError
    pub fn level(&self) -> ErrorLevel {
        self.level
    }
    /// Returns the anyhow error of this TypedError
    pub fn source(&self) -> &anyhow::Error {
        &self.source
    }
}
impl From<LeveledError> for TypedError {
    fn from(le: LeveledError) -> Self {
        // Basically just cut off the level field
        Self {
            err: le.err,
            source: le.source,
        }
    }
}

/// Converts a Result into one of our own Result types
pub trait ResultExt<T> {
    /// Converts a Result to a TypedResult
    fn typ(self, err: SystemError) -> TypedResult<T>;
    /// Converts a Result to a LeveledResult
    fn lev_typ(self, err: SystemError, level: ErrorLevel) -> LeveledResult<T>;
}

/// Converts a TypedResult to one of our own Result types
pub trait TypedResultExt<T> {
    /// Creates a LeveledResult from a TypedResult
    fn lev(self, level: ErrorLevel) -> LeveledResult<T>;
}

impl<T> TypedResultExt<T> for TypedResult<T> {
    fn lev(self, level: ErrorLevel) -> LeveledResult<T> {
        // This basically just creates a LeveledError with all fields tken even from
        // the TypedResult, except the level being added.
        self.map_err(|e| LeveledError {
            err: e.err,
            level,
            source: e.source,
        })
    }
}

impl<T, E: Into<anyhow::Error>> ResultExt<T> for Result<T, E> {
    fn typ(self, err: SystemError) -> TypedResult<T> {
        self.map_err(|e| TypedError {
            err,
            source: e.into(),
        })
    }

    fn lev_typ(self, err: SystemError, level: ErrorLevel) -> LeveledResult<T> {
        self.map_err(|e| LeveledError {
            err,
            level,
            source: e.into(),
        })
    }
}