use anyhow::{anyhow, Result};
use procfs::process::{FDTarget, Process};
use serde::{Deserialize, Serialize};

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

pub const SYSTEM_TIME: &str = "start_time";

pub fn get_fd(name: &str) -> Result<i32> {
    Process::myself()?
        .fd()?
        .flatten()
        .find_map(|f| {
            if let FDTarget::Path(p) = &f.target {
                if p.to_string_lossy().contains(name) {
                    Some(f.fd)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow!("No File Descriptor with Name: {name}"))
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PartitionInfo {
    name: String,
    // TODO Channel
}
