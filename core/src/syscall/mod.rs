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
    use crate::syscall::SyscallResponse;

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
            let syscall_handler = |request: SyscallRequest| {
                assert_eq!(request.params, &[1, 2, 3]);
                SyscallResponse {
                    id: request.id,
                    status: 456,
                }
            };

            let syscall_was_handled = receiver
                .receive_one(Some(Duration::from_secs(1)), syscall_handler)
                .unwrap();

            assert!(syscall_was_handled);
        });

        // Use random data for now
        let request = SyscallRequest {
            id: SyscallType::GetProcessId,
            params: vec![1, 2, 3],
        };

        let response = sender.execute(request).unwrap();

        assert_eq!(response.id, SyscallType::GetProcessId);
        assert_eq!(response.status, 456);

        // join the receiver thread just to be safe
        receiver_thread.join().unwrap();
    }
}
