use std::{time::Duration, os::unix::prelude::{RawFd, IntoRawFd}, fs::File, io::{Read, Write}};

use apex_hal::{prelude::StartCondition, bindings::PortDirection};
use memfd::{MemfdOptions, FileSeal};
use serde::{Serialize, Deserialize};

use crate::error::{TypedError, TypedResult, ResultExt, SystemError};

pub const PARTITION_CONSTANTS_FD: &str = "PARTITION_CONSTANTS_FD";
// TODO add ENV for channel (or file?)


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PartitionConstants{
  pub name: String,
  pub identifier: i32,
  pub period: Duration,
  pub duration: Duration,
  pub start_condition: StartCondition,
  pub sender_fd: RawFd,
  pub start_time_fd: RawFd,
  pub partition_mode_fd: RawFd,
  pub sampling: Vec<SamplingConstant>
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SamplingConstant{
  pub name: String,
  pub dir: PortDirection,
  pub msg_size: usize,
}

impl PartitionConstants{
  pub const APERIODIC_PROCESS_CGROUP: &'static str = "aperiodic";
  pub const PERIODIC_PROCESS_CGROUP: &'static str = "periodic";
}

impl TryFrom<RawFd> for PartitionConstants{
  type Error = TypedError;

    fn try_from(file: RawFd) -> TypedResult<Self> {
        let mut file = File::open(format!("/proc/self/fd/{file}")).typ(SystemError::Panic)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).typ(SystemError::Panic)?;
        bincode::deserialize(&buf).typ(SystemError::Panic)
    }
}

impl TryFrom<PartitionConstants> for RawFd{
    type Error = TypedError;

    fn try_from(consts: PartitionConstants) -> TypedResult<Self> {
      let bytes = bincode::serialize(&consts).typ(SystemError::Panic)?;

      let mem = MemfdOptions::default()
          .close_on_exec(false)
          .allow_sealing(true)
          .create("constants")
          .typ(SystemError::Panic)?;
      mem.as_file().set_len(bytes.len() as u64).typ(SystemError::Panic)?;
      mem.as_file().write_all(&bytes).typ(SystemError::Panic)?;
      mem.add_seals(&[FileSeal::SealShrink, FileSeal::SealGrow, FileSeal::SealWrite, FileSeal::SealSeal])
          .typ(SystemError::Panic)?;

      Ok(mem.into_raw_fd())
    }
}