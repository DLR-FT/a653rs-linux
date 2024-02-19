use std::cmp::Ordering;
use std::collections::HashMap;
use std::thread::sleep;
use std::time::{Duration, Instant};

use a653rs::prelude::OperatingMode;
use anyhow::{anyhow, bail};
use itertools::Itertools;

use a653rs_linux_core::error::{ErrorLevel, LeveledError, LeveledResult, SystemError, TypedResult};
use a653rs_linux_core::sampling::Sampling;

use crate::hypervisor::partition::Partition;

/// A timeframe inside of a major frame.
/// Both `start` and `end` are [Duration]s as they describe the time passed since the major frame's start.
#[derive(Clone, Debug)]
pub(crate) struct ScheduledTimeframe {
    pub partition_name: String,
    pub start: Duration,
    pub end: Duration,
}

impl PartialEq for ScheduledTimeframe {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for ScheduledTimeframe {}

impl Ord for ScheduledTimeframe {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.start.cmp(&other.start) {
            Ordering::Equal => self.end.cmp(&other.end),
            other => other,
        }
    }
}

impl PartialOrd for ScheduledTimeframe {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The schedule for the execution of partitions in each major frame.
/// It consists of a [Vec] of timeframes sorted by their start time, which are guaranteed to not overlap.
pub(crate) struct PartitionSchedule {
    pub timeframes: Vec<ScheduledTimeframe>,
}

impl PartitionSchedule {
    /// Creates a new partition schedule from given timeframes.
    /// Returns `Err` if there are overlaps.
    pub fn from_timeframes(mut timeframes: Vec<ScheduledTimeframe>) -> anyhow::Result<Self> {
        timeframes.sort();

        // Verify no overlaps
        for (prev, next) in timeframes.iter().tuple_windows() {
            if prev.end > next.start {
                bail!("Overlapping partition timeframes: {prev:?}, {next:?})");
            }
        }

        Ok(Self { timeframes })
    }

    /// Returns an iterator through all timeframes sorted by start time
    pub fn iter(&self) -> impl Iterator<Item = &ScheduledTimeframe> {
        self.timeframes.iter()
    }
}

/// A scheduler that schedules the execution timeframes of partition according to a given [PartitionSchedule].
/// By calling [Scheduler::run_major_frame] a single major frame can be run.
pub(crate) struct Scheduler {
    schedule: PartitionSchedule,
}

impl Scheduler {
    pub fn new(schedule: PartitionSchedule) -> Self {
        Self { schedule }
    }
    /// Takes &mut self for now because P4 limits scheduling to a single core
    pub fn run_major_frame(
        &mut self,
        current_frame_start: Instant,
        partitions_by_name: &mut HashMap<String, Partition>,
        sampling_channels_by_name: &mut HashMap<String, Sampling>,
    ) -> LeveledResult<()> {
        for timeframe in self.schedule.iter() {
            sleep(
                timeframe
                    .start
                    .saturating_sub(current_frame_start.elapsed()),
            );

            let timeframe_timeout = Timeout::new(current_frame_start, timeframe.end);
            let partition = partitions_by_name
                .get_mut(&timeframe.partition_name)
                .expect("partition to exist because its name comes from `timeframe`");
            PartitionTimeframeScheduler::new(partition, timeframe_timeout).run()?;

            partition.run_post_timeframe(sampling_channels_by_name);
        }

        Ok(())
    }
}

/// A scheduler for a single partition timeframe
struct PartitionTimeframeScheduler<'a> {
    partition: &'a mut Partition,
    timeout: Timeout,
}

impl<'a> PartitionTimeframeScheduler<'a> {
    fn new(partition: &'a mut Partition, timeout: Timeout) -> Self {
        Self { partition, timeout }
    }

    fn run(&mut self) -> LeveledResult<()> {
        // Stop if the time is already over
        if !self.timeout.has_time_left() {
            return Ok(());
        }

        // If we are in the normal mode at the beginning of the time frame,
        // only then we may schedule the periodic process inside a partition
        if let OperatingMode::Normal = self.partition.get_base_run().1.mode() {
            let res = self.partition.run_periodic_process(self.timeout);
            if self.handle_partition_result(res)? == Some(false) {
                // Periodic process was not run -> run aperiodic process
                let res = self.partition.run_aperiodic_process(self.timeout);
                if self.handle_partition_result(res)? == Some(false) {
                    // Aperiodic process was also not run
                    return Err(LeveledError::new(
                        SystemError::Panic,
                        ErrorLevel::Partition,
                        anyhow!("At least one periodic or aperiodic process is expected to exist"),
                    ));
                }
            }
        }

        // Only continue if we have time left
        if self.timeout.has_time_left() {
            let res = self.run_post_periodic();
            self.handle_partition_result(res)?;
        }
        Ok(())
    }

    fn run_post_periodic(&mut self) -> TypedResult<()> {
        // if we are in the idle mode, just sleep until the end of the frame
        match self.partition.get_base_run().1.mode() {
            OperatingMode::Idle => {
                sleep(self.timeout.remaining_time());
                Ok(())
            }
            mode @ OperatingMode::ColdStart | mode @ OperatingMode::WarmStart => self
                .partition
                .run_start(self.timeout, mode == OperatingMode::WarmStart),
            OperatingMode::Normal => self
                .partition
                .run_aperiodic_process(self.timeout)
                .map(|_| ()),
        }
    }

    /// Takes in a [TypedResult] and makes the partition handle the `Err` variant.
    /// A successful handling of the error will then return `Ok(None)`.
    /// In case of `Ok(_)` the contained value is returned as `Ok(Some(_))`.
    fn handle_partition_result<T>(&mut self, res: TypedResult<T>) -> LeveledResult<Option<T>> {
        res.map(|t| Some(t))
            .or_else(|err| self.partition.handle_error(err).map(|_| None))
    }
}

/// A simple object for keeping track of a timeout that starts at some instant and has a fixed duration.
/// This object also exposes some basic functionality like querying the remaining time.
#[derive(Copy, Clone)]
pub(crate) struct Timeout {
    start: Instant,
    stop: Duration,
}

impl Timeout {
    pub fn new(start: Instant, stop: Duration) -> Self {
        Self { start, stop }
    }

    pub fn remaining_time(&self) -> Duration {
        self.stop.saturating_sub(self.start.elapsed())
    }

    pub fn has_time_left(&self) -> bool {
        self.remaining_time() > Duration::ZERO
    }

    pub fn total_duration(&self) -> Duration {
        self.stop
    }
}
