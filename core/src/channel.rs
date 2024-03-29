//! The channels used in combination with the related *sampling* module
// TODO: Consider merging this module with sampling, as having a module only
// providing structs might be weird.
use std::collections::HashSet;

use bytesize::ByteSize;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SamplingChannelConfig {
    #[serde(deserialize_with = "de_size_str")]
    pub msg_size: ByteSize,
    pub source: PortConfig,
    pub destination: HashSet<PortConfig>,
}

impl SamplingChannelConfig {
    pub fn name(&self) -> &str {
        &self.source.port
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QueuingChannelConfig {
    #[serde(deserialize_with = "de_size_str")]
    pub msg_size: ByteSize,
    pub msg_num: usize,
    pub source: PortConfig,
    pub destination: PortConfig,
}

impl QueuingChannelConfig {
    pub fn name(&self) -> &str {
        &self.source.port
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Hash, PartialEq, Eq)]
pub struct PortConfig {
    pub partition: String,
    pub port: String,
}

impl PortConfig {
    pub fn name(&self) -> String {
        format!("{}:{}", self.partition, self.port)
    }
}

fn de_size_str<'de, D>(de: D) -> Result<ByteSize, D::Error>
where
    D: Deserializer<'de>,
{
    String::deserialize(de)?
        .parse::<ByteSize>()
        .map_err(serde::de::Error::custom)
}
