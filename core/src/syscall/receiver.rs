use std::io::IoSliceMut;
use std::num::NonZeroUsize;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixDatagram;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use nix::libc::EINTR;
use nix::sys::socket::{recvmsg, ControlMessageOwned, MsgFlags};
use nix::{cmsg_space, unistd};
use polling::{Event, Events, Poller};

use crate::mfd::{Mfd, Seals};
use crate::syscall::{SyscallRequest, SyscallResponse};

pub struct SyscallReceiver(UnixDatagram);

impl SyscallReceiver {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let socket = UnixDatagram::bind(path)?;
        socket.set_nonblocking(true)?;
        Ok(Self(socket))
    }

    /// Receives an FD triple from fd
    // TODO: Use generics here
    fn recv_fd_triple(&self) -> Result<[OwnedFd; 3]> {
        let mut cmsg = cmsg_space!([RawFd; 3]);
        let mut iobuf = [0u8];
        let mut iov = [IoSliceMut::new(&mut iobuf)];
        let res = recvmsg::<()>(
            self.0.as_raw_fd(),
            &mut iov,
            Some(&mut cmsg),
            MsgFlags::empty(),
        )?;

        let fds: Vec<RawFd> = match res.cmsgs()?.next().unwrap() {
            ControlMessageOwned::ScmRights(fds) => fds,
            _ => bail!("received an unknown cmsg"),
        };
        let fds = fds
            .into_iter()
            .map(|fd| unsafe { OwnedFd::from_raw_fd(fd) })
            .collect::<Vec<_>>();
        fds.try_into()
            .map_err(|_| anyhow!("received fds but not a tripe"))
    }

    /// Waits for readable data on fd
    fn wait_fds(&self, timeout: Option<Duration>) -> Result<bool> {
        let poller = Poller::new()?;
        let mut events = Events::with_capacity(NonZeroUsize::MIN);
        unsafe { poller.add(self.0.as_raw_fd(), Event::readable(0))? };
        loop {
            match poller.wait(&mut events, timeout) {
                Ok(0) => return Ok(false),
                Ok(1) => return Ok(true),
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
    }

    /// Handles an unlimited amount of system calls, until timeout is reached
    ///
    /// Returns the amount of executed system calls
    pub fn handle(&self, timeout: Option<Duration>) -> Result<u32> {
        let start = Instant::now();
        let mut nsyscalls: u32 = 0;

        // A loop in which each iteration resembles the execution of one syscall
        loop {
            if let Some(timeout) = timeout {
                let remaining_time = timeout.saturating_sub(start.elapsed());
                if remaining_time.is_zero() {
                    break;
                }

                if !self.wait_fds(Some(remaining_time))? {
                    // Timeout was reached
                    break;
                }
            } else {
                let res = self.wait_fds(None)?;
                assert!(res);
            }

            let [request_fd, resp_fd, event_fd] = self.recv_fd_triple()?;
            let mut request_fd = Mfd::from_fd(request_fd)?;
            let mut response_fd = Mfd::from_fd(resp_fd)?;

            // Fetch the request
            let request = SyscallRequest::deserialize(&request_fd.read_all()?)?;
            debug!("Received system call {:?}", request);

            // Write the response (dummy response right now)
            let response = SyscallResponse {
                id: request.id,
                status: 0,
            };
            response_fd.write(&response.serialize()?)?;
            response_fd.finalize(Seals::Readable)?;

            // Trigger the event
            let buf = 1_u64.to_ne_bytes();
            unistd::write(event_fd, &buf)?;

            nsyscalls += 1;
        }

        Ok(nsyscalls)
    }
}

// #[cfg(test)]
// mod tests_receiver {
//     use std::io::IoSlice;
//     use std::os::fd::{AsFd, AsRawFd};

//     use nix::sys::eventfd::EventFd;
//     use nix::sys::socket::{
//         sendmsg, socketpair, AddressFamily, ControlMessage, SockFlag,
// SockType,     };

//     use super::*;
//     use crate::syscall::SyscallType;

//     #[test]
//     fn test_handle() {
//         let (requester, responder) = socketpair(
//             AddressFamily::Unix,
//             SockType::Datagram,
//             None,
//             SockFlag::empty(),
//         )
//         .unwrap();

//         let request_thread = std::thread::spawn(move || {
//             let mut request_fd = Mfd::create("requ").unwrap();
//             let mut response_fd = Mfd::create("resp").unwrap();
//             let event_fd = EventFd::new().unwrap();

//             // Initialize the request fd
//             request_fd
//                 .write(
//                     &SyscallRequest {
//                         id: SyscallType::Start,
//                         params: vec![1, 2, 3],
//                     }
//                     .serialize()
//                     .unwrap(),
//                 )
//                 .unwrap();
//             request_fd.finalize(Seals::Readable).unwrap();

//             // Send the fds to the responder
//             {
//                 let fds = [
//                     request_fd.as_raw_fd(),
//                     response_fd.as_raw_fd(),
//                     event_fd.as_raw_fd(),
//                 ];
//                 let cmsg = [ControlMessage::ScmRights(&fds)];
//                 let buffer = 0_u64.to_be_bytes();
//                 let iov = [IoSlice::new(buffer.as_slice())];
//                 sendmsg::<()>(requester.as_raw_fd(), &iov, &cmsg,
// MsgFlags::empty(), None).unwrap();             }

//             // Wait for a response
//             {
//                 let poller = Poller::new().unwrap();
//                 let mut events = Events::with_capacity(NonZeroUsize::MIN);
//                 unsafe {
//                     poller.add(&event_fd, Event::readable(0)).unwrap();
//                 }
//                 poller.wait(&mut events, None).unwrap();
//                 assert_eq!(events.len(), 1);
//             }

//             let response =
// SyscallResponse::deserialize(&response_fd.read_all().unwrap()).unwrap();
//             assert_eq!(response.id, SyscallType::Start);
//             assert_eq!(response.status, 0);
//         });

//         let response_thread = std::thread::spawn(move || {
//             let n = handle(responder.as_fd(),
// Some(Duration::from_secs(1))).unwrap();             assert_eq!(n, 1);
//         });

//         request_thread.join().unwrap();
//         response_thread.join().unwrap();
//     }
// }
