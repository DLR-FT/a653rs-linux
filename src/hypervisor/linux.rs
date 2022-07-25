use anyhow::anyhow;
use anyhow::Result;
use std::collections::HashMap;
use std::thread::sleep;
use std::time::Duration;
use std::time::Instant;

use nix::sys::signal::*;

//TODO add logging (trace, debug, error)

use super::config::Channel;
use super::config::Config;
use super::config::Partition as PartitionConfig;
use super::{cgroup::CGroup, partition::Partition};

#[derive(Debug)]
pub struct Hypervisor {
    cg: CGroup,
    major_frame: Duration,
    schedule: Vec<(Duration, String, bool)>,
    partitions: HashMap<String, Partition>,
}

impl Hypervisor {
    pub fn new(config: Config) -> Result<Self> {
        // TODO maybe dont panic for forcing unwind
        let sig_action = SigAction::new(SigHandler::Handler(handle_sigint),
            SaFlags::empty(),
            SigSet::empty());
        unsafe{ sigaction(SIGINT, &sig_action) }.unwrap();
        let schedule = config.generate_schedule();
        let cg = CGroup::new(config.cgroup_root, &config.cgroup_name)?;
        let mut hv = Self {
            cg,
            schedule,
            major_frame: config.major_frame,
            partitions: Default::default(),
        };

        for c in config.channel {
            hv.add_channel(c)?;
        }

        for p in config.partitions {
            hv.add_partition(&p.name, p.entry)?;
        }

        Ok(hv)
    }

    fn add_partition(&mut self, name: &str, entry: fn()) -> Result<()> {
        if self.partitions.contains_key(name) {
            return Err(anyhow!("Partition {name} already exists"));
        }
        self.partitions.insert(
            name.to_string(),
            Partition::from_cgroup(self.cg.path(), name, entry)?,
        );

        Ok(())
    }

    fn add_channel(&mut self, _channel: Channel) -> Result<()> {
        todo!()
    }

    pub fn run(mut self) -> !{
        self.cg.add_process(nix::unistd::getpid()).unwrap();

        for p in self.partitions.values_mut() {
            p.initialize()
        }

        let mut frame_start = Instant::now();
        loop {
            for (target_time, partition_name, start) in &self.schedule{
                sleep(target_time.saturating_sub(frame_start.elapsed()));
                let partition = self.partitions.get_mut(partition_name).unwrap();
                if *start{
                    partition.unfreeze().unwrap();
                } else {
                    partition.freeze().unwrap();
                }
            }
            sleep(self.major_frame.saturating_sub(frame_start.elapsed()));

            frame_start += self.major_frame;
        }
    }
}

impl Drop for Hypervisor {
    fn drop(&mut self) {
        for (_, m) in self.partitions.drain() {
            if let Err(e) = m.delete() {
                eprintln!("{e}")
            }
        }
    }
}

extern fn handle_sigint(_:i32) {
    panic!();
}