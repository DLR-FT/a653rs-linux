//! Common definitions for the execution of system calls

use anyhow::Result;

pub const SYSCALL_SOCKET_PATH: &str = "/syscall-a653";

pub mod receiver;
pub mod sender;
pub mod syscalls;
mod ty;

pub use ty::SyscallType;

// This is the data type that is transferred to the hypervisor when a
// syscall request is made by a partition. The parameter data is stored as an
// already serialized `Vec<u8>`, so that the receiver can deserialize the
// SyscallType without knowing the parameter's types.
type SyscallRequest = (SyscallType, Vec<u8>);

// This is the data type that is returned from the hypervisor to the partition
// when a syscall was handled. In contrast to [`SyscallRequest`], a generic can
// be used for the return value's type.
type SyscallResponse<T> = Result<T, a653rs::bindings::ErrorReturnCode>;

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::os::unix::net::UnixDatagram;
    use std::thread;
    use std::time::Duration;

    use a653rs::bindings::ApexSystemTime;
    use a653rs::prelude::{QueueOverflow, QueuingPortId};

    use super::SyscallType;
    use crate::syscall::receiver::{self, SyscallReceiver};
    use crate::syscall::sender::SyscallSender;
    use crate::syscall::syscalls;

    fn new_sender_receiver_pair() -> (SyscallSender, SyscallReceiver) {
        let (sender, receiver) = UnixDatagram::pair().unwrap();
        (
            SyscallSender::from_datagram(sender),
            SyscallReceiver::from_datagram(receiver),
        )
    }

    #[test]
    pub fn single_syscall() {
        let (sender, receiver) = new_sender_receiver_pair();

        let receiver_thread = thread::spawn(move || {
            let syscall_handler = |ty: SyscallType, serialized_params: &[u8]| -> Vec<u8> {
                assert_eq!(ty, SyscallType::SendQueuingMessage);

                receiver::wrap_serialization::<syscalls::SendQueuingMessage, _>(
                    serialized_params,
                    |params| {
                        assert_eq!(
                            params,
                            (
                                0 as QueuingPortId,
                                [1u8, 2, 3].as_slice(),
                                0 as ApexSystemTime
                            )
                        );

                        Ok(())
                    },
                )
                .expect("serialization to succeed")
            };

            let syscall_was_handled = receiver
                .receive_one(Some(Duration::from_secs(1)), syscall_handler)
                .unwrap();

            assert!(syscall_was_handled);
        });

        // Make a syscall
        let response: Result<(), a653rs::bindings::ErrorReturnCode> = sender
            .execute::<syscalls::SendQueuingMessage>((
                0 as QueuingPortId,
                &[1, 2, 3],
                0 as ApexSystemTime,
            ))
            .expect("sending and receiving a response to succeed");

        assert_eq!(response, Ok(()));

        // join the receiver thread just to be safe
        receiver_thread.join().unwrap();
    }

    #[test]
    pub fn two_syscalls() {
        let (sender, receiver) = new_sender_receiver_pair();

        // The receiver thread represents the hypervisor.
        let receiver_thread = thread::spawn(move || {
            // A simulated queuing port. This represents the hypervisor state.
            let mut queuing_port_state: VecDeque<Vec<u8>> = VecDeque::new();

            let mut syscall_handler = |ty: SyscallType, serialized_params: &[u8]| -> Vec<u8> {
                match ty {
                    SyscallType::SendQueuingMessage => {
                        receiver::wrap_serialization::<syscalls::SendQueuingMessage, _>(
                            serialized_params,
                            |params| {
                                queuing_port_state.push_back(params.1.to_owned());

                                Ok(())
                            },
                        )
                        .expect("serialization to succeed")
                    }
                    SyscallType::ReceiveQueuingMessage => {
                        receiver::wrap_serialization::<syscalls::ReceiveQueuingMessage, _>(
                            serialized_params,
                            |_params| {
                                queuing_port_state
                                    .pop_front()
                                    .map(|msg| (false as QueueOverflow, msg))
                                    .ok_or(a653rs::bindings::ErrorReturnCode::NotAvailable)
                            },
                        )
                        .expect("serialization to succeed")
                    }
                    _ => unimplemented!("this test only implements two syscalls"),
                }
            };

            // Let's handle exactly three syscalls
            for _ in 0..3 {
                let syscall_was_handled = receiver
                    .receive_one(Some(Duration::from_secs(1)), &mut syscall_handler)
                    .unwrap(); // TODO log error and ignore syscall
                assert!(syscall_was_handled);
            }
        });

        // Send one message into the queuing port
        let response = sender
            .execute::<syscalls::SendQueuingMessage>((
                0 as QueuingPortId,
                &[4, 3, 2, 1],
                0 as ApexSystemTime,
            ))
            .unwrap();
        assert_eq!(response, Ok(()));

        // Receive the previous message from the queuing port
        let response = sender
            .execute::<syscalls::ReceiveQueuingMessage>((0 as QueuingPortId, 0 as ApexSystemTime))
            .expect("sending and receiving a response to succeed");
        assert_eq!(response, Ok((false as QueueOverflow, vec![4, 3, 2, 1])));

        // Now the queuing port should be empty
        let response = sender
            .execute::<syscalls::ReceiveQueuingMessage>((0 as QueuingPortId, 0 as ApexSystemTime))
            .expect("sending and receiving a response to succeed");
        assert_eq!(
            response,
            Err(a653rs::bindings::ErrorReturnCode::NotAvailable)
        );

        // join the receiver thread just to be safe
        receiver_thread.join().unwrap();
    }
}
