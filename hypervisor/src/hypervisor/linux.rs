use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use apex_hal::prelude::StartCondition;
use linux_apex_core::cgroup::CGroup;
use linux_apex_core::file::TempFile;
use linux_apex_core::partition::SYSTEM_TIME_FILE;
use procfs::process::Process;

use super::partition::PartitionStartArgs;
//TODO add better errors than anyhow?
use super::{
    config::{Channel, Config},
    partition::Partition,
};

//#[derive(Debug)]
pub struct Hypervisor {
    cg: CGroup,
    major_frame: Duration,
    schedule: Vec<(Duration, String, bool)>,
    partitions: HashMap<String, Partition>,
    prev_cg: PathBuf,
    start_time: TempFile<Instant>,
    config: Config,
}

impl Hypervisor {
    pub fn new(config: Config) -> Result<Self> {
        let proc = Process::myself()?;
        let prev_cgroup = proc.cgroups()?.get(0).unwrap().pathname.clone();
        //TODO use mountinfo in proc for /sys/fs/cgroup path
        //      This could be put into the CGroup struct
        let prev_cg = PathBuf::from(format!("/sys/fs/cgroup{prev_cgroup}"));

        let schedule = config.generate_schedule();
        let cg = CGroup::new(
            config.cgroup.parent().unwrap(),
            config.cgroup.file_name().unwrap().to_str().unwrap(),
        )?;

        let start_time = TempFile::new(SYSTEM_TIME_FILE)?;

        let mut hv = Self {
            cg,
            schedule,
            major_frame: config.major_frame,
            partitions: Default::default(),
            prev_cg,
            start_time,
            config: config.clone(),
        };

        for c in config.channel {
            hv.add_channel(c)?;
        }

        for (i, p) in config.partitions.iter().enumerate() {
            hv.add_partition(&p.name, i + 1, p.image.clone())?;
        }

        Ok(hv)
    }

    fn add_partition<P: AsRef<Path>>(&mut self, name: &str, id: usize, bin: P) -> Result<()> {
        if self.partitions.contains_key(name) {
            return Err(anyhow!("Partition {name} already exists"));
        }
        self.partitions.insert(
            name.to_string(),
            Partition::from_cgroup(self.cg.path(), name, id, bin)?,
        );

        Ok(())
    }

    fn add_channel(&mut self, _channel: Channel) -> Result<()> {
        // TODO Implement Channels first, then implement this
        Ok(())
    }

    pub fn run(mut self) -> ! {
        self.cg.add_process(nix::unistd::getpid()).unwrap();

        for p in self.partitions.values_mut() {
            let part = &self.config.partitions[p.id() - 1];
            let args = PartitionStartArgs {
                condition: StartCondition::NormalStart,
                duration: part.duration,
                // use period aswell
                period: Duration::default(),
            };

            p.restart(args)
        }

        let mut frame_start = Instant::now();

        self.start_time.write(&frame_start).unwrap();
        self.start_time.seal_read_only().unwrap();
        loop {
            for (target_time, partition_name, start) in &self.schedule {
                sleep(target_time.saturating_sub(frame_start.elapsed()));
                let partition = self.partitions.get_mut(partition_name).unwrap();
                if *start {
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
        for (_, m) in self.partitions.iter_mut() {
            if let Err(e) = m.freeze() {
                error!("{e}")
            }
        }
        if let Err(e) = CGroup::add_process_to(&self.prev_cg, nix::unistd::getpid()) {
            error!("{e}")
        }
        for (_, m) in self.partitions.drain() {
            if let Err(e) = m.delete() {
                error!("{e}")
            }
        }
        if let Err(e) = self.cg.delete() {
            error!("{e}")
        }
    }
}
