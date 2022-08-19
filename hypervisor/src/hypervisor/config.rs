use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow};
use itertools::Itertools;
use linux_apex_core::error::{TypedResult, SystemError, ResultExt};
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
    pub id: usize,
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
pub struct PartitionHMTable {
    pub partition_init: PartitionErrorLevel,
    pub partition_main_panic: PartitionErrorLevel,
    pub hypervisor_panic: PartitionErrorLevel,
    pub segmentation: PartitionErrorLevel,
    pub time_duration_exceeded: PartitionErrorLevel,
    pub invalid_os_call: PartitionErrorLevel,
    pub application_error: PartitionErrorLevel,
    pub floating_point_error: PartitionErrorLevel,
    pub bad_partition_state: PartitionErrorLevel,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModuleInitHMTable {
    pub config: ModuleRecoveryAction,
    pub module_config: ModuleRecoveryAction,
    pub partition_config: ModuleRecoveryAction,
    pub partition_init: ModuleRecoveryAction,
    pub hypervisor_panic: ModuleRecoveryAction,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModuleRunHMTable {
    pub partition_init: ModuleRecoveryAction,
    pub hypervisor_panic: ModuleRecoveryAction,
}

impl Default for PartitionHMTable {
    fn default() -> Self {
        Self {
            partition_init: PartitionErrorLevel::Module(ModuleRecoveryAction::Ignore),
            segmentation: PartitionErrorLevel::Partition(PartitionRecoveryAction::WarmStart),
            time_duration_exceeded: PartitionErrorLevel::Module(ModuleRecoveryAction::Ignore),
            invalid_os_call: PartitionErrorLevel::Partition(PartitionRecoveryAction::WarmStart),
            floating_point_error: PartitionErrorLevel::Partition(
                PartitionRecoveryAction::WarmStart,
            ),
            partition_main_panic: PartitionErrorLevel::Partition(
                PartitionRecoveryAction::WarmStart,
            ),
            application_error: PartitionErrorLevel::Partition(PartitionRecoveryAction::WarmStart),
            hypervisor_panic: PartitionErrorLevel::Partition(PartitionRecoveryAction::WarmStart),
            bad_partition_state: PartitionErrorLevel::Partition(PartitionRecoveryAction::WarmStart),
        }
    }
}

impl Default for ModuleInitHMTable {
    fn default() -> Self {
        Self {
            config: ModuleRecoveryAction::Shutdown,
            module_config: ModuleRecoveryAction::Shutdown,
            partition_config: ModuleRecoveryAction::Shutdown,
            partition_init: ModuleRecoveryAction::Shutdown,
            hypervisor_panic: ModuleRecoveryAction::Shutdown,
        }
    }
}

impl Default for ModuleRunHMTable {
    fn default() -> Self {
        Self {
            partition_init: ModuleRecoveryAction::Shutdown,
            hypervisor_panic: ModuleRecoveryAction::Shutdown,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum PartitionErrorLevel {
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
    pub(crate) fn generate_schedule(&self) -> TypedResult<Vec<(Duration, Duration, String)>> {
        // Verify Periods and Major Frame
        if !self.partitions.is_empty() {
            let lcm_periods = self
                .partitions
                .iter()
                .map(|p| p.period.as_nanos())
                .reduce(num::integer::lcm)
                .unwrap();
            if self.major_frame.as_nanos() % lcm_periods != 0 {
                return anyhow!("major frame is not a multiple of the least-common-multiple of all partition periods.\n\
                lcm: {:?}, major_frame: {:?}", Duration::from_nanos(lcm_periods as u64), self.major_frame)
                    .typ_res(SystemError::Config);
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
                return anyhow!("Overlapping Partition Windows: {pname} (start: {pstart:?}, end: {pend:?}). {nname} (start: {nstart:?}, end: {nend:?})")
                    .typ_res(SystemError::PartitionConfig);
            }
        }

        Ok(s)
    }
}
