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

use super::SyscallType;
use crate::mfd::{Mfd, Seals};
use crate::syscall::syscalls::Syscall;
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
    pub fn receive_one(
        &self,
        timeout: Option<Duration>,
        handler: impl FnOnce(SyscallType, &[u8]) -> Vec<u8>,
    ) -> Result<bool> {
        if self.wait_fds(timeout)? {
            let [request_fd, resp_fd, event_fd] = self.recv_fd_triple()?;
            let mut request_fd = Mfd::from_fd(request_fd)?;
            let mut response_fd = Mfd::from_fd(resp_fd)?;

            // TODO Refactor this function to not be able to return unless a response has
            // been sent. The partition is blocked and is waiting for
            // response. Thus returning here would discard the received
            // message and the partition will never unblock.

            // Fetch the request
            let serialized_payload = request_fd.read_all()?;

            // Deserialize the type and data
            let payload: SyscallRequest = bincode::deserialize(&serialized_payload)?;

            let serialized_response = handler(payload.0, &payload.1);

            // Write the response
            response_fd.write(&serialized_response)?;
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
    pub fn receive_all<F, Params, Returns>(
        &self,
        timeout: Option<Duration>,
        mut handler: impl FnMut(SyscallType, &[u8]) -> Vec<u8>,
    ) -> Result<usize>
where {
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
    ///
    /// Note: Even if timeout is None, this can return Ok(false). See
    /// [Poller::wait]
    fn wait_fds(&self, timeout: Option<Duration>) -> Result<bool> {
        let end_of_timeout = timeout.map(|duration| Instant::now() + duration);

        // This closure calculates the duratioon until the end of timeout is reached.
        // It is necessary, so that the timeout duration can be recalculated in case the
        // `poller.wait` method returns spuriously.
        let remaining_timeout_duration =
            || end_of_timeout.map(|end| end.duration_since(Instant::now()));

        let poller = Poller::new()?;
        let mut events = Events::with_capacity(NonZeroUsize::MIN);
        unsafe { poller.add(self.0.as_raw_fd(), Event::readable(0))? };
        loop {
            match poller.wait(&mut events, remaining_timeout_duration()) {
                Ok(0) => {
                    // The poller's `wait` method may return spuriously.
                    // In that case call it again. `remaining_time_duration` will automatically
                    // calculate the new timeout duration.
                    continue;
                }
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

/// A helper function that is used to:
/// 1. deserialize parameters for a given syscall type,
/// 2. execute a function `f` on the parameters and
/// 3. serialize the returned response
///
/// Its usage is mainly to abstract away the underlying
/// serialization implementation.
pub fn wrap_serialization<'params, S: Syscall<'params>, F>(
    serialized_params: &'params [u8],
    f: F,
) -> Result<Vec<u8>>
where
    F: FnOnce(S::Params) -> Result<S::Returns, a653rs::bindings::ErrorReturnCode>,
{
    let params: S::Params = bincode::deserialize(serialized_params)?;

    let response: SyscallResponse<S::Returns> = f(params);

    bincode::serialize(&response).map_err(Into::into)
}
