use std::cmp::min;
use std::net::UdpSocket;

use apex_rs::prelude::{ApexErrorP4Ext, MAX_ERROR_MESSAGE_SIZE};
use linux_apex_core::error::SystemError;
use linux_apex_core::health_event::PartitionCall;
use log::{set_logger, set_max_level, LevelFilter, Record, SetLoggerError};

use crate::{CONSTANTS, IO_RX, SENDER};

/// Static functions for within a partition
#[derive(Debug, Clone, Copy)]
pub struct ApexLinuxPartition;

impl ApexLinuxPartition {
    pub fn get_partition_name() -> String {
        CONSTANTS.name.clone()
    }

    /// Receives UDP sockets from the hypervisor.
    /// Will panic if an error occurs while receiving the sockets.
    pub fn receive_udp_sockets() -> Vec<UdpSocket> {
        let mut sockets: Vec<UdpSocket> = Vec::default();
        loop {
            match IO_RX.try_receive() {
                Ok(sock) => {
                    if let Some(sock) = sock {
                        sockets.push(sock);
                    } else {
                        return sockets;
                    }
                }
                Err(e) => panic!("Could not receive UDP sockets from hypervisor: {e:?}"),
            }
        }
    }

    pub(crate) fn raise_system_error(error: SystemError) {
        if let Err(e) = SENDER.try_send(&PartitionCall::Error(error)) {
            panic!("Could not send SystemError event {error:?}. {e:?}")
        };
    }
}

static APEX_LOGGER: ApexLogger = ApexLogger();

#[derive(Debug, Clone, Copy)]
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
            ApexLinuxPartition::report_application_message(&msg.as_bytes()[0..max]).ok();
        }
    }

    fn flush(&self) {}
}
