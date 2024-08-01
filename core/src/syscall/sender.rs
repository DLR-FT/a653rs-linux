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
    pub fn from_datagram(datagram: UnixDatagram) -> Self {
        Self(datagram)
    }

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
                Ok(0) => {
                    // The poller's `wait` method may return spuriously.
                    // In that case, just call it again, because the `wait_event` method may only
                    // return an event or an error.
                    continue;
                }
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

    pub fn execute(&self, request: SyscallRequest) -> Result<SyscallResponse> {
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
