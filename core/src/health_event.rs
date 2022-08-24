use apex_hal::prelude::OperatingMode;
use log::Level;
use serde::{Deserialize, Serialize};

use crate::error::SystemError;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum PartitionCall {
    Transition(OperatingMode),
    Error(SystemError),
    Message(String),
}

impl PartitionCall {
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
