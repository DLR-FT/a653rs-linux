use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug)]
pub struct Config {
    pub major_frame: Duration,
    pub cgroup_root: PathBuf,
    pub cgroup_name: String,
    pub partitions: HashSet<Partition>,
    pub channel: HashSet<Channel>,
}

#[derive(Debug, Eq)]
pub struct Partition {
    pub name: String,
    pub duration: Duration,
    pub offset: Duration,
    pub entry: fn(),
}

#[derive(Debug, Hash, Eq, PartialEq)]
pub enum Channel {
    Queuing(QueuingChannel),
    Sampling(SamplingChannel),
}

#[derive(Debug, Eq)]
pub struct SamplingChannel {
    pub name: String,
    pub source: String,
    pub destination: HashSet<String>,
}

#[derive(Debug, Eq)]
pub struct QueuingChannel {
    pub name: String,
    pub source: String,
    pub destination: String,
}

impl PartialEq for Partition {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Hash for Partition {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}

impl PartialEq for QueuingChannel {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Hash for QueuingChannel {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}

impl PartialEq for SamplingChannel {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Hash for SamplingChannel {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}

impl Config{
    pub fn generate_schedule(&self) -> Vec<(Duration, String, bool)>{
        let mut s = self.partitions.iter().flat_map(|p| {
            [(p.offset, p.name.clone(), true), 
            (p.offset + p.duration, p.name.clone(), false)]
        }).collect::<Vec<_>>();
        s.sort_by(|(d1, ..), (d2, ..)| d1.cmp(d2));
        s
    }
}