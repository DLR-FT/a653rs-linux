//! Common definitions for the execution of system calls

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub const SYSCALL_SOCKET_PATH: &str = "/syscall-a653";

pub mod receiver;
pub mod sender;
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

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixDatagram;
    use std::thread;
    use std::time::Duration;

    use super::{SyscallRequest, SyscallType};
    use crate::syscall::receiver::SyscallReceiver;
    use crate::syscall::sender::SyscallSender;

    #[test]
    pub fn single_syscall() {
        let (sender, receiver) = {
            let (sender, receiver) = UnixDatagram::pair().unwrap();
            (
                SyscallSender::from_datagram(sender),
                SyscallReceiver::from_datagram(receiver),
            )
        };

        let receiver_thread = thread::spawn(move || {
            let num_handled_syscalls = receiver.handle(Some(Duration::from_secs(1))).unwrap();
            assert_eq!(num_handled_syscalls, 1);
        });

        // Use random data for now
        let request = SyscallRequest {
            id: SyscallType::GetProcessId,
            params: Vec::default(),
        };

        let response = sender.execute_fd(request).unwrap();

        assert_eq!(response.id, SyscallType::GetProcessId);
        assert_eq!(
            response.status, 0,
            "expected default syscall return status of 0"
        );

        // join the receiver thread until `SysCallReceiver` allows to receive just a
        // single syscall
        receiver_thread.join().unwrap();
    }
}
