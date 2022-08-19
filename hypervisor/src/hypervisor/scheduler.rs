use std::os::unix::prelude::{AsRawFd, OwnedFd};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use apex_hal::prelude::{OperatingMode, StartCondition};
use linux_apex_core::error::{LeveledResult, ResultExt, TypedResult, SystemError};
use linux_apex_core::health_event::PartitionCall;
use polling::{Event, Poller};

use super::partition::{Base, Run};

pub(crate) struct Timeout {
    start: Instant,
    stop: Duration,
}

impl Timeout {
    pub fn new(start: Instant, stop: Duration) -> Self {
        Self { start, stop }
    }

    fn remaining_time(&self) -> Duration {
        self.stop.saturating_sub(self.start.elapsed())
    }

    fn time_left(&self) -> bool {
        self.remaining_time() > Duration::ZERO
    }
}

pub(crate) struct PartitionTimeWindow<'a> {
    base: &'a mut Base,
    run: &'a mut Option<Run>,
    timeout: Timeout,
}

impl<'a> PartitionTimeWindow<'a> {
    pub fn new(
        base: &'a mut Base,
        run: &'a mut Option<Run>,
        timeout: Timeout,
    ) -> PartitionTimeWindow<'a> {
        PartitionTimeWindow { base, run, timeout }
    }

    fn init_run<'b>(base: &'_ Base, run: &'b mut Option<Run>) -> TypedResult<&'b mut Run> {
        match run {
            Some(run) => Ok(run),
            run @ None => {
                let new_run = Run::new(base, StartCondition::NormalStart, false).typ_res(SystemError::PartitionInit)?;
                run.get_or_insert(new_run);
                Ok(run.as_mut().unwrap())
            }
        }
    }

    pub fn run(&mut self) -> TypedResult<()> {
        // Stop if the time is already over
        if !self.timeout.time_left() {
            return Ok(());
        }

        // Init Run variable (only replaces run if it was none)
        let run = Self::init_run(self.base, self.run)?;

        // If we are in the normal mode at the beginning of the time frame,
        // only then we may schedule the periodic process inside a partition
        if let OperatingMode::Normal = run.mode() {
            Self::run_periodic(self.base, run, &self.timeout)?;
        }

        // Only continue if we have time left
        if self.timeout.time_left() {
            Self::run_post_periodic(self.base, run, &self.timeout)?;
        }
        Ok(())
    }

    fn run_post_periodic(base: &Base, run: &mut Run, timeout: &Timeout) -> TypedResult<()> {
        // if we are in the idle mode, just sleep until the end of the frame
        if let OperatingMode::Idle = run.mode() {
            sleep(timeout.remaining_time());
        } else {
            // Else we are in either a start mode or normal mode (post periodic/mid aperiodic time frame)
            // Either-way we are supposed to unfreeze the partition
            base.unfreeze().typ_res(SystemError::CGroup)?;

            let mut leftover = timeout.remaining_time();
            while leftover > Duration::ZERO {
                match run.receiver().try_recv_timeout(timeout.remaining_time())
                  .typ_res(SystemError::Panic)? {
                    // TODO Error Handling with HM
                    Some(e @ PartitionCall::Error(_)) => e.print_partition_log(base.name()),
                    Some(m @ PartitionCall::Message(_)) => m.print_partition_log(base.name()),
                    Some(PartitionCall::Transition(mode)) => {
                        // In case of a transition to idle, just sleep. Do not care for the rest
                        if let Some(OperatingMode::Idle) =
                            run.handle_transition(base, mode).unwrap()
                        {
                            sleep(timeout.remaining_time());
                            return Ok(());
                        }
                    }
                    None => {}
                }

                leftover = timeout.remaining_time();
            }
        }

        run.freeze_aperiodic().typ_res(SystemError::CGroup)?;

        Ok(())
    }

    fn run_periodic(base: &Base, run: &mut Run, timeout: &Timeout) -> TypedResult<()> {
        run.unfreeze_periodic().typ_res(SystemError::CGroup)?;
        let mut poller = PeriodicPoller::new(run).typ_res(SystemError::CGroup)?;
        // TODO add ? again
        base.unfreeze().typ_res(SystemError::CGroup)?;

        let mut leftover = timeout.remaining_time();
        while leftover > Duration::ZERO {
            let event = poller.wait_timeout(run, timeout.remaining_time()).unwrap();
            match event {
                PeriodicEvent::Timeout => {}
                PeriodicEvent::Frozen => {
                    // In case of a frozen periodic cgroup, we may start the aperiodic process
                    run.unfreeze_aperiodic().typ_res(SystemError::CGroup)?;
                    return Ok(());
                }
                // TODO Error Handling with HM
                PeriodicEvent::Call(e @ PartitionCall::Error(_)) => {
                    e.print_partition_log(base.name())
                }
                PeriodicEvent::Call(c @ PartitionCall::Message(_)) => {
                    c.print_partition_log(base.name())
                }
                PeriodicEvent::Call(PartitionCall::Transition(mode)) => {
                    // Only exit run_periodic, if we changed our mode
                    if run.handle_transition(base, mode)?.is_some() {
                        return Ok(());
                    }
                }
            }

            leftover = timeout.remaining_time();
        }

        // TODO being here means that we exceeded the timeout
        // So we should return a SystemError stating that the time was exceeded
        Ok(())
    }
}

