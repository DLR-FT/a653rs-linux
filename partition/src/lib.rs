#![allow(dead_code)]
#[macro_use]
extern crate log;

use std::os::unix::prelude::{FromRawFd, OwnedFd, RawFd};
use std::time::{Duration, Instant};

use apex_hal::prelude::{OperatingMode, PartitionId, StartCondition};
use linux_apex_core::file::{get_memfd, TempFile};
use linux_apex_core::health_event::PartitionEvent;
use linux_apex_core::ipc::IpcSender;
use linux_apex_core::partition::*;
use once_cell::sync::Lazy;
use process::Process;
use procfs::process::{FDTarget, Process as Proc};

pub mod apex;
pub mod partition;
mod scheduler;
// TODO un-pub process
pub mod process;

pub(crate) static SYSTEM_TIME: Lazy<Instant> = Lazy::new(|| {
    let fd = std::env::var(SYSTEM_TIME_FD_ENV)
        .unwrap()
        .parse::<RawFd>()
        .unwrap();
    TempFile::<Instant>::from_fd(fd).unwrap().read().unwrap()
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
    // TODO Get rid of get_memfd? Use env instead?
    if let Ok(fd) = get_memfd(APERIODIC_PROCESS_FILE) {
        TempFile::from_fd(fd).unwrap()
    } else {
        let file: TempFile<Option<Process>> = TempFile::new(APERIODIC_PROCESS_FILE).unwrap();
        file.write(&None).unwrap();
        file
    }
});

pub static PERIODIC_PROCESS: Lazy<TempFile<Option<Process>>> = Lazy::new(|| {
    if let Ok(fd) = get_memfd(PERIODIC_PROCESS_FILE) {
        TempFile::from_fd(fd).unwrap()
    } else {
        let file: TempFile<Option<Process>> = TempFile::new(PERIODIC_PROCESS_FILE).unwrap();
        file.write(&None).unwrap();
        file
    }
});

pub static HEALTH_EVENT_SENDER: Lazy<IpcSender<PartitionEvent>> = Lazy::new(|| unsafe {
    let fd = std::env::var(HEALTH_SENDER_FD_ENV)
        .unwrap()
        .parse::<RawFd>()
        .unwrap();
    IpcSender::from_raw_fd(fd)
});
