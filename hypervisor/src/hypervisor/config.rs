//! Configuration for a653rs-linux-hypervisor.
//!
//! A configuration contains information about the partition schedule.
//! Currently, only a single schedule is supported. Each schedule's major frame
//! (MAF) has a fixed duration and number of slots. Each slot is occupied by at
//! most one partition that executes during the slot. A slot has a fixed
//! duration and offset inside the MAF. A partition may occupy multiple slots
//! inside the schedule, in which case it may be repeated using the `period`
//! parameter. Also the MAF must be cleanly dividable by this period.
//!
//! The hypervisor runs the executable file specified by `image` for each
//! partition as a long-running process that is started and stopped according to
//! the partition schedule.
//!
//! Partitions can communicate using channels (Sampling and Queuing). The name
//! of the ports by which a partition can access a channel is the same for all
//! attached partitions.

//! ```rust
//! # use a653rs_linux_hypervisor::hypervisor::config::Config;
//! # let yaml = "
//! major_frame: 1s
//! partitions:
//!   - id: 0
//!     name: Foo
//!     duration: 10ms
//!     offset: 0ms
//!     period: 500ms
//!     image: target/x86_64-unknown-linux-musl/release/hello_part
//!   - id: 1
//!     name: Bar
//!     offset: 100ms
//!     duration: 10ms
//!     image: target/x86_64-unknown-linux-musl/release/hello_part
//!     period: 1s
//! channel:
//!   - !Sampling
//!     name: Hello
//!     msg_size: 10KB
//!     source: Foo
//!     destination:
//!       - Bar
//! # ";
//! # serde_yaml::from_str::<Config>(yaml).unwrap();
//! ```

use std::path::PathBuf;
use std::time::Duration;

use a653rs_linux_core::channel::{QueuingChannelConfig, SamplingChannelConfig};
use a653rs_linux_core::error::{ResultExt, SystemError, TypedResult};
use a653rs_linux_core::health::{ModuleInitHMTable, ModuleRunHMTable, PartitionHMTable};
use anyhow::anyhow;
use itertools::Itertools;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    #[serde(with = "humantime_serde")]
    pub major_frame: Duration,
    #[serde(default)]
    pub cgroup: PathBuf,
    pub partitions: Vec<Partition>,
    #[serde(default)]
    pub channel: Vec<Channel>,
    #[serde(default)]
    pub hm_init_table: ModuleInitHMTable,
    #[serde(default)]
    pub hm_run_table: ModuleRunHMTable,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Partition {
    pub id: i64,
    pub name: String,
    #[serde(with = "humantime_serde")]
    pub duration: Duration,
    #[serde(with = "humantime_serde")]
    pub offset: Duration,
    /// Repetition interval of the slice inside the MAF.
    #[serde(with = "humantime_serde")]
    pub period: Duration,
    pub image: PathBuf,
    #[serde(default)]
    pub devices: Vec<Device>,
    #[serde(default)]
    pub hm_table: PartitionHMTable,
    #[serde(default)]
    pub mounts: Vec<(PathBuf, PathBuf)>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Device {
    pub path: PathBuf,
    pub read_only: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum Channel {
    Queuing(QueuingChannelConfig),
    Sampling(SamplingChannelConfig),
}

impl Channel {
    pub fn queueing(&self) -> Option<QueuingChannelConfig> {
        if let Self::Queuing(q) = self {
            return Some(q.clone());
        }
        None
    }

    pub fn sampling(&self) -> Option<SamplingChannelConfig> {
        if let Self::Sampling(s) = self {
            return Some(s.clone());
        }
        None
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum ModuleStates {
    Init,
    Run,
}

impl Config {
    pub(crate) fn generate_schedule(&self) -> TypedResult<Vec<(Duration, Duration, String)>> {
        // Verify Periods and Major Frame
        let lcm_periods = self
            .partitions
            .iter()
            .map(|p| p.period.as_nanos())
            .reduce(num::integer::lcm);
        if let Some(lcm_periods) = lcm_periods {
            if self.major_frame.as_nanos() % lcm_periods != 0 {
                return Err(anyhow!("major frame is not a multiple of the least-common-multiple of all partition periods.\n\
                lcm: {:?}, major_frame: {:?}", Duration::from_nanos(lcm_periods as u64), self.major_frame))
                    .typ(SystemError::Config);
            }
        }

        // Generate Schedule
        let mut s = self
            .partitions
            .iter()
            .flat_map(|p| {
                let pimf = (self.major_frame.as_nanos() / p.period.as_nanos()) as u32;
                (0..pimf).map(|i| {
                    let start = p.offset + (p.period * i);
                    (start, start + p.duration, p.name.clone())
                })
            })
            .collect::<Vec<_>>();
        s.sort_by(|(d1, ..), (d2, ..)| d1.cmp(d2));

        // Verify no overlaps
        for ((pstart, pend, pname), (nstart, nend, nname)) in s.iter().tuple_windows() {
            if pend > nstart {
                return Err(anyhow!("Overlapping Partition Windows: {pname} (start: {pstart:?}, end: {pend:?}). {nname} (start: {nstart:?}, end: {nend:?})"))
                    .typ(SystemError::PartitionConfig);
            }
        }

        Ok(s)
    }
}
