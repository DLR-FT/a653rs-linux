use anyhow::{anyhow, Result};
use linux_apex_core::cgroup::{CGroup, DomainCGroup};
use nix::sys::signal::*;
use procfs::process::Process;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    thread::sleep,
    time::{Duration, Instant},
};

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
}

impl Hypervisor {
    pub fn new(config: Config) -> Result<Self> {
        let proc = Process::myself()?;
        let prev_cgroup = proc.cgroups()?.get(0).unwrap().pathname.clone();
        //TODO use mountinfo in proc for /sys/fs/cgroup path
        //      This could be put into the CGroup struct
        let prev_cg = PathBuf::from(format!("/sys/fs/cgroup{prev_cgroup}"));

        // TODO maybe dont panic for forcing unwind
        let sig_action = SigAction::new(
            SigHandler::Handler(handle_sigint),
            SaFlags::empty(),
            SigSet::empty(),
        );
        unsafe { sigaction(SIGINT, &sig_action) }.unwrap();
        let schedule = config.generate_schedule();
        let cg = DomainCGroup::new(
            config.cgroup.parent().unwrap(),
            config.cgroup.file_name().unwrap().to_str().unwrap(),
        )?;
        let mut hv = Self {
            cg,
            schedule,
            major_frame: config.major_frame,
            partitions: Default::default(),
            prev_cg,
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
                eprintln!("{e}")
            }
        }
        if let Err(e) = CGroup::add_process_to(&self.prev_cg, nix::unistd::getpid()) {
            eprintln!("{e}")
        }
        for (_, m) in self.partitions.drain() {
            if let Err(e) = m.delete() {
                eprintln!("{e}")
            }
        }
        if let Err(e) = self.cg.delete() {
            eprintln!("{e}")
        }
    }
}

extern "C" fn handle_sigint(_: i32) {
    panic!();
}
