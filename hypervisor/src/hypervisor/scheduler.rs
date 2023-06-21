use std::os::unix::prelude::{AsRawFd, OwnedFd};
use std::thread::sleep;
use std::time::{Duration, Instant};

use a653rs::prelude::{OperatingMode, StartCondition};
use a653rs_linux_core::error::{
    ErrorLevel, LeveledResult, ResultExt, SystemError, TypedError, TypedResult, TypedResultExt,
};
use a653rs_linux_core::health::{ModuleRecoveryAction, RecoveryAction};
use a653rs_linux_core::health_event::PartitionCall;
use anyhow::anyhow;
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
    base: &'a Base,
    run: &'a mut Run,
    timeout: Timeout,
}

impl<'a> PartitionTimeWindow<'a> {
    pub fn new(base: &'a Base, run: &'a mut Run, timeout: Timeout) -> PartitionTimeWindow<'a> {
        PartitionTimeWindow { base, run, timeout }
    }

    fn handle_part_err(&mut self, res: TypedResult<()>) -> LeveledResult<()> {
        debug!("Partition \"{}\" received err: {res:?}", self.base.name());
        if let Err(err) = res.as_ref() {
            let now = Instant::now();

            let action = match self.base.part_hm().try_action(err.err()) {
                None => {
                    warn!("Could not map \"{res:?}\" to action. Using Panic action instead");
                    match self.base.part_hm().panic {
                        // We do not Handle Module Recovery actions here
                        RecoveryAction::Module(_) => return res.lev(ErrorLevel::Partition),
                        RecoveryAction::Partition(action) => action,
                    }
                }
                // We do not Handle Module Recovery actions here
                Some(RecoveryAction::Module(_)) => return res.lev(ErrorLevel::Partition),
                Some(RecoveryAction::Partition(action)) => action,
            };

            debug!("Handling: {err:?}");
            debug!("Apply Partition Recovery Action: {action:?}");

            // TODO do not unwrap/expect these errors. Maybe raise Module Level
            // PartitionInit Error?
            match action {
                a653rs_linux_core::health::PartitionRecoveryAction::Idle => self
                    .run
                    .idle_transition(self.base)
                    .expect("Idle Transition Failed"),
                a653rs_linux_core::health::PartitionRecoveryAction::ColdStart => self
                    .run
                    .start_transition(self.base, false, StartCondition::HmPartitionRestart)
                    .expect("Start(Cold) Transition Failed"),
                a653rs_linux_core::health::PartitionRecoveryAction::WarmStart => self
                    .run
                    .start_transition(self.base, false, StartCondition::HmPartitionRestart)
                    .expect("Start(Warm) Transition Failed"),
            }

            trace!("Partition Error Handling took: {:?}", now.elapsed())
        }

        Ok(())
    }

    pub fn run(&mut self) -> LeveledResult<()> {
        // Stop if the time is already over
        if !self.timeout.time_left() {
            return Ok(());
        }

        // If we are in the normal mode at the beginning of the time frame,
        // only then we may schedule the periodic process inside a partition
        if let OperatingMode::Normal = self.run.mode() {
            let res = self.run.unfreeze_periodic();
            let res = match res {
                Ok(true) => self.run_periodic(),
                // Check if there is no periodic process
                Ok(false) => {
                    self.run.unfreeze_aperiodic().lev(ErrorLevel::Partition)?;
                    Ok(())
                }
                Err(e) => Err(e),
            };
            self.handle_part_err(res)?;
        }

        // Only continue if we have time left
        if self.timeout.time_left() {
            let res = self.run_post_periodic();
            self.handle_part_err(res)?;
        }
        Ok(())
    }

    fn run_post_periodic(&mut self) -> TypedResult<()> {
        // if we are in the idle mode, just sleep until the end of the frame
        if let OperatingMode::Idle = self.run.mode() {
            sleep(self.timeout.remaining_time());
        } else {
            // Else we are in either a start mode or normal mode (post periodic/mid
            // aperiodic time frame) Either-way we are supposed to unfreeze the
            // partition
            self.base.unfreeze()?;

            let mut leftover = self.timeout.remaining_time();
            while leftover > Duration::ZERO {
                match &self
                    .run
                    .receiver()
                    .try_recv_timeout(self.timeout.remaining_time())?
                {
                    Some(m @ PartitionCall::Message(_)) => m.print_partition_log(self.base.name()),
                    Some(e @ PartitionCall::Error(se)) => {
                        e.print_partition_log(self.base.name());
                        match self.base.part_hm().try_action(*se){
                            Some(RecoveryAction::Module(ModuleRecoveryAction::Ignore)) => {},
                            Some(_) => return Err(TypedError::new(*se, anyhow!("Received Partition Error"))) ,
                            None =>  return Err(TypedError::new(SystemError::Panic, anyhow!("Could not get recovery action for requested partition error: {se}"))),
                        };
                    }
                    Some(t @ PartitionCall::Transition(mode)) => {
                        // In case of a transition to idle, just sleep. Do not care for the rest
                        t.print_partition_log(self.base.name());
                        if let Some(OperatingMode::Idle) =
                            self.run.handle_transition(self.base, *mode)?
                        {
                            sleep(self.timeout.remaining_time());
                            return Ok(());
                        }
                    }
                    None => {}
                }

                leftover = self.timeout.remaining_time();
            }
        }

        self.run.freeze_aperiodic()?;

        Ok(())
    }

    fn run_periodic(&mut self) -> TypedResult<()> {
        let mut poller = PeriodicPoller::new(self.run)?;

        self.base.unfreeze()?;

        let mut leftover = self.timeout.remaining_time();
        while leftover > Duration::ZERO {
            let event = poller.wait_timeout(self.run, self.timeout.remaining_time())?;
            match &event {
                PeriodicEvent::Timeout => {}
                PeriodicEvent::Frozen => {
                    // In case of a frozen periodic cgroup, we may start the aperiodic process
                    self.run.unfreeze_aperiodic()?;
                    return Ok(());
                }
                // TODO Error Handling with HM
                PeriodicEvent::Call(e @ PartitionCall::Error(se)) => {
                    e.print_partition_log(self.base.name());
                    match self.base.part_hm().try_action(*se) {
                        Some(RecoveryAction::Module(ModuleRecoveryAction::Ignore)) => {}
                        Some(_) => {
                            return Err(TypedError::new(*se, anyhow!("Received Partition Error")))
                        }
                        None => {
                            return Err(TypedError::new(
                                SystemError::Panic,
                                anyhow!(
                                "Could not get recovery action for requested partition error: {se}"
                            ),
                            ))
                        }
                    };
                }
                PeriodicEvent::Call(c @ PartitionCall::Message(_)) => {
                    c.print_partition_log(self.base.name())
                }
                PeriodicEvent::Call(PartitionCall::Transition(mode)) => {
                    // Only exit run_periodic, if we changed our mode
                    if self.run.handle_transition(self.base, *mode)?.is_some() {
                        return Ok(());
                    }
                }
            }

            leftover = self.timeout.remaining_time();
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

    pub fn new(run: &Run) -> TypedResult<PeriodicPoller> {
        let events = run.periodic_events()?;

        let poll = Poller::new().typ(SystemError::Panic)?;
        poll.add(events.as_raw_fd(), Event::readable(Self::EVENTS_ID))
            .typ(SystemError::Panic)?;
        poll.add(
            run.receiver().as_raw_fd(),
            Event::readable(Self::RECEIVER_ID),
        )
        .typ(SystemError::Panic)?;

        Ok(PeriodicPoller { poll, events })
    }

    pub fn wait_timeout(&mut self, run: &mut Run, timeout: Duration) -> TypedResult<PeriodicEvent> {
        let start = Instant::now();

        if run.is_periodic_frozen()? {
            return Ok(PeriodicEvent::Frozen);
        }

        let mut leftover = timeout.saturating_sub(start.elapsed());
        while leftover > Duration::ZERO {
            let mut events = vec![];
            self.poll
                .wait(events.as_mut(), Some(leftover))
                .typ(SystemError::Panic)?;

            for e in events {
                match e.key {
                    // Got a Frozen event
                    Self::EVENTS_ID => {
                        // Re-sub the readable event
                        self.poll
                            .modify(self.events.as_raw_fd(), Event::readable(Self::EVENTS_ID))
                            .typ(SystemError::Panic)?;

                        // Then check if the cg is actually frozen
                        if run.is_periodic_frozen()? {
                            return Ok(PeriodicEvent::Frozen);
                        }
                    }
                    // got a call events
                    Self::RECEIVER_ID => {
                        // Re-sub the readable event
                        // This will result in the event instantly being ready again should we have
                        // something to read, but that is better than
                        // accidentally missing an event (at the expense of one extra loop per
                        // receive)
                        self.poll
                            .modify(
                                run.receiver().as_raw_fd(),
                                Event::readable(Self::RECEIVER_ID),
                            )
                            .typ(SystemError::Panic)?;

                        // Now receive anything
                        if let Some(call) = run.receiver().try_recv()? {
                            return Ok(PeriodicEvent::Call(call));
                        }
                    }
                    _ => {
                        return Err(anyhow!("Unexpected Event Received: {e:?}"))
                            .typ(SystemError::Panic)
                    }
                }
            }

            leftover = timeout.saturating_sub(start.elapsed());
        }

        Ok(PeriodicEvent::Timeout)
    }
}
