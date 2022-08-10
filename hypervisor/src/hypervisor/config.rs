use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    #[serde(with = "humantime_serde")]
    pub major_frame: Duration,
    #[serde(default)]
    pub cgroup: PathBuf,
    pub partitions: Vec<Partition>,
    #[serde(default)]
    pub channel: Vec<Channel>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Partition {
    pub name: String,
    #[serde(with = "humantime_serde")]
    pub duration: Duration,
    #[serde(with = "humantime_serde")]
    pub offset: Duration,
    pub image: PathBuf,
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize, Clone)]
pub enum Channel {
    //Queuing(QueuingChannel),
    //Sampling(SamplingChannel),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SamplingChannel {
    pub name: String,
    pub source: String,
    pub destination: HashSet<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QueuingChannel {
    pub name: String,
    pub source: String,
    pub destination: String,
}

impl Config {
    pub fn generate_schedule(&self) -> Vec<(Duration, String, bool)> {
        let mut s = self
            .partitions
            .iter()
            .flat_map(|p| {
                [
                    (p.offset, p.name.clone(), true),
                    (p.offset + p.duration, p.name.clone(), false),
                ]
            })
            .collect::<Vec<_>>();
        s.sort_by(|(d1, ..), (d2, ..)| d1.cmp(d2));
        s
    }
}
