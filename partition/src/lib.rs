#![allow(dead_code)]
#[macro_use]
extern crate log;

use std::os::unix::prelude::AsRawFd;
use std::time::{Duration, Instant};

use apex_hal::prelude::{OperatingMode, PartitionId, StartCondition};
use linux_apex_core::fd::Fd;
use linux_apex_core::file::{get_memfd, TempFile};
use linux_apex_core::partition::*;
use nix::sys::eventfd::{eventfd, EfdFlags};
use once_cell::sync::Lazy;
use process::Process;
use procfs::process::{FDTarget, Process as Proc};

pub mod apex;
pub mod partition;
mod scheduler;
// TODO un-pub process
pub mod process;

pub(crate) static SYSTEM_TIME: Lazy<Instant> = Lazy::new(|| {
    TempFile::<Instant>::from_fd(get_memfd(SYSTEM_TIME_FILE).unwrap())
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

pub static APERIODIC_PROCESS: Lazy<TempFile<Option<Process>>> = Lazy::new(|| {
    if let Ok(fd) = get_memfd(APERIODIC_PROCESS_FILE) {
        TempFile::from_fd(fd).unwrap()
    } else {
        let file: TempFile<Option<Process>> = TempFile::new(APERIODIC_PROCESS_FILE).unwrap();
        file.write(&None).unwrap();
        file
    }
});

pub(crate) static PART_OPERATION_MODE: Lazy<TempFile<OperatingMode>> =
    Lazy::new(|| TempFile::from_fd(get_memfd(PARTITION_STATE_FILE).unwrap()).unwrap());

pub static PERIODIC_PROCESS: Lazy<TempFile<Option<Process>>> = Lazy::new(|| {
    if let Ok(fd) = get_memfd(PERIODIC_PROCESS_FILE) {
        TempFile::from_fd(fd).unwrap()
    } else {
        let file: TempFile<Option<Process>> = TempFile::new(PERIODIC_PROCESS_FILE).unwrap();
        file.write(&None).unwrap();
        file
    }
});

pub static EXTERNAL_HEALTH_EVENT_FILE: Lazy<Fd> = Lazy::new(|| {
    let internal = INTERNAL_HEALTH_EVENT_FILE.as_raw_fd();

    Proc::myself()
        .unwrap()
        .fd()
        .unwrap()
        .flatten()
        .filter(|f| f.fd != internal)
        .find_map(|f| {
            if let FDTarget::AnonInode(_) = &f.target {
                Some(f.fd)
            } else {
                None
            }
        })
        .unwrap()
        .try_into()
        .unwrap()
});

pub static INTERNAL_HEALTH_EVENT_FILE: Lazy<Fd> =
    Lazy::new(|| eventfd(0, EfdFlags::empty()).unwrap().try_into().unwrap());
