#![allow(dead_code)]
#[macro_use]
extern crate log;

use std::time::{Duration, Instant};

use apex_hal::bindings::ApexSystemTime;
use apex_hal::prelude::{NumCores, OperatingMode, PartitionId, StartCondition, SystemTime};
use linux_apex_core::file::{get_fd, TempFile};
use linux_apex_core::partition::*;
use once_cell::sync::{Lazy, OnceCell};
use process::Process;

pub mod apex;
pub mod partition;
mod scheduler;
// TODO un-pub process
pub mod process;

pub(crate) static SYSTEM_TIME: Lazy<Instant> = Lazy::new(|| {
    TempFile::<Instant>::from_fd(get_fd(SYSTEM_TIME_FILE).unwrap())
        .unwrap()
        .read()
        .unwrap()
});

pub(crate) static PART_NAME: Lazy<String> = Lazy::new(|| std::env::var(NAME_ENV).unwrap());

pub(crate) static PART_PERIOD: Lazy<Duration> =
    Lazy::new(|| Duration::from_nanos(std::env::var(PERIOD_ENV).unwrap().parse::<u64>().unwrap()));

pub(crate) static PART_DURATION: Lazy<Duration> = Lazy::new(|| {
    Duration::from_nanos(std::env::var(DURATION_ENV).unwrap().parse::<u64>().unwrap())
});

pub(crate) static PART_IDENTIFIER: Lazy<PartitionId> =
    Lazy::new(|| std::env::var(IDENTIFIER_ENV).unwrap().parse().unwrap());

pub(crate) static PART_START_CONDITION: Lazy<StartCondition> = Lazy::new(|| {
    std::env::var(START_CONDITION_ENV)
        .unwrap()
        .parse::<u32>()
        .unwrap()
        .try_into()
        .unwrap()
});

pub(crate) static HEALTH_MONITOR_STATE: Lazy<TempFile<u8>> =
    Lazy::new(|| TempFile::from_fd(get_fd(HEALTH_STATE_FILE).unwrap()).unwrap());

pub(crate) static PART_OPERATION_MODE: Lazy<TempFile<OperatingMode>> =
    Lazy::new(|| TempFile::from_fd(get_fd(PARTITION_STATE_FILE).unwrap()).unwrap());

pub static PERIODIC_PROCESS: Lazy<TempFile<Option<Process>>> = Lazy::new(|| {
    if let Ok(fd) = get_fd(PERIODIC_PROCESS_FILE) {
        TempFile::from_fd(fd).unwrap()
    } else {
        TempFile::new(PERIODIC_PROCESS_FILE).unwrap()
    }
});

pub static APERIODIC_PROCESS: Lazy<TempFile<Option<Process>>> = Lazy::new(|| {
    if let Ok(fd) = get_fd(APERIODIC_PROCESS_FILE) {
        TempFile::from_fd(fd).unwrap()
    } else {
        let file: TempFile<Option<Process>> = TempFile::new(APERIODIC_PROCESS_FILE).unwrap();
        file.write(&None).unwrap();
        file
    }
});