pub(crate) struct PeriodicPoller {
    poll: Poller,
    events: OwnedFd,
}

pub enum PeriodicEvent {
    Timeout,
    Frozen,
    Call(PartitionCall),
}

impl PeriodicPoller {
    const EVENTS_ID: usize = 1;
    const RECEIVER_ID: usize = 2;

    pub fn new(run: &Run) -> Result<PeriodicPoller> {
        let events = run.periodic_events()?;

        let poll = Poller::new()?;
        poll.add(events.as_raw_fd(), Event::readable(Self::EVENTS_ID))?;
        poll.add(
            run.receiver().as_raw_fd(),
            Event::readable(Self::RECEIVER_ID),
        )?;

        Ok(PeriodicPoller { poll, events })
    }

    pub fn wait_timeout(&mut self, run: &mut Run, timeout: Duration) -> Result<PeriodicEvent> {
        let start = Instant::now();

        if run.is_periodic_frozen().unwrap() {
            return Ok(PeriodicEvent::Frozen);
        }

        let mut leftover = timeout.saturating_sub(start.elapsed());
        while leftover > Duration::ZERO {
            let mut events = vec![];
            self.poll.wait(events.as_mut(), Some(leftover))?;

            for e in events {
                match e.key {
                    // Got a Frozen event
                    Self::EVENTS_ID => {
                        // Re-sub the readable event
                        self.poll
                            .modify(self.events.as_raw_fd(), Event::readable(Self::EVENTS_ID))?;

                        // Then check if the cg is actually frozen
                        if run.is_periodic_frozen()? {
                            return Ok(PeriodicEvent::Frozen);
                        }
                    }
                    // got a call events
                    Self::RECEIVER_ID => {
                        // Re-sub the readable event
                        // This will result in the event instantly being ready again should we have something to read,
                        // but that is better than accidentally missing an event (at the expense of one extra loop per receive)
                        self.poll.modify(
                            run.receiver().as_raw_fd(),
                            Event::readable(Self::RECEIVER_ID),
                        )?;

                        // Now receive anything
                        if let Some(call) = run.receiver().try_recv()? {
                            return Ok(PeriodicEvent::Call(call));
                        }
                    }
                    _ => return Err(anyhow!("Unexpected Event Received: {e:?}")),
                }
            }

            leftover = timeout.saturating_sub(start.elapsed());
        }

        Ok(PeriodicEvent::Timeout)
    }
}
