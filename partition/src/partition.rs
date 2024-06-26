use std::cmp::min;

#[cfg(feature = "socket")]
use std::{
    fmt::Display,
    io,
    net::{TcpStream, UdpSocket},
};

use a653rs::prelude::{ApexErrorP4Ext, MAX_ERROR_MESSAGE_SIZE};
use a653rs_linux_core::error::SystemError;
use a653rs_linux_core::health_event::PartitionCall;
use log::{set_logger, set_max_level, LevelFilter, Record, SetLoggerError};

use crate::{CONSTANTS, SENDER};

#[cfg(feature = "socket")]
use crate::{TCP_SOCKETS, UDP_SOCKETS};

/// Static functions for within a partition
#[derive(Debug, Clone, Copy)]
pub struct ApexLinuxPartition;

impl ApexLinuxPartition {
    pub fn get_partition_name() -> String {
        CONSTANTS.name.clone()
    }

    #[cfg(feature = "socket")]
    pub fn get_udp_socket(sockaddr: &str) -> Result<Option<UdpSocket>, ApexLinuxError> {
        for stored in UDP_SOCKETS.iter() {
            if stored.local_addr()?.to_string() == sockaddr {
                let socket = stored.try_clone()?;
                return Ok(Some(socket));
            }
        }
        Ok(None)
    }

    #[cfg(feature = "socket")]
    pub fn get_tcp_stream(sockaddr: &str) -> Result<Option<TcpStream>, ApexLinuxError> {
        for stored in TCP_SOCKETS.iter() {
            if stored.peer_addr()?.to_string() == sockaddr {
                let socket = stored.try_clone()?;
                return Ok(Some(socket));
            }
        }
        Ok(None)
    }

    pub(crate) fn raise_system_error(error: SystemError) {
        if let Err(e) = SENDER.try_send(&PartitionCall::Error(error)) {
            panic!("Could not send SystemError event {error:?}. {e:?}")
        };
    }
}

#[cfg(feature = "socket")]
#[derive(Debug, Clone)]
pub enum ApexLinuxError {
    SocketError,
}

#[cfg(feature = "socket")]
impl Display for ApexLinuxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApexLinuxError::SocketError => f.write_str("Failed to get socket"),
        }
    }
}

#[cfg(feature = "socket")]
impl From<io::Error> for ApexLinuxError {
    fn from(_value: io::Error) -> Self {
        ApexLinuxError::SocketError
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
