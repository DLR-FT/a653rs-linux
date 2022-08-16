use apex_hal::prelude::ErrorCode;
use log::Level;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub enum SystemError {
    Config,
    ModuleConfig,
    PartitionConfig,
    PartitionInit,
    Segmentation,
    TimeDurationExceeded,
    InvalidOsCall,
    DivideByZero,
    FloatingPointError,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum PartitionEvent {
    Error(SystemError),
    Message(String),
}

impl PartitionEvent {
    pub fn print_partition_log(&self, name: &str) {
        let name = &format!("Partition: {name}");
        match self {
            PartitionEvent::Error(e) => warn!(target: name, "{e:?}"),
            PartitionEvent::Message(msg) => {
                let mut msg_chars = msg.chars();
                if let Some(level) = msg_chars.next() {
                    let msg = msg_chars.collect::<String>();
                    if let Ok(level) = level.to_string().parse::<usize>() {
                        return match level {
                            l if l == Level::Debug as usize => debug!(target: name, "{msg}"),
                            l if l == Level::Warn as usize => warn!(target: name, "{msg}"),
                            l if l == Level::Trace as usize => trace!(target: name, "{msg}"),
                            _ => info!(target: name, "{msg}"),
                        };
                    }
                }
                info!(target: name, "{msg}")
            }
        }
    }
}
