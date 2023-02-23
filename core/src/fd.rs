//! Implementation of PID file descriptors
// TODO: Consider renaming this module to "pidfd" for less ambiguity
// TODO: Remove this, as soon as the following is available in stable Rust:
// https://doc.rust-lang.org/stable/std/os/linux/process/struct.PidFd.html
use std::io::ErrorKind;
use std::os::unix::prelude::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use nix::libc::{c_uint, syscall, SYS_pidfd_open};
use nix::unistd::Pid;
use polling::{Event, Poller};
use thiserror::Error;

use crate::error::{ResultExt, SystemError, TypedError, TypedResult};

#[derive(Error, Debug)]
/// The fundamental error type for this crate
// TODO: Consider replacing it with a normal TypedError and using TimeDurationExceeded instead
pub enum PidWaitError {
    /// A timeout has a occurred
    #[error("Timeout reached")]
    Timeout,
    /// Another error has occurred
    #[error("{0}")]
    Err(anyhow::Error),
}

impl From<anyhow::Error> for PidWaitError {
    fn from(e: anyhow::Error) -> Self {
        Self::Err(e)
    }
}

#[derive(Debug)]
/// The internal type of this module for handling PidFds.
pub struct PidFd {
    pid: Pid,
    fd: OwnedFd,
}

impl PidFd {
    pub fn pid(&self) -> Pid {
        self.pid
    }
    //     /// Returns when the PidFd is ready to be read or if timeout occurred
    //     pub fn wait_signal_timeout(&self, timeout: Duration) -> Result<(), PidWaitError> {
    //         let now = Instant::now();

    //         let poller = Poller::new()
    //             .map_err(anyhow::Error::from)
    //             .typ(SystemError::Panic)?;

    //         loop {
    //             // The second argument to Poller::modify() is totally valid and correct, due to
    //             // epoll(2) internals, which demand providing a "user data variable" -- a feature
    //             // that we make no use of.
    //             poller
    //                 .modify(self.0.as_raw_fd(), Event::readable(42))
    //                 .map_err(anyhow::Error::from)
    //                 .typ(SystemError::Panic)?;

    //             let poll_res = poller.wait(
    //                 Vec::new().as_mut(),
    //                 Some(timeout.saturating_sub(now.elapsed())),
    //             );
    //             match poll_res {
    //                 Ok(0) => return Err(PidWaitError::Timeout),
    //                 Ok(_) => return Ok(()),
    //                 Err(e) => {
    //                     if e.kind() != ErrorKind::Interrupted {
    //                         return Err(e)
    //                             .map_err(anyhow::Error::from)
    //                             .typ(SystemError::Panic)
    //                             .map_err(PidWaitError::Err);
    //                     }
    //                 }
    //             }
    //         }
    //     }
}

impl AsRawFd for PidFd {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl TryFrom<Pid> for PidFd {
    type Error = TypedError;

    fn try_from(pid: Pid) -> TypedResult<Self> {
        let pidfd: std::os::raw::c_int = unsafe {
            syscall(SYS_pidfd_open, pid.as_raw(), 0 as c_uint)
                .try_into()
                .typ(SystemError::Panic)?
        };
        if pidfd < 0 {
            // TODO: pidfd will be -1 in that case. Printing this is not useful. Access errno instead.
            return Err(anyhow!("Error getting pidfd from {pid}. {pidfd}")).typ(SystemError::Panic);
        }
        Ok(PidFd {
            pid,
            fd: unsafe { OwnedFd::from_raw_fd(pidfd) },
        })
    }
}
