#[macro_use]
extern crate log;
#[macro_use]
extern crate lazy_static;

use std::sync::atomic::AtomicPtr;
use std::time::Instant;

use apex_hal::prelude::OperatingMode;
use linux_apex_core::file::{get_fd, TempFile};
use linux_apex_core::partition::{
    HEALTH_STATE_FILE, NAME_ENV, PARTITION_STATE_FILE, SYSTEM_TIME_FILE,
};

use crate::process::Process;

pub mod apex;
pub mod partition;
mod scheduler;
// TODO un-pub process
pub mod process;

lazy_static! {
    pub(crate) static ref SYSTEM_TIME: Instant =
        TempFile::from_fd(get_fd(SYSTEM_TIME_FILE).unwrap())
            .unwrap()
            .read()
            .unwrap();
    pub(crate) static ref PART_NAME: String = std::env::var(NAME_ENV).unwrap();
    pub(crate) static ref HEALTH_MONITOR_STATE: TempFile<u8> =
        TempFile::from_fd(get_fd(HEALTH_STATE_FILE).unwrap()).unwrap();
    pub(crate) static ref PARTITION_STATE: TempFile<OperatingMode> =
        TempFile::from_fd(get_fd(PARTITION_STATE_FILE).unwrap()).unwrap();
    pub static ref PROCESSES: TempFile<ProcessesType> = TempFile::new("processes").unwrap();
}

pub const MAX_PROCESSES: usize = 255;
//pub type ProcessesType = heapless::Vec<Process, MAX_PROCESSES>;
pub type ProcessesType = heapless::Vec<usize, MAX_PROCESSES>;
//pub(crate) static PROCESSES: Vec<usize> = Vec::new();
//pub static PROCESSES_PTR: AtomicPtr<Vec<usize>> = AtomicPtr::new(PROCESSES.ptr);
