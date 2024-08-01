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
    pub fn from_datagram(datagram: UnixDatagram) -> Self {
        Self(datagram)
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let socket = UnixDatagram::bind(path)?;
        socket.set_nonblocking(true)?;
        Ok(Self(socket))
    }

    /// Returns whether a syscall was handled
    pub fn receive_one<F>(&self, timeout: Option<Duration>, handler: F) -> Result<bool>
    where
        F: FnOnce(SyscallRequest) -> SyscallResponse,
    {
        if self.wait_fds(timeout)? {
            let [request_fd, resp_fd, event_fd] = self.recv_fd_triple()?;
            let mut request_fd = Mfd::from_fd(request_fd)?;
            let mut response_fd = Mfd::from_fd(resp_fd)?;

            // Fetch the request
            let request: SyscallRequest = bincode::deserialize(&request_fd.read_all()?)?;
            let request_id = request.id;

            trace!("Handling system call {:?}", request.id);
            let response = handler(request);

            assert_eq!(request_id, response.id);

            // Write the response
            response_fd.write(&bincode::serialize(&response)?)?;
            response_fd.finalize(Seals::Readable)?;

            // Trigger the event
            let buf = 1_u64.to_ne_bytes();
            unistd::write(event_fd, &buf)?;

            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Returns the number of syscalls handled
    pub fn receive_all<F>(&self, timeout: Option<Duration>, mut handler: F) -> Result<usize>
    where
        F: FnMut(SyscallRequest) -> SyscallResponse,
    {
        let start = Instant::now();

        let mut num_syscalls = 0;
        // A loop in which each iteration resembles the execution of one syscall
        loop {
            let remaining_time = timeout.map(|timeout| timeout.saturating_sub(start.elapsed()));

            if let Some(Duration::ZERO) = remaining_time {
                break;
            }
            if self.receive_one(remaining_time, &mut handler)? {
                num_syscalls += 1;
            }
        }

        Ok(num_syscalls)
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
}
