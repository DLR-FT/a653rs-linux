use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::prelude::{IntoRawFd, RawFd};
use std::time::Duration;

use apex_rs::bindings::PortDirection;
use apex_rs::prelude::{PartitionId, StartCondition};
use memfd::{FileSeal, MemfdOptions};
use serde::{Deserialize, Serialize};

use crate::error::{ResultExt, SystemError, TypedError, TypedResult};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PartitionConstants {
    pub name: String,
    pub identifier: PartitionId,
    pub period: Duration,
    pub duration: Duration,
    pub start_condition: StartCondition,
    pub sender_fd: RawFd,
    pub start_time_fd: RawFd,
    pub partition_mode_fd: RawFd,
    pub sampling: Vec<SamplingConstant>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SamplingConstant {
    pub name: String,
    pub dir: PortDirection,
    pub msg_size: usize,
    pub fd: RawFd,
}

impl PartitionConstants {
    pub const PARTITION_CONSTANTS_FD: &'static str = "PARTITION_CONSTANTS_FD";
    pub const APERIODIC_PROCESS_CGROUP: &'static str = "aperiodic";
    pub const PERIODIC_PROCESS_CGROUP: &'static str = "periodic";

    pub fn open() -> TypedResult<Self> {
        let fd = std::env::var(Self::PARTITION_CONSTANTS_FD)
            .typ(SystemError::PartitionInit)?
            .parse::<RawFd>()
            .typ(SystemError::PartitionInit)?;
        PartitionConstants::try_from(fd).typ(SystemError::PartitionInit)
    }
}

impl TryFrom<RawFd> for PartitionConstants {
    type Error = TypedError;

    fn try_from(file: RawFd) -> TypedResult<Self> {
        let mut file = File::open(format!("/proc/self/fd/{file}")).typ(SystemError::Panic)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).typ(SystemError::Panic)?;
        bincode::deserialize(&buf).typ(SystemError::Panic)
    }
}

impl TryFrom<PartitionConstants> for RawFd {
    type Error = TypedError;

    fn try_from(consts: PartitionConstants) -> TypedResult<Self> {
        let bytes = bincode::serialize(&consts).typ(SystemError::Panic)?;

        let mem = MemfdOptions::default()
            .close_on_exec(false)
            .allow_sealing(true)
            .create("constants")
            .typ(SystemError::Panic)?;
        mem.as_file()
            .set_len(bytes.len() as u64)
            .typ(SystemError::Panic)?;
        mem.as_file().write_all(&bytes).typ(SystemError::Panic)?;
        mem.add_seals(&[
            FileSeal::SealShrink,
            FileSeal::SealGrow,
            FileSeal::SealWrite,
            FileSeal::SealSeal,
        ])
        .typ(SystemError::Panic)?;

        Ok(mem.into_raw_fd())
    }
}
