use std::path::PathBuf;
use std::time::Duration;

use anyhow::anyhow;
use itertools::Itertools;
use linux_apex_core::channel::{QueuingChannelConfig, SamplingChannelConfig};
use linux_apex_core::error::{ResultExt, SystemError, TypedResult};
use linux_apex_core::health::{ModuleInitHMTable, ModuleRunHMTable, PartitionHMTable};
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
