//! Fetch information from a partition
use a653rs::prelude::OperatingMode;
use log::Level;
use serde::{Deserialize, Serialize};

use crate::error::SystemError;

#[derive(Debug, Clone, Deserialize, Serialize)]
/// The core unit for communication in that module
pub enum PartitionCall {
    /// The status of the partition
    Transition(OperatingMode),
    /// Potential errors
    Error(SystemError),
    /// Potential messages
    Message(String),
}

impl PartitionCall {
    /// Prints debugs, warnings, traces and errors to their accompanying streams
    // TODO: Somehow comment what is going on inside this beast
    pub fn print_partition_log(&self, name: &str) {
        let name = &format!("Partition: {name}");
        match self {
            PartitionCall::Error(e) => error!(target: name, "{e:?}"),
            PartitionCall::Message(msg) => {
                let mut msg_chars = msg.chars();
                if let Some(level) = msg_chars.next() {
                    let msg = msg_chars.collect::<String>();
                    if let Ok(level) = level.to_string().parse::<usize>() {
                        return match level {
                            l if l == Level::Debug as usize => debug!(target: name, "{msg}"),
                            l if l == Level::Warn as usize => warn!(target: name, "{msg}"),
                            l if l == Level::Trace as usize => trace!(target: name, "{msg}"),
                            l if l == Level::Error as usize => error!(target: name, "{msg}"),
                            _ => info!(target: name, "{msg}"),
                        };
                    }
                }
                info!(target: name, "{msg}")
            }
            PartitionCall::Transition(mode) => {
                debug!(target: name, "Received Transition Request: {mode:?}")
            }
        }
    }
}
