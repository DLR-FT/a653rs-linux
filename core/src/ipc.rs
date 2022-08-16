use std::{os::unix::{net::UnixDatagram, prelude::{AsRawFd, RawFd, FromRawFd}}, marker::PhantomData, time::{Duration, Instant}, io::ErrorKind};
use anyhow::{Result, anyhow, Error};
use nix::sys::socket::{socketpair, AddressFamily, SockType, SockFlag};
use polling::{Poller, Event};
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub struct IpcSender<T>{
  socket: UnixDatagram,
  _p: PhantomData<T>,
}

#[derive(Debug)]
pub struct IpcReceiver<T>{
  socket: UnixDatagram,
  _p: PhantomData<T>,
}

impl<T> IpcSender<T> where T: Serialize{
  pub fn try_send(&self, value: &T) -> Result<()>{
    let len = self.socket.send(bincode::serialize(value)?.as_ref())?;
    info!("Send sucessfully {len}");
    Ok(())
  }

  pub fn try_send_timeout(&self, value: &T, duration: Duration) -> Result<bool>{
    todo!()
  }
}

impl<T> IpcReceiver<T> where T: for<'de> Deserialize<'de> + Serialize {
  pub fn try_recv_timeout(&self, duration: Duration) -> Result<Option<T>>{
    let poller = Poller::new().unwrap();
    poller.add(self.socket.as_raw_fd(), Event::readable(0)).unwrap();
  
    let poll_res = poller.wait(
        Vec::new().as_mut(),
        Some(duration),
    );
    if let Err(_) | Ok(0) = poll_res{
      return Ok(None)
    }
  
    let mut buffer = vec![0; 65507];
    let len = match self.socket.recv(&mut buffer){
      Ok(len) => len,
      Err(e) if e.kind() != ErrorKind::TimedOut =>
        return Err(e.into()),
      _ => 
        return Ok(None)
      
    };

    bincode::deserialize(&buffer[0..len]).map(|r| Some(r)).map_err(Error::from)
  }
}

pub fn channel_pair<T>() -> std::io::Result<(IpcSender<T>, IpcReceiver<T>)> where T: for<'de> Deserialize<'de> + Serialize {
  let (tx, rx) = socketpair(AddressFamily::Unix, SockType::Datagram, None, SockFlag::SOCK_NONBLOCK)?;

  unsafe {
    let tx = IpcSender::from_raw_fd(tx);
    let rx = IpcReceiver::from_raw_fd(rx);
  
    Ok((tx, rx))
  }
}

impl<T> AsRawFd for IpcSender<T>{
    fn as_raw_fd(&self) -> RawFd {
        self.socket.as_raw_fd()
    }
}

impl<T> AsRawFd for IpcReceiver<T>{
    fn as_raw_fd(&self) -> RawFd {
      self.socket.as_raw_fd()
    }
}

impl<T> FromRawFd for IpcSender<T>{
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self { 
          socket: UnixDatagram::from_raw_fd(fd), 
          _p: Default::default(),
        }
    }
}

impl<T> FromRawFd for IpcReceiver<T>{
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
      Self { 
        socket: UnixDatagram::from_raw_fd(fd), 
        _p: Default::default(),
      }
    }
}