use std::ptr::null_mut;
use std::time::Duration;

use nix::libc::sigtimedwait;
use nix::sys::signal::Signal;
use nix::sys::signalfd::SigSet;
use nix::sys::time::TimeSpec;

pub trait SigSetExt {
    fn wait_timeout(&self, timeout: Duration) -> nix::Result<Signal>;
}

impl SigSetExt for SigSet {
    fn wait_timeout(&self, timeout: Duration) -> nix::Result<Signal> {
        loop {
            let res = unsafe {
                sigtimedwait(
                    self.as_ref(),
                    null_mut(),
                    TimeSpec::from_duration(timeout).as_ref(),
                )
            };
        }

        todo!()
    }
}
