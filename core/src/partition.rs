use serde::{Deserialize, Serialize};
use std::time::SystemTime;

pub enum PartitionState {
    Idle,
    ColdStart,
    WarmStart,
    Normal,
}

impl TryFrom<isize> for PartitionState {
    type Error = isize;

    fn try_from(value: isize) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(PartitionState::Idle),
            1 => Ok(PartitionState::ColdStart),
            2 => Ok(PartitionState::WarmStart),
            3 => Ok(PartitionState::Normal),
            u => Err(u),
        }
    }
}

impl From<PartitionState> for isize {
    fn from(s: PartitionState) -> Self {
        match s {
            PartitionState::Idle => 0,
            PartitionState::ColdStart => 1,
            PartitionState::WarmStart => 2,
            PartitionState::Normal => 3,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PartitionInfo {
    // TODO SystemTime system_start should probably be a single shared memory
    //    which is then distributed to all partitions as a read only sealed fd
    system_start: SystemTime,
    name: String,
    // TODO Channel
}
