use std::cmp::Ordering;
use std::time::Duration;

use a653rs::bindings::PartitionId;
use anyhow::bail;
use itertools::Itertools;

/// The schedule for the execution of partitions in each major frame.
/// It consists of a [Vec] of timeframes sorted by their start time, which are
/// guaranteed to not overlap.
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

/// A timeframe inside of a major frame.
/// Both `start` and `end` are [Duration]s as they describe the time passed
/// since the major frame's start.
#[derive(Clone, Debug)]
pub(crate) struct ScheduledTimeframe {
    pub partition: PartitionId,
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
