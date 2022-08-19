use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Error, bail};
use apex_hal::prelude::{OperatingMode, StartCondition};
use linux_apex_core::cgroup::CGroup;
use linux_apex_core::error::{ResultExt, ErrorLevel, SystemError};
use linux_apex_core::file::TempFile;
use once_cell::sync::Lazy;
use procfs::process::Process;

use super::scheduler::{PartitionTimeWindow, Timeout};
//TODO add better errors than anyhow?
use super::{
    config::{Channel, Config},
    partition::Partition,
};

pub static SYSTEM_START_TIME: Lazy<TempFile<Instant>> =
    Lazy::new(|| TempFile::new("system_time").unwrap());

//#[derive(Debug)]
pub struct Hypervisor {
    cg: CGroup,
    major_frame: Duration,
    schedule: Vec<(Duration, Duration, String)>,
    partitions: HashMap<String, Partition>,
    prev_cg: PathBuf,
    config: Config,
}

impl Hypervisor {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let proc = Process::myself().lev_res(SystemError::Panic, ErrorLevel::ModuleInit)?;
        let prev_cgroup = proc.cgroups().lev_res(SystemError::Panic, ErrorLevel::ModuleInit)?.get(0).unwrap().pathname.clone();
        //TODO use mountinfo in proc for /sys/fs/cgroup path
        //      This could be put into the CGroup struct
        let prev_cg = PathBuf::from(format!("/sys/fs/cgroup{prev_cgroup}"));

        let schedule = config.generate_schedule().unwrap();
        let cg = CGroup::new(
            config.cgroup.parent().unwrap(),
            config.cgroup.file_name().unwrap().to_str().unwrap(),
        ).lev_res(SystemError::Panic, ErrorLevel::ModuleInit)?;

        let mut hv = Self {
            cg,
            schedule,
            major_frame: config.major_frame,
            partitions: Default::default(),
            prev_cg,
            config: config.clone(),
        };

        for c in config.channel {
            hv.add_channel(c)?;
        }

        for p in config.partitions.iter() {
            if hv.partitions.contains_key(&p.name) {
                bail!("Partition {} already exists", p.name);
            }
            hv.partitions
                .insert(p.name.clone(), Partition::new(hv.cg.path(), p.clone())?);
        }

        Ok(hv)
    }

    fn add_channel(&mut self, _channel: Channel) -> anyhow::Result<()> {
        // TODO Implement Channels first, then implement this
        Ok(())
    }

    pub fn run(mut self) -> ! {
        self.cg.add_process(nix::unistd::getpid()).unwrap();

        //for p in self.partitions.values_mut() {
        //    let part = &self.config.partitions[p.id() - 1];
        //    //let args = PartitionStartArgs {
        //    //    condition: StartCondition::NormalStart,
        //    //    duration: part.duration,
        //    //    period: part.period,
        //    //    warm_start: false,
        //    //};
        //
        //    p.init().unwrap();
        //}

        let mut frame_start = Instant::now();

        SYSTEM_START_TIME.write(&frame_start).unwrap();
        SYSTEM_START_TIME.seal_read_only().unwrap();
        loop {
            for (target_start, target_stop, partition_name) in &self.schedule {
                sleep(target_start.saturating_sub(frame_start.elapsed()));

                self.partitions
                    .get_mut(partition_name)
                    .unwrap()
                    .run(Timeout::new(frame_start, *target_stop))
                    .unwrap();

                //sleep(target_stop.saturating_sub(frame_start.elapsed()));
                //let mut leftover = target_stop.saturating_sub(frame_start.elapsed());
                //while leftover > Duration::ZERO {
                //    let res = partition.wait_event_timeout(leftover);

                //    // TODO What to do with errors?
                //    if let Ok(Some(event)) = res {
                //        match event{
                //            linux_apex_core::health_event::PartitionCall::Transition(mode) => {
                //                match partition.get_transition_action(mode){
                //                    super::partition::TransitionAction::Stop => todo!(),
                //                    super::partition::TransitionAction::Normal => todo!(),
                //                    super::partition::TransitionAction::Restart => todo!(),
                //                    super::partition::TransitionAction::Error => todo!(),
                //                }
                //            },
                //            _ => event.print_partition_log(partition_name)
                //        }
                //    }

                //    leftover = target_stop.saturating_sub(frame_start.elapsed());
                //}
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
