//! Implementation of the mechanism to perform system calls

use std::io::IoSliceMut;
use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use a653rs_linux_core::mfd::{Mfd, Seals};
use a653rs_linux_core::syscall::{SyscallRequ, SyscallResp};
use anyhow::{bail, Result};
use libc::EINTR;
use nix::sys::socket::{recvmsg, ControlMessageOwned, MsgFlags};
use nix::{cmsg_space, unistd};
use polling::{Event, Poller};

/// Receives an FD triple from fd
// TODO: Use generics here
fn recv_fd_triple(fd: RawFd) -> Result<[RawFd; 3]> {
    let mut cmsg = cmsg_space!([RawFd; 3]);
    let mut iobuf = [0u8];
    let mut iov = [IoSliceMut::new(&mut iobuf)];
    let res = recvmsg::<()>(fd, &mut iov, Some(&mut cmsg), MsgFlags::empty())?;

    let fds: Vec<RawFd> = match res.cmsgs().next().unwrap() {
        ControlMessageOwned::ScmRights(fds) => fds,
        _ => bail!("received an unknown cmsg"),
    };
    if fds.len() != 3 {
        bail!("received fds but not a tripe")
    }

    Ok([fds[0], fds[1], fds[2]])
}

/// Waits for readable data on fd
fn wait_fds(fd: RawFd, timeout: Option<Duration>) -> Result<bool> {
    let poller = Poller::new()?;
    let mut events = Vec::with_capacity(1);
    poller.add(fd, Event::readable(0))?;
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
pub fn handle(fd: RawFd, timeout: Option<Duration>) -> Result<u32> {
    let start = Instant::now();
    let mut nsyscalls: u32 = 0;

    // A loop in which each iteration resembles the execution of one syscall
    loop {
        if let Some(timeout) = timeout {
            let remaining_time = timeout.saturating_sub(start.elapsed());
            if remaining_time.is_zero() {
                break;
            }

            if !wait_fds(fd, Some(remaining_time))? {
                // Timeout was reached
                break;
            }
        } else {
            let res = wait_fds(fd, None)?;
            assert!(res);
        }

        let fds = recv_fd_triple(fd)?;
        let mut requ_fd = Mfd::from_fd(fds[0])?;
        let mut resp_fd = Mfd::from_fd(fds[1])?;
        let event_fd = fds[2];

        // Fetch the request
        let requ = SyscallRequ::deserialize(&requ_fd.read_all()?)?;
        debug!("Received system call {:?}", requ);

        // Write the response (dummy response right now)
        let resp = SyscallResp {
            id: requ.id,
            status: 0,
        };
        resp_fd.write(&resp.serialize()?)?;
        resp_fd.finalize(Seals::Readable)?;

        // Trigger the event
        let buf = 1_u64.to_ne_bytes();
        unistd::write(event_fd, &buf)?;

        nsyscalls += 1;
    }

    Ok(nsyscalls)
}

#[cfg(test)]
mod tests {
    use std::io::IoSlice;

    use a653rs_linux_core::syscall::ApexSyscall;
    use nix::sys::eventfd::{eventfd, EfdFlags};
    use nix::sys::socket::{
        sendmsg, socketpair, AddressFamily, ControlMessage, SockFlag, SockType,
    };

    use super::*;

    #[test]
    fn test_handle() {
        let (requester, responder) = socketpair(
            AddressFamily::Unix,
            SockType::Datagram,
            None,
            SockFlag::empty(),
        )
        .unwrap();

        let request_thread = std::thread::spawn(move || {
            let mut requ_fd = Mfd::create("requ").unwrap();
            let mut resp_fd = Mfd::create("resp").unwrap();
            let event_fd = eventfd(0, EfdFlags::empty()).unwrap();

            // Initialize the request fd
            requ_fd
                .write(
                    &SyscallRequ {
                        id: ApexSyscall::Start,
                        params: vec![1, 2, 3],
                    }
                    .serialize()
                    .unwrap(),
                )
                .unwrap();
            requ_fd.finalize(Seals::Readable).unwrap();

            // Send the fds to the responder
            {
                let fds = [requ_fd.get_fd(), resp_fd.get_fd(), event_fd];
                let cmsg = [ControlMessage::ScmRights(&fds)];
                let buffer = 0_u64.to_be_bytes();
                let iov = [IoSlice::new(buffer.as_slice())];
                sendmsg::<()>(requester, &iov, &cmsg, MsgFlags::empty(), None).unwrap();
            }

            // Wait for a response
            {
                let poller = Poller::new().unwrap();
                let mut events = Vec::with_capacity(1);
                poller.add(event_fd, Event::readable(0)).unwrap();
                poller.wait(&mut events, None).unwrap();
                assert_eq!(events.len(), 1);
            }

            let resp = SyscallResp::deserialize(&resp_fd.read_all().unwrap()).unwrap();
            assert_eq!(resp.id, ApexSyscall::Start);
            assert_eq!(resp.status, 0);
        });

        let response_thread = std::thread::spawn(move || {
            let n = handle(responder, Some(Duration::from_secs(1))).unwrap();
            assert_eq!(n, 1);
        });

        request_thread.join().unwrap();
        response_thread.join().unwrap();
    }
}
