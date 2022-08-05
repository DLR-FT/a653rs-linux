use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use linux_apex_core::cgroup::{CGroup, DomainCGroup};
use linux_apex_core::file::TempFile;
use linux_apex_core::partition::SYSTEM_TIME_FILE;
use procfs::process::Process;

//TODO add better errors than anyhow?
use super::{
    config::{Channel, Config},
    partition::Partition,
};

//#[derive(Debug)]
pub struct Hypervisor {
    cg: DomainCGroup,
    major_frame: Duration,
    schedule: Vec<(Duration, String, bool)>,
    partitions: HashMap<String, Partition>,
    prev_cg: PathBuf,
    start_time: TempFile<Instant>,
}

impl Hypervisor {
    pub fn new(config: Config) -> Result<Self> {
        let proc = Process::myself()?;
        let prev_cgroup = proc.cgroups()?.get(0).unwrap().pathname.clone();
        //TODO use mountinfo in proc for /sys/fs/cgroup path
        //      This could be put into the CGroup struct
        let prev_cg = PathBuf::from(format!("/sys/fs/cgroup{prev_cgroup}"));

        let schedule = config.generate_schedule();
        let cg = DomainCGroup::new(
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
        };

        for c in config.channel {
            hv.add_channel(c)?;
        }

        for p in config.partitions {
            hv.add_partition(&p.name, p.image)?;
        }

        Ok(hv)
    }

    fn add_partition<P: AsRef<Path>>(&mut self, name: &str, bin: P) -> Result<()> {
        if self.partitions.contains_key(name) {
            return Err(anyhow!("Partition {name} already exists"));
        }
        self.partitions.insert(
            name.to_string(),
            Partition::from_cgroup(self.cg.path(), name, bin)?,
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
            p.initialize()
        }

        let mut frame_start = Instant::now();
        self.start_time.write(frame_start).unwrap();
        self.start_time.lock_all().unwrap();
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
