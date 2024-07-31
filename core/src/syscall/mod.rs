//! Common definitions for the execution of system calls

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub const SYSCALL_SOCKET_PATH: &str = "/syscall-a653";

mod ty;

pub use ty::SyscallType;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct SyscallRequest {
    pub id: SyscallType,
    pub params: Vec<u64>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct SyscallResponse {
    pub id: SyscallType,
    pub status: u64,
}

impl SyscallRequest {
    /// Serializes a SyscallRequest into its binary representation
    pub fn serialize(&self) -> Result<Vec<u8>> {
        bincode::serialize(self).map_err(Into::into)
    }

    /// Deserializes a serialized SyscallRequest back into its internal
    /// representation
    pub fn deserialize(serialized: &[u8]) -> Result<Self> {
        bincode::deserialize::<Self>(serialized).map_err(Into::into)
    }
}

impl SyscallResponse {
    /// Serializes a SyscallResponse into its binary representation
    pub fn serialize(&self) -> Result<Vec<u8>> {
        bincode::serialize(self).map_err(Into::into)
    }

    /// Deserializes a serialized SyscallResponse back into its internal
    /// representation
    pub fn deserialize(serialized: &[u8]) -> Result<Self> {
        bincode::deserialize::<Self>(serialized).map_err(Into::into)
    }
}
