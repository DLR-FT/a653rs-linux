#![allow(dead_code)]
#[macro_use]
extern crate log;

use std::os::unix::prelude::{FromRawFd, RawFd};
use std::time::{Duration, Instant};

use apex_hal::prelude::{OperatingMode};
use linux_apex_core::file::{get_memfd, TempFile};
use linux_apex_core::health_event::PartitionCall;
use linux_apex_core::ipc::IpcSender;
use linux_apex_core::partition::*;
use memmap2::{MmapMut, MmapOptions};
use once_cell::sync::Lazy;
use process::Process;


pub(crate) mod apex;
pub mod partition;
//mod scheduler;
// TODO un-pub process
pub(crate) mod process;


pub const APERIODIC_PROCESS_FILE: &str = "aperiodic";
pub const PERIODIC_PROCESS_FILE: &str = "periodic";


pub(crate) static CONSTANTS: Lazy<PartitionConstants> = Lazy::new(|| {
    let fd = std::env::var(PARTITION_CONSTANTS_FD).unwrap().parse::<RawFd>()
    .unwrap();
    PartitionConstants::try_from(fd).unwrap()
});

pub(crate) static SYSTEM_TIME: Lazy<Instant> = Lazy::new(|| {
    TempFile::<Instant>::try_from(CONSTANTS.start_time_fd).unwrap().read().unwrap()
});

pub(crate) static PARTITION_MODE: Lazy<TempFile<OperatingMode>> = Lazy::new(|| {
    TempFile::<OperatingMode>::try_from(CONSTANTS.partition_mode_fd).unwrap()
});


pub(crate) static APERIODIC_PROCESS: Lazy<TempFile<Option<Process>>> = Lazy::new(|| {
    // TODO Get rid of get_memfd? Use env instead?
    if let Ok(fd) = get_memfd(APERIODIC_PROCESS_FILE) {
        TempFile::try_from(fd).unwrap()
    } else {
        let file: TempFile<Option<Process>> = TempFile::create(APERIODIC_PROCESS_FILE).unwrap();
        file.write(&None).unwrap();
        file
    }
});

// TODO generate in hypervisor
pub(crate) static PERIODIC_PROCESS: Lazy<TempFile<Option<Process>>> = Lazy::new(|| {
    if let Ok(fd) = get_memfd(PERIODIC_PROCESS_FILE) {
        TempFile::try_from(fd).unwrap()
    } else {
        let file: TempFile<Option<Process>> = TempFile::create(PERIODIC_PROCESS_FILE).unwrap();
        file.write(&None).unwrap();
        file
    }
});

pub(crate) static SENDER: Lazy<IpcSender<PartitionCall>> = Lazy::new(|| unsafe {
    IpcSender::from_raw_fd(CONSTANTS.sender_fd)
});

pub(crate) static SIGNAL_STACK: Lazy<MmapMut> = Lazy::new(|| {
    MmapOptions::new()
        .stack()
        .len(nix::libc::SIGSTKSZ)
        .map_anon()
        .unwrap()
});
