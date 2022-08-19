use std::any;

use thiserror::Error;

use serde::{Deserialize, Serialize};

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
    #[error("General panic")]
    Panic,
    #[error("Floating point error occurred")]
    FloatingPoint,
    #[error("cgroup related error")]
    CGroup
}

#[derive(Debug, Clone, Copy)]
pub enum ErrorLevel {
    Partition,
    ModuleInit,
    ModuleRun,
}

#[derive(Error, Debug)]
#[error("{err:?}: {source:?}")]
pub struct TypedError {
    err: SystemError,
    source: anyhow::Error,
}

#[derive(Error, Debug)]
#[error("{err:?}: {level:?}, {source:?}")]
pub struct LeveledError {
    err: SystemError,
    level: ErrorLevel,
    source: anyhow::Error,
}

pub trait ResultExt<T> {
  fn typ_res(self, err: SystemError) -> TypedResult<T>;
  fn lev_res(self, err: SystemError, level: ErrorLevel) -> LeveledResult<T>;
}

pub trait ErrorExt{
  fn typ_err(self, err: SystemError) -> TypedError;
  fn lev_err(self, err: SystemError, level: ErrorLevel) -> LeveledError;
}

pub trait TypedResultExt<T>{
  fn upgrade(self, level: ErrorLevel) -> LeveledResult<T>;
}

impl<T> TypedResultExt<T> for TypedResult<T>{
    fn upgrade(self, level: ErrorLevel) -> LeveledResult<T> {
      match self {
        Ok(t) => Ok(t),
        Err(e) => Err(LeveledError{
          err: e.err, level, source: e.source
        })
      }
    }
}

impl<T, E: Into<anyhow::Error>> ResultExt<T> for Result<T, E> {
  fn typ_res(self, err: SystemError) -> TypedResult<T> {
      match self {
          Ok(t) => Ok(t),
          Err(e) => e.into().typ_res(err)
      }
  }

fn lev_res(self, err: SystemError, level: ErrorLevel) -> LeveledResult<T> {
  match self {
      Ok(t) => Ok(t),
      Err(e) => e.into().lev_res(err, level)
  }
    }

}

impl<T> ResultExt<T> for anyhow::Error {
fn typ_res(self, err: SystemError) -> TypedResult<T> {
  TypedResult::Err(self.typ_err(err))
}

fn lev_res(self, err: SystemError, level: ErrorLevel) -> LeveledResult<T> {
  LeveledResult::Err(self.lev_err(err, level))
    }
}

impl ErrorExt for anyhow::Error{
    fn typ_err(self, err: SystemError) -> TypedError {
        TypedError { err, source: self }
    }

    fn lev_err(self, err: SystemError, level: ErrorLevel) -> LeveledError {
        LeveledError { err, level, source: self }
    }
}