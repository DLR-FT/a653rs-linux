use std::fmt::Debug;
use std::io::ErrorKind;
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

use anyhow::bail;
use nix::sys::ptrace;
use nix::sys::signal::{raise, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use polling::{Event, Poller};

use crate::fd::{PidFd, PidWaitError};

#[derive(Debug)]
pub struct PartitionTrace {
    main: PidFd,
    children: Vec<PidFd>,
    poller: Poller,
}

// impl Debug for PartitionTrace{
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         f.debug_list().entries(self.0.iter()).finish()
//     }
// }

impl PartitionTrace {
    pub fn install() -> anyhow::Result<()> {
        ptrace::traceme()?;
        raise(Signal::SIGSTOP)?;
        Ok(())
    }

    /// Creates new PartitionTrace instance for intercepting/emulating syscalls of a whole partition
    pub fn new(main: Pid, timeout: Duration) -> anyhow::Result<Self> {
        let trace = Self {
            main: main.try_into()?,
            children: vec![],
            poller: Poller::new()?,
        };
        trace.poller.add(
            trace.main.as_raw_fd(),
            Event::readable(trace.main.pid().as_raw() as usize),
        )?;
        // Wait for process to stop
        match trace.wait(timeout)? {
            // We explicitly expect the tracee to be stopped and nothing else
            WaitStatus::Stopped(_, _) => {
                use ptrace::Options as TOptions;
                ptrace::setoptions(
                    main,
                    TOptions::PTRACE_O_TRACEFORK
                        | TOptions::PTRACE_O_TRACEVFORK
                        | TOptions::PTRACE_O_TRACECLONE
                        | TOptions::PTRACE_O_EXITKILL,
                )?;
                // Stop the process on the next syscall attempt
                ptrace::syscall(main, None)?;
            }
            // Return an error should a different WaitStatus was received
            status => bail!("Expected main process to be stopped: {status:?}"),
        }
        Ok(trace)
    }

    /// Wait on any trace trap in this partition with a timeout
    pub fn wait(&self, timeout: Duration) -> Result<WaitStatus, PidWaitError> {
        let now = Instant::now();
        loop {
            // Refresh poller events
            for p in std::iter::once(&self.main).chain(&self.children) {
                self.poller
                    .modify(p.as_raw_fd(), Event::readable(p.pid().as_raw() as usize))
                    .map_err(|e| PidWaitError::Err(e.into()))?;
            }

            // Wait on any stopped process
            let poll_res = self.poller.wait(
                Vec::new().as_mut(),
                Some(timeout.saturating_sub(now.elapsed())),
            );
            match poll_res {
                // On timeout exit with timeouterror
                Ok(0) => return Err(PidWaitError::Timeout),
                Ok(pid) => {
                    // On success do a nohang wait for obtaining the WaitStatus
                    match waitpid(Pid::from_raw(pid as i32), Some(WaitPidFlag::WNOHANG)) {
                        Ok(st) => return Ok(st),
                        // Should this fail, debug print and poll again
                        Err(e) => debug!("Unexpected error on waitpid: {e:?}"),
                    };
                }
                Err(e) => match e.kind() {
                    // On error, only care for the interrupted error kind
                    ErrorKind::Interrupted => {
                        return Err(e).map_err(|e| PidWaitError::Err(e.into()))
                    }
                    // Else debug print and continue
                    e => debug!("Unexpected polling error: {e:?}"),
                },
            }
        }
    }

    // pub fn write_data(&self, )
}
