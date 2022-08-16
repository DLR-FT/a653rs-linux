//TODO remove this
#![allow(dead_code)]

use std::cmp::{max, min};

use apex_hal::prelude::{ApexError, ErrorHandler, Partition, MAX_ERROR_MESSAGE_SIZE};
use linux_apex_core::health_event::{PartitionEvent, SystemError};
use log::{set_logger, set_max_level, LevelFilter, Record, SetLoggerError};

use crate::{HEALTH_EVENT_SENDER, PART_NAME};

/// Static functions for within a partition
pub struct ApexLinuxPartition;

impl ApexLinuxPartition {
    pub fn get_partition_name() -> String {
        PART_NAME.clone()
    }

    pub(crate) fn raise_system_error(error: SystemError) {
        if let Err(e) = HEALTH_EVENT_SENDER.try_send(&PartitionEvent::Error(error)) {
            panic!("Could not send SystemError event {error:?}. {e:?}")
        };
    }
}

static APEX_LOGGER: ApexLogger = ApexLogger();

pub struct ApexLogger();

impl ApexLogger {
    pub fn install(level: LevelFilter) -> Result<(), SetLoggerError> {
        set_logger(&APEX_LOGGER).map(|()| set_max_level(level))
    }
}

impl log::Log for ApexLogger {
    fn enabled(&self, meta: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        let msg = format!("{}{}", record.level() as usize, record.args());
        //println!("{msg}")
        let max = min(MAX_ERROR_MESSAGE_SIZE, msg.len());
        ErrorHandler::<ApexLinuxPartition>::report_message(&msg.as_bytes()[0..max]).ok();
    }

    fn flush(&self) {}
}
