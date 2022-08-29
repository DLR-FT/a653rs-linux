//TODO remove this
#![allow(dead_code)]

use std::cmp::min;

use apex_hal::prelude::{report_message, MAX_ERROR_MESSAGE_SIZE};
use linux_apex_core::error::SystemError;
use linux_apex_core::health_event::PartitionCall;
use log::{set_logger, set_max_level, LevelFilter, Record, SetLoggerError};

use crate::{CONSTANTS, SENDER};

/// Static functions for within a partition
pub struct ApexLinuxPartition;

impl ApexLinuxPartition {
    pub fn get_partition_name() -> String {
        CONSTANTS.name.clone()
    }

    pub(crate) fn raise_system_error(error: SystemError) {
        if let Err(e) = SENDER.try_send(&PartitionCall::Error(error)) {
            panic!("Could not send SystemError event {error:?}. {e:?}")
        };
    }
}

static APEX_LOGGER: ApexLogger = ApexLogger();

pub struct ApexLogger();

impl ApexLogger {
    pub fn install_logger(level: LevelFilter) -> Result<(), SetLoggerError> {
        set_logger(&APEX_LOGGER).map(|()| set_max_level(level))
    }

    pub fn install_panic_hook() {
        std::panic::set_hook(Box::new(|panic_info| error!("{panic_info:#?}")));
    }
}

impl log::Log for ApexLogger {
    fn enabled(&self, _meta: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        let level = record.level() as usize;
        for line in record.args().to_string().lines() {
            let msg = if line.len() < MAX_ERROR_MESSAGE_SIZE {
                format!("{level}{line}")
            } else {
                format!("{level}{}..", &line[..(MAX_ERROR_MESSAGE_SIZE - 3)])
            };
            let max = min(MAX_ERROR_MESSAGE_SIZE, msg.len());
            report_message::<ApexLinuxPartition>(&msg.as_bytes()[0..max]).ok();
        }
    }

    fn flush(&self) {}
}
