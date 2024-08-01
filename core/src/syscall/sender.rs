use std::io::IoSlice;
use std::num::NonZeroUsize;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::os::unix::net::UnixDatagram;
use std::path::Path;

use anyhow::Result;
use nix::libc::EINTR;
use nix::sys::eventfd::EventFd;
use nix::sys::socket::{sendmsg, ControlMessage, MsgFlags};
use polling::{Event, Events, Poller};

use crate::mfd::{Mfd, Seals};
use crate::syscall::{SyscallRequest, SyscallResponse};

pub struct SyscallSender(UnixDatagram);

impl SyscallSender {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let socket = UnixDatagram::unbound()?;
        socket.connect(path.as_ref())?;
        socket.set_nonblocking(true)?;
        Ok(Self(socket))
    }

    /// Sends a vector of file descriptors through a Unix socket
    fn send_fds<const COUNT: usize, T: AsRawFd>(&self, fds: [T; COUNT]) -> Result<()> {
        let fds = fds.map(|f| f.as_raw_fd());
        let cmsg = [ControlMessage::ScmRights(&fds)];
        let buffer = 0_u64.to_ne_bytes();
        let iov = [IoSlice::new(buffer.as_slice())];
        sendmsg::<()>(self.0.as_raw_fd(), &iov, &cmsg, MsgFlags::empty(), None)?;
        Ok(())
    }

    /// Waits for action on the event fd
    // TODO: Consider timeout
    fn wait_event(event_fd: BorrowedFd) -> Result<()> {
        let poller = Poller::new()?;
        let mut events = Events::with_capacity(NonZeroUsize::MIN);
        unsafe {
            poller.add(event_fd.as_raw_fd(), Event::readable(0))?;
        }

        loop {
            match poller.wait(&mut events, None) {
                Ok(1) => break,
                Err(e) => {
                    if e.raw_os_error() == Some(EINTR) {
                        continue;
                    } else {
                        panic!("poller failed with {:?}", e)
                    }
                }
                _ => panic!("unknown poller state"),
            }
        }

        Ok(())
    }

    pub fn execute_fd(&self, request: SyscallRequest) -> Result<SyscallResponse> {
        // Create the file descriptor triple
        let mut request_fd = Mfd::create("requ")?;
        let mut response_fd = Mfd::create("resp")?;
        let event_fd = EventFd::new()?;

        // Initialize the request file descriptor
        request_fd.write(&request.serialize()?)?;
        request_fd.finalize(Seals::Readable)?;

        // Send the file descriptors to the hypervisor
        self.send_fds([request_fd.as_fd(), response_fd.as_fd(), event_fd.as_fd()])?;

        Self::wait_event(event_fd.as_fd())?;

        let response = SyscallResponse::deserialize(&response_fd.read_all()?)?;
        Ok(response)
    }
}

// #[cfg(test)]
// mod tests {
//     use std::io::IoSliceMut;
//     use std::os::fd::{FromRawFd, OwnedFd, RawFd};

//     use nix::sys::socket::{
//         recvmsg, socketpair, AddressFamily, ControlMessageOwned, SockFlag,
// SockType,     };
//     use nix::{cmsg_space, unistd};

//     use super::*;
//     use crate::syscall::SyscallType;

//     #[test]
//     fn test_execute() {
//         let (requester, responder) = socketpair(
//             AddressFamily::Unix,
//             SockType::Datagram,
//             None,
//             SockFlag::empty(),
//         )
//         .unwrap();

//         let request_thread = std::thread::spawn(move || {
//             let response = execute_fd(
//                 requester.as_fd(),
//                 SyscallRequest {
//                     id: SyscallType::Start,
//                     params: vec![1, 2, 42],
//                 },
//             )
//             .unwrap();

//             assert_eq!(response.id, SyscallType::Start);
//             assert_eq!(response.status, 42);
//         });
//         let response_thread = std::thread::spawn(move || {
//             // Receive the file descriptors
//             let mut cmsg = cmsg_space!([RawFd; 3]);
//             let mut iobuf = [0u8];
//             let mut iov = [IoSliceMut::new(&mut iobuf)];
//             let res = recvmsg::<()>(
//                 responder.as_raw_fd(),
//                 &mut iov,
//                 Some(&mut cmsg),
//                 MsgFlags::empty(),
//             )
//             .unwrap();

//             let fds: Vec<OwnedFd> = match
// res.cmsgs().unwrap().next().unwrap() {
// ControlMessageOwned::ScmRights(fds) => fds                     .into_iter()
//                     .map(|fd| unsafe { OwnedFd::from_raw_fd(fd) })
//                     .collect::<Vec<_>>(),
//                 _ => panic!("unknown cmsg received"),
//             };

//             let [request, response, event_fd] = fds.try_into().unwrap();
//             let mut request_fd = Mfd::from_fd(request).unwrap();
//             let mut response_fd = Mfd::from_fd(response).unwrap();

//             // Fetch the request
//             let request =
// SyscallRequest::deserialize(&request_fd.read_all().unwrap()).unwrap();
//             assert_eq!(request.id, SyscallType::Start);
//             assert_eq!(request.params, vec![1, 2, 42]);

//             // Write the response
//             response_fd
//                 .write(
//                     &SyscallResponse {
//                         id: SyscallType::Start,
//                         status: 42,
//                     }
//                     .serialize()
//                     .unwrap(),
//                 )
//                 .unwrap();
//             response_fd.finalize(Seals::Readable).unwrap();

//             // Trigger the eventfd
//             let buf = 1_u64.to_ne_bytes();
//             unistd::write(event_fd, &buf).unwrap();
//         });

//         request_thread.join().unwrap();
//         response_thread.join().unwrap();
//     }
// }
