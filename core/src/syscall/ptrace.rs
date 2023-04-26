use core::pin::Pin;
use std::collections::HashSet;
use std::fmt::Debug;
use std::task::Context;

use anyhow::{bail, Result};
use futures::task::Poll;
use futures::Stream;
use nix::sys::ptrace;
use nix::sys::signal::{raise, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{setpgid, Pid};
use tokio::signal::unix::{signal as tokio_signal, Signal as SignalListener, SignalKind};

#[derive(Debug)]
pub struct PartitionTrace {
    main: Pid,
    children: HashSet<Pid>,
    signal: SignalListener,
}

impl PartitionTrace {
    pub fn install() -> Result<()> {
        ptrace::traceme()?;
        raise(Signal::SIGSTOP)?;
        Ok(())
    }

    /// Creates new PartitionTrace instance for intercepting/emulating syscalls of a whole partition
    pub async fn new(main: Pid) -> Result<Self> {
        let mut trace = Self {
            main,
            children: Default::default(),
            signal: tokio_signal(SignalKind::from_raw(Signal::SIGCHLD as i32))?,
        };
        // Set pgid of main partition process to its own pid
        setpgid(main, main)?;
        // Wait for process to stop
        match trace.wait().await? {
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
            // Return an error should we have received a different WaitStatus
            status => bail!("Expected main process to be stopped: {status:?}"),
        }
        Ok(trace)
    }

    fn add_process(&mut self, child: Pid) -> Result<()> {
        if !self.children.insert(child) {
            warn!("{child:?} already present");
            return Ok(());
        }
        setpgid(child, self.get_pgid())?;
        Ok(())
    }

    pub fn get_pgid(&self) -> Pid {
        self.main
    }

    /// Wait on any trace trap in this partition
    pub async fn wait(&mut self) -> Result<WaitStatus> {
        let pgid = Pid::from_raw(-self.get_pgid().as_raw());
        loop {
            match waitpid(pgid, Some(WaitPidFlag::WNOHANG))? {
                // If the process is neither stopped nor exited continue
                WaitStatus::StillAlive => {}
                // Any other status should stop this
                st => return Ok(st),
            };
            // Wait for any SIGCHLD
            if self.signal.recv().await.is_none() {
                bail!("SIGCHLD receiver broke")
            }
        }
    }

    // pub fn write_data(&self, )
}

impl Stream for PartitionTrace {
    type Item = WaitStatus;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.signal.poll_recv(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Some(_)) => {}
            _ => panic!("serious problem in the tokio signal stream"),
        }

        let pgid = Pid::from_raw(-self.get_pgid().as_raw());

        // This loop is necessary, because POSIX signals are the worst invention
        // ever since the invention of computers.
        // See this StackOverflow thread:
        // https://stackoverflow.com/questions/8398298/handling-multiple-sigchld
        loop {
            match waitpid(pgid, Some(WaitPidFlag::WNOHANG)).unwrap() {
                // If the process is neither stopped nor exited continue
                WaitStatus::StillAlive => {}
                // Any other status should stop this
                st => return Poll::Ready(Some(st)),
            }
        }
    }
}
