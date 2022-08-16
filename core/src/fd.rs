use std::io::ErrorKind;
use std::os::unix::prelude::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Error, Result};
use nix::libc::{c_uint, syscall, SYS_pidfd_open};
use nix::unistd::Pid;
use polling::{Event, Poller};

#[derive(Debug)]
pub struct PidFd(OwnedFd);

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

impl AsRawFd for PidFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl TryFrom<Pid> for PidFd {
    type Error = Error;

    fn try_from(value: Pid) -> Result<Self, Self::Error> {
        let pidfd: i32 =
            unsafe { syscall(SYS_pidfd_open, value.as_raw(), 0 as c_uint).try_into()? };
        if pidfd < 0 {
            return Err(anyhow!("Error getting pidfd from {value}. {pidfd}"));
        }
        Ok(PidFd(unsafe { OwnedFd::from_raw_fd(pidfd) }))
    }
}
