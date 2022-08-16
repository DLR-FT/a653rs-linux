use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, Result};
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
    pub hm_table: ModuleHMTable,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Partition {
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
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Device {
    pub path: PathBuf,
    pub read_only: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum Channel {
    Queuing(QueuingChannel),
    Sampling(SamplingChannel),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SamplingChannel {
    pub name: String,
    pub source: String,
    pub destination: HashSet<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QueuingChannel {
    pub name: String,
    pub source: String,
    pub destination: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum ModuleStates {
    Init,
    Run,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum SystemErrors {
    Config,
    ModuleConfig,
    PartitionConfig,
    PartitionInit,
    Segmentation,
    TimeDurationExceeded,
    InvalidOsCall,
    DivideByZero,
    FloatingPointError,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PartitionHMTable {
    pub partition_init: ErrorLevel,
    pub segmentation: ErrorLevel,
    pub time_duration_exceeded: ErrorLevel,
    pub invalid_os_call: ErrorLevel,
    pub divide_by_zero: ErrorLevel,
    pub floating_point_error: ErrorLevel,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModuleHMTable {
    pub config: ModuleRecoveryAction,
    pub module_config: ModuleRecoveryAction,
    pub partition_config: ModuleRecoveryAction,
    pub partition_init: ModuleRecoveryAction,
    pub segmentation: ModuleRecoveryAction,
    pub time_duration_exceeded: ModuleRecoveryAction,
    pub invalid_os_call: ModuleRecoveryAction,
    pub divide_by_zero: ModuleRecoveryAction,
    pub floating_point_error: ModuleRecoveryAction,
}

impl Default for PartitionHMTable {
    fn default() -> Self {
        Self {
            partition_init: ErrorLevel::Module(ModuleRecoveryAction::Shutdown),
            segmentation: ErrorLevel::Partition(PartitionRecoveryAction::WarmStart),
            time_duration_exceeded: ErrorLevel::Module(ModuleRecoveryAction::Ignore),
            invalid_os_call: ErrorLevel::Partition(PartitionRecoveryAction::WarmStart),
            divide_by_zero: ErrorLevel::Partition(PartitionRecoveryAction::WarmStart),
            floating_point_error: ErrorLevel::Partition(PartitionRecoveryAction::WarmStart),
        }
    }
}

impl Default for ModuleHMTable {
    fn default() -> Self {
        Self {
            config: ModuleRecoveryAction::Shutdown,
            module_config: ModuleRecoveryAction::Shutdown,
            partition_config: ModuleRecoveryAction::Shutdown,
            partition_init: ModuleRecoveryAction::Shutdown,
            segmentation: ModuleRecoveryAction::Shutdown,
            time_duration_exceeded: ModuleRecoveryAction::Shutdown,
            invalid_os_call: ModuleRecoveryAction::Shutdown,
            divide_by_zero: ModuleRecoveryAction::Shutdown,
            floating_point_error: ModuleRecoveryAction::Shutdown,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum ErrorLevel {
    Module(ModuleRecoveryAction),
    Partition(PartitionRecoveryAction),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum ModuleRecoveryAction {
    Ignore,
    Shutdown,
    Reset,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum PartitionRecoveryAction {
    Idle,
    ColdStart,
    WarmStart,
}

impl Config {
    pub fn generate_schedule(&self) -> Result<Vec<(Duration, Duration, String)>> {
        // Verify Periods and Major Frame
        if !self.partitions.is_empty() {
            let lcm_periods = self
                .partitions
                .iter()
                .map(|p| p.period.as_nanos())
                .reduce(num::integer::lcm)
                .unwrap();
            if self.major_frame.as_nanos() % lcm_periods != 0 {
                return Err(anyhow!("major frame is not a multiple of the least-common-multiple of all partition periods.\n\
                    lcm: {:?}, major_frame: {:?}", Duration::from_nanos(lcm_periods as u64), self.major_frame));
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
                return Err(anyhow!("Overlapping Partition Windows: {pname} (start: {pstart:?}, end: {pend:?}). {nname} (start: {nstart:?}, end: {nend:?})"));
            }
        }

        Ok(s)
    }
}
