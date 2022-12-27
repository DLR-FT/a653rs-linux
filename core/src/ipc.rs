//! Implementation of IPC
use std::io::{ErrorKind, IoSlice, IoSliceMut};
use std::marker::PhantomData;
use std::os::unix::net::UnixDatagram;
use std::os::unix::prelude::{AsRawFd, FromRawFd, RawFd};
use std::time::Duration;

use anyhow::Error;
use nix::cmsg_space;
use nix::errno::Errno;
use nix::sys::socket::{
    recvmsg, sendmsg, socketpair, AddressFamily, ControlMessage, ControlMessageOwned, MsgFlags,
    SockFlag, SockType,
};
use polling::{Event, Poller};
use serde::{Deserialize, Serialize};

use crate::error::{ResultExt, SystemError, TypedResult};

#[derive(Debug)]
/// Internal data type for the IPC sender
pub struct IpcSender<T> {
    socket: UnixDatagram,
    _p: PhantomData<T>,
}

#[derive(Debug)]
/// Internal data type for the IPC receiver
pub struct IpcReceiver<T> {
    socket: UnixDatagram,
    _p: PhantomData<T>,
}

impl<T> IpcSender<T>
where
    T: Serialize,
{
    /// Sends value alongside the IpcSender
    /// This fails if the resource is temporarily not available.
    pub fn try_send(&self, value: &T) -> TypedResult<()> {
        self.socket
            .send(bincode::serialize(value).typ(SystemError::Panic)?.as_ref())
            .typ(SystemError::Panic)?;
        Ok(())
    }

    /// Try sending value alongside the IpcSender for a certain duration
    pub fn try_send_timeout(&self, _value: &T, _duration: Duration) -> TypedResult<bool> {
        todo!()
    }
}

impl<T> IpcReceiver<T>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    /// Reads a single instance of T from the IpcReceiver
    pub fn try_recv(&self) -> TypedResult<Option<T>> {
        let mut buffer = vec![0; 65507];
        let len = match self.socket.recv(&mut buffer) {
            Ok(len) => len,
            Err(e) if e.kind() != ErrorKind::TimedOut => {
                return Err(Error::from(e)).typ(SystemError::Panic)
            }
            _ => return Ok(None),
        };

        // Serialize the received data into T
        bincode::deserialize(&buffer[0..len])
            .map(|r| Some(r))
            .typ(SystemError::Panic)
    }

    /// Reads a single instance of T from the IpcReceiver but fail after
    /// duration
    pub fn try_recv_timeout(&self, duration: Duration) -> TypedResult<Option<T>> {
        let poller = Poller::new().typ(SystemError::Panic)?;
        poller
            .add(self.socket.as_raw_fd(), Event::readable(42))
            .typ(SystemError::Panic)?;

        let poll_res = poller.wait(Vec::new().as_mut(), Some(duration));
        if let Err(_) | Ok(0) = poll_res {
            return Ok(None);
        }

        self.try_recv()
    }
}

/// Create a pair consisting of an IpcSender and an IpcReceiver
pub fn channel_pair<T>() -> TypedResult<(IpcSender<T>, IpcReceiver<T>)>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    trace!("Create IPC channel pair");
    let (tx, rx) = socketpair(
        AddressFamily::Unix,
        SockType::Datagram,
        None,
        SockFlag::SOCK_NONBLOCK,
    )
    .typ(SystemError::Panic)?;

    unsafe {
        let tx = IpcSender::from_raw_fd(tx);
        let rx = IpcReceiver::from_raw_fd(rx);

        Ok((tx, rx))
    }
}

impl<T> AsRawFd for IpcSender<T> {
    fn as_raw_fd(&self) -> RawFd {
        self.socket.as_raw_fd()
    }
}

impl<T> AsRawFd for IpcReceiver<T> {
    fn as_raw_fd(&self) -> RawFd {
        self.socket.as_raw_fd()
    }
}

impl<T> FromRawFd for IpcSender<T> {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self {
            socket: UnixDatagram::from_raw_fd(fd),
            _p: PhantomData,
        }
    }
}

impl<T> FromRawFd for IpcReceiver<T> {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self {
            socket: UnixDatagram::from_raw_fd(fd),
            _p: PhantomData,
        }
    }
}

/// Creates a pair of sockets that are meant for passing file descriptors to
/// partitions.
pub fn io_pair<T>() -> TypedResult<(IoSender<T>, IoReceiver<T>)> {
    let (tx, rx) = socketpair(
        AddressFamily::Unix,
        SockType::Datagram,
        None,
        SockFlag::empty(),
    )
    .typ(SystemError::Panic)?;
    unsafe { Ok((IoSender::from_raw_fd(tx), IoReceiver::from_raw_fd(rx))) }
}

#[derive(Debug)]
/// Internal data type for the IO resource sender
pub struct IoSender<T> {
    socket: UnixDatagram,
    _p: PhantomData<T>,
}

impl<T> IoSender<T>
where
    T: AsRawFd,
{
    /// Sends a resource to the receiving socket.
    pub fn try_send(&self, resource: impl AsRawFd) -> TypedResult<()> {
        let fds = [resource.as_raw_fd()];
        let cmsg = [ControlMessage::ScmRights(&fds)];
        let buffer = [0u8; 1];
        let iov = [IoSlice::new(buffer.as_slice())];
        let io_fd = self.socket.as_raw_fd();
        sendmsg::<()>(io_fd, &iov, &cmsg, MsgFlags::empty(), None).typ(SystemError::Panic)?;
        Ok(())
    }
}

impl<T> IoReceiver<T>
where
    T: FromRawFd,
{
    /// Returns the next available IO resource.
    /// Returns `None`, if no further resources can be read from the socket.
    ///
    /// # Safety
    /// Only safe if `T` matches the type of the file descriptor.
    pub unsafe fn try_receive(&self) -> TypedResult<Option<T>> {
        let mut cmsg = cmsg_space!(RawFd);
        let mut iobuf = [0u8; 1];
        let mut iov = [IoSliceMut::new(&mut iobuf)];
        let io_fd = self.socket.as_raw_fd();
        match recvmsg::<()>(io_fd, &mut iov, Some(&mut cmsg), MsgFlags::MSG_DONTWAIT) {
            Ok(msg) => {
                if let Some(ControlMessageOwned::ScmRights(fds)) = msg.cmsgs().next() {
                    if let &[raw_fd] = fds.as_slice() {
                        let sock = unsafe { T::from_raw_fd(raw_fd) };
                        return Ok(Some(sock));
                    }
                }
                Ok(None)
            }
            // This should never block since the socket is only written to before the partition
            // starts.
            Err(e) if e != Errno::EAGAIN && e != Errno::EINTR => {
                Err(Error::from(e)).typ(SystemError::Panic)
            }
            _ => Ok(None),
        }
    }
}

#[derive(Debug)]
/// Internal data type for the IO resource sender
pub struct IoReceiver<T> {
    socket: UnixDatagram,
    _p: PhantomData<T>,
}

impl<T> AsRawFd for IoSender<T> {
    fn as_raw_fd(&self) -> RawFd {
        self.socket.as_raw_fd()
    }
}

impl<T> AsRawFd for IoReceiver<T> {
    fn as_raw_fd(&self) -> RawFd {
        self.socket.as_raw_fd()
    }
}

impl<T> FromRawFd for IoSender<T> {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self {
            socket: UnixDatagram::from_raw_fd(fd),
            _p: PhantomData,
        }
    }
}

impl<T> FromRawFd for IoReceiver<T> {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self {
            socket: UnixDatagram::from_raw_fd(fd),
            _p: PhantomData,
        }
    }
}
