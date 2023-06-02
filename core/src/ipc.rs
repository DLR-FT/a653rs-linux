//! Implementation of IPC
use std::io::ErrorKind;
use std::marker::PhantomData;
use std::os::unix::net::UnixDatagram;
use std::os::unix::prelude::{AsRawFd, FromRawFd, RawFd};
use std::time::Duration;

use anyhow::Error;
use nix::sys::socket::{socketpair, AddressFamily, SockFlag, SockType};
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

    /// Reads a single instance of T from the IpcReceiver but fail after duration
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
