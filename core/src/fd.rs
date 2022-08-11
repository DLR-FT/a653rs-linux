use std::io::ErrorKind;
use std::mem::forget;
use std::os::unix::prelude::{AsRawFd, RawFd};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Error, Result};
use nix::libc::{c_uint, syscall, SYS_pidfd_open};
use nix::unistd::{close, Pid};
use polling::{Event, Poller};

#[derive(Debug)]
pub struct Fd(RawFd);

impl Fd {
    pub fn forget(self) {
        forget(self)
    }
}

impl TryFrom<RawFd> for Fd {
    type Error = Error;

    fn try_from(value: RawFd) -> Result<Self, Self::Error> {
        if value < 0 {
            return Err(anyhow!("Invalid fd: {value}"));
        }
        Ok(Fd(value))
    }
}

impl Drop for Fd {
    fn drop(&mut self) {
        close(self.0).ok();
    }
}

impl AsRawFd for Fd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

#[derive(Debug)]
pub struct PidFd(Fd);

impl PidFd {
    pub fn wait_exited_timeout(&self, timeout: Duration) -> Result<bool> {
        let now = Instant::now();
        let poller = Poller::new()?;
        poller.add(self.0.as_raw_fd(), Event::readable(0)).unwrap();

        loop {
            let poll_res = poller.wait(
                Vec::new().as_mut(),
                Some(timeout.saturating_sub(now.elapsed())),
            );
            match poll_res {
                Ok(events) => return Ok(events > 0),
                Err(e) => {
                    if e.kind() != ErrorKind::Interrupted {
                        return Err(e.into());
                    }
                }
            }
            poller
                .modify(self.0.as_raw_fd(), Event::readable(0))
                .unwrap();
        }
    }
}

impl TryFrom<Pid> for PidFd {
    type Error = Error;

    fn try_from(value: Pid) -> Result<Self, Self::Error> {
        let pidfd: i32 =
            unsafe { syscall(SYS_pidfd_open, value.as_raw(), 0 as c_uint).try_into()? };
        let fd =
            Fd::try_from(pidfd).map_err(|e| anyhow!("Error getting pidfd from {value}. {e:#?}"))?;
        Ok(PidFd(fd))
    }
}
