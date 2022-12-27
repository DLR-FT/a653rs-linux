//! Implementation of the mechanism to perform system calls

// TODO: Document the mechanism here

use std::io::IoSlice;
use std::os::fd::{AsRawFd, RawFd};

use a653rs_linux_core::mfd::{Mfd, Seals};
use a653rs_linux_core::syscall::{SyscallRequ, SyscallResp};
use anyhow::Result;
use nix::libc::EINTR;
use nix::sys::eventfd::{self, EfdFlags};
use nix::sys::socket::{sendmsg, ControlMessage, MsgFlags};
use polling::{Event, Poller};

use crate::SYSCALL;

/// Sends a vector of file descriptors through a Unix socket
fn send_fds<const COUNT: usize>(hv: RawFd, fds: &[RawFd; COUNT]) -> Result<()> {
    let cmsg = [ControlMessage::ScmRights(fds)];
    let buffer = 0_u64.to_ne_bytes();
    let iov = [IoSlice::new(buffer.as_slice())];
    sendmsg::<()>(hv, &iov, &cmsg, MsgFlags::empty(), None)?;
    Ok(())
}

/// Waits for action on the event fd
// TODO: Consider timeout
fn wait_event(event_fd: RawFd) -> Result<()> {
    let poller = Poller::new()?;
    let mut events = Vec::with_capacity(1);
    poller.add(event_fd, Event::readable(0))?;

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

fn execute_fd(fd: RawFd, requ: SyscallRequ) -> Result<SyscallResp> {
    // Create the file descriptor triple
    let mut requ_fd = Mfd::create("requ")?;
    let mut resp_fd = Mfd::create("resp")?;
    let event_fd = eventfd::eventfd(0, EfdFlags::empty())?;

    // Initialize the request file descriptor
    requ_fd.write(&requ.serialize()?)?;
    requ_fd.finalize(Seals::Readable)?;

    // Send the file descriptors to the hypervisor
    send_fds(
        fd,
        &[requ_fd.get_fd(), resp_fd.get_fd(), event_fd.as_raw_fd()],
    )?;

    wait_event(event_fd)?;

    let resp = SyscallResp::deserialize(&resp_fd.read_all()?)?;
    Ok(resp)
}

pub fn execute(requ: SyscallRequ) -> Result<SyscallResp> {
    execute_fd(*SYSCALL, requ)
}

#[cfg(test)]
mod tests {
    use std::io::IoSliceMut;

    use a653rs_linux_core::syscall::ApexSyscall;
    use nix::sys::socket::{
        recvmsg, socketpair, AddressFamily, ControlMessageOwned, SockFlag, SockType,
    };
    use nix::{cmsg_space, unistd};

    use super::*;

    #[test]
    fn test_execute() {
        let (requester, responder) = socketpair(
            AddressFamily::Unix,
            SockType::Datagram,
            None,
            SockFlag::empty(),
        )
        .unwrap();

        let request_thread = std::thread::spawn(move || {
            let resp = execute_fd(
                requester,
                SyscallRequ {
                    id: ApexSyscall::Start,
                    params: vec![1, 2, 42],
                },
            )
            .unwrap();

            assert_eq!(resp.id, ApexSyscall::Start);
            assert_eq!(resp.status, 42);
        });
        let response_thread = std::thread::spawn(move || {
            // Receive the file descriptors
            let mut cmsg = cmsg_space!([RawFd; 3]);
            let mut iobuf = [0u8];
            let mut iov = [IoSliceMut::new(&mut iobuf)];
            let res =
                recvmsg::<()>(responder, &mut iov, Some(&mut cmsg), MsgFlags::empty()).unwrap();

            let fds: Vec<RawFd> = match res.cmsgs().next().unwrap() {
                ControlMessageOwned::ScmRights(fds) => fds.to_vec(),
                _ => panic!("unknown cmsg received"),
            };

            let mut requ_fd = Mfd::from_fd(fds[0]).unwrap();
            let mut resp_fd = Mfd::from_fd(fds[1]).unwrap();
            let event_fd = fds[2];

            // Fetch the request
            let requ = SyscallRequ::deserialize(&requ_fd.read_all().unwrap()).unwrap();
            assert_eq!(requ.id, ApexSyscall::Start);
            assert_eq!(requ.params, vec![1, 2, 42]);

            // Write the response
            resp_fd
                .write(
                    &SyscallResp {
                        id: ApexSyscall::Start,
                        status: 42,
                    }
                    .serialize()
                    .unwrap(),
                )
                .unwrap();
            resp_fd.finalize(Seals::Readable).unwrap();

            // Trigger the eventfd
            let buf = 1_u64.to_ne_bytes();
            unistd::write(event_fd, &buf).unwrap();
        });

        request_thread.join().unwrap();
        response_thread.join().unwrap();
    }
}
