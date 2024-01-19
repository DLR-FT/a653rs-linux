use std::collections::HashMap;
use std::thread::sleep;
use std::time::Instant;

use a653rs::bindings::PartitionId;
use a653rs::prelude::OperatingMode;

use a653rs_linux_core::error::{LeveledResult, TypedResult};
use a653rs_linux_core::queuing::Queuing;
use a653rs_linux_core::sampling::Sampling;
pub(crate) use schedule::{PartitionSchedule, ScheduledTimeframe};
pub(crate) use timeout::Timeout;

use crate::hypervisor::partition::Partition;

mod schedule;
mod timeout;

/// A scheduler that schedules the execution timeframes of partition according
/// to a given [PartitionSchedule]. By calling [Scheduler::run_major_frame] a
/// single major frame can be run.
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
        partitions: &mut HashMap<PartitionId, Partition>,
        sampling_channels_by_name: &mut HashMap<String, Sampling>,
        queuing_channels_by_name: &mut HashMap<String, Queuing>,
    ) -> LeveledResult<()> {
        for timeframe in self.schedule.iter() {
            sleep(
                timeframe
                    .start
                    .saturating_sub(current_frame_start.elapsed()),
            );

            let timeframe_timeout = Timeout::new(current_frame_start, timeframe.end);
            let partition = partitions
                .get_mut(&timeframe.partition)
                .expect("partition to exist because its name comes from `timeframe`");
            PartitionTimeframeScheduler::new(partition, timeframe_timeout).run()?;

            partition.run_post_timeframe(sampling_channels_by_name, queuing_channels_by_name);
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
                    let part_name = self.partition.name();
                    warn!("partition {part_name}: no process is scheduled")
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

    /// Takes in a [TypedResult] and makes the partition handle the `Err`
    /// variant. A successful handling of the error will then return
    /// `Ok(None)`. In case of `Ok(_)` the contained value is returned as
    /// `Ok(Some(_))`.
    fn handle_partition_result<T>(&mut self, res: TypedResult<T>) -> LeveledResult<Option<T>> {
        res.map(|t| Some(t))
            .or_else(|err| self.partition.handle_error(err).map(|_| None))
    }
}
