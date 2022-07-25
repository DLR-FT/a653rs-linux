use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::health::{
    ModuleInitHMTable, ModuleRecoveryAction, ModuleRunHMTable, PartitionHMTable, RecoveryAction,
};

pub type TypedResult<T> = Result<T, TypedError>;
pub type LeveledResult<T> = Result<T, LeveledError>;

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

#[derive(Debug, Clone, Copy)]
pub enum ErrorLevel {
    /// Synchronous to Partition Time Window
    Partition,
    /// During Module Init Phase
    ModuleInit,
    /// Asynchronous to Partition Time Window
    ModuleRun,
}

#[derive(Error, Debug)]
#[error("{err:?}: {source:?}")]
pub struct TypedError {
    err: SystemError,
    source: anyhow::Error,
}

impl TypedError {
    pub fn new(err: SystemError, source: anyhow::Error) -> Self {
        Self { err, source }
    }
    pub fn err(&self) -> SystemError {
        self.err
    }
    pub fn source(&self) -> &anyhow::Error {
        &self.source
    }
}

#[derive(Error, Debug)]
#[error("{err:?}: {level:?}, {source:?}")]
pub struct LeveledError {
    err: SystemError,
    level: ErrorLevel,
    source: anyhow::Error,
}

impl LeveledError {
    pub fn new(err: SystemError, level: ErrorLevel, source: anyhow::Error) -> Self {
        Self { err, level, source }
    }

    pub fn err(&self) -> SystemError {
        self.err
    }
    pub fn level(&self) -> ErrorLevel {
        self.level
    }
    pub fn source(&self) -> &anyhow::Error {
        &self.source
    }
}
impl From<LeveledError> for TypedError {
    fn from(le: LeveledError) -> Self {
        Self {
            err: le.err,
            source: le.source,
        }
    }
}

pub trait ResultExt<T> {
    fn typ(self, err: SystemError) -> TypedResult<T>;
    fn lev_typ(self, err: SystemError, level: ErrorLevel) -> LeveledResult<T>;
}

pub trait TypedResultExt<T> {
    fn lev(self, level: ErrorLevel) -> LeveledResult<T>;
}

pub trait TypedErrorExt<T> {
    fn map_ignore_part(self, hm: &PartitionHMTable) -> LeveledResult<T>;
    fn map_ignore_mod_init(self, hm: &ModuleInitHMTable) -> LeveledResult<T>;
    fn map_ignore_mod_run(self, hm: &ModuleRunHMTable) -> LeveledResult<T>;
}

impl<T> TypedResultExt<T> for TypedResult<T> {
    fn lev(self, level: ErrorLevel) -> LeveledResult<T> {
        self.map_err(|e| LeveledError {
            err: e.err,
            level,
            source: e.source,
        })
    }
}

impl TypedErrorExt<()> for TypedResult<()> {
    fn map_ignore_part(self, hm: &PartitionHMTable) -> LeveledResult<()> {
        if let Err(err) = self {
            return err.map_ignore_part(hm);
        }
        self.lev(ErrorLevel::Partition)
    }

    fn map_ignore_mod_init(self, hm: &ModuleInitHMTable) -> LeveledResult<()> {
        if let Err(err) = self {
            return err.map_ignore_mod_init(hm);
        }
        self.lev(ErrorLevel::ModuleInit)
    }

    fn map_ignore_mod_run(self, hm: &ModuleRunHMTable) -> LeveledResult<()> {
        if let Err(err) = self {
            return err.map_ignore_mod_run(hm);
        }
        self.lev(ErrorLevel::ModuleRun)
    }
}

impl TypedErrorExt<()> for TypedError {
    fn map_ignore_part(self, hm: &PartitionHMTable) -> LeveledResult<()> {
        if let Some(RecoveryAction::Module(ModuleRecoveryAction::Ignore)) = hm.try_action(self.err)
        {
            return Ok(());
        }
        Err(self).lev(ErrorLevel::Partition)
    }

    fn map_ignore_mod_init(self, hm: &ModuleInitHMTable) -> LeveledResult<()> {
        if let Some(ModuleRecoveryAction::Ignore) = hm.try_action(self.err) {
            return Ok(());
        }
        Err(self).lev(ErrorLevel::ModuleInit)
    }

    fn map_ignore_mod_run(self, hm: &ModuleRunHMTable) -> LeveledResult<()> {
        if let Some(ModuleRecoveryAction::Ignore) = hm.try_action(self.err) {
            return Ok(());
        }
        Err(self).lev(ErrorLevel::ModuleRun)
    }
}

impl<T, E: Into<anyhow::Error>> ResultExt<T> for Result<T, E> {
    fn typ(self, err: SystemError) -> TypedResult<T> {
        match self {
            Ok(t) => Ok(t),
            Err(e) => Err(TypedError {
                err,
                source: e.into(),
            }),
        }
    }

    fn lev_typ(self, err: SystemError, level: ErrorLevel) -> LeveledResult<T> {
        match self {
            Ok(t) => Ok(t),
            Err(e) => Err(LeveledError {
                err,
                level,
                source: e.into(),
            }),
        }
    }

    //fn rec_res(self, err: SystemError, level: ErrorLevel) -> RecoverableResult<T> {
    //    match self {
    //        Ok(t) => Ok(t),
    //        Err(e) => e.into().rec_res(err, level),
    //    }
    //}
}

//impl<T> ResultExt<T> for anyhow::Error {
//    fn typ(self, err: SystemError) -> TypedResult<T> {
//        TypedResult::Err(self.typ(err))
//    }
//
//    fn lev_typ(self, err: SystemError, level: ErrorLevel) -> LeveledResult<T> {
//        todo!()
//    }
//
//    //fn rec_res(self, err: SystemError, level: ErrorLevel) -> RecoverableResult<T> {
//    //    Err(RecoverableError::new(err, level, self))
//    //}
//}
