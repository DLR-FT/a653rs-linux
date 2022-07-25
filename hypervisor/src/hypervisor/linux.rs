use std::collections::HashMap;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use linux_apex_core::cgroup::CGroup;
use linux_apex_core::error::{ErrorLevel, LeveledResult, ResultExt, SystemError, TypedResultExt};
use linux_apex_core::file::TempFile;
use linux_apex_core::sampling::Sampling;
use once_cell::sync::OnceCell;
use procfs::process::Process;

use super::config::{Channel, Config};
use super::partition::Partition;
use super::scheduler::Timeout;

pub static SYSTEM_START_TIME: OnceCell<TempFile<Instant>> = OnceCell::new();

//#[derive(Debug)]
pub struct Hypervisor {
    cg: CGroup,
    major_frame: Duration,
    schedule: Vec<(Duration, Duration, String)>,
    partitions: HashMap<String, Partition>,
    sampling_channel: HashMap<String, Sampling>,
    prev_cg: PathBuf,
    _config: Config,
}

impl Hypervisor {
    pub fn new(config: Config) -> LeveledResult<Self> {
        // Init SystemTime
        SYSTEM_START_TIME
            .get_or_try_init(|| TempFile::create("system_time").lev(ErrorLevel::ModuleInit))?;

        let proc = Process::myself().lev_typ(SystemError::Panic, ErrorLevel::ModuleInit)?;
        let prev_cgroup = proc
            .cgroups()
            .lev_typ(SystemError::Panic, ErrorLevel::ModuleInit)?
            .get(0)
            .unwrap()
            .pathname
            .clone();
        //TODO use mountinfo in proc for /sys/fs/cgroup path
        //      This could be put into the CGroup struct
        let prev_cg = PathBuf::from(format!("/sys/fs/cgroup{prev_cgroup}"));

        let schedule = config.generate_schedule().lev(ErrorLevel::ModuleInit)?;
        let cg = CGroup::new(
            config.cgroup.parent().unwrap(),
            config.cgroup.file_name().unwrap().to_str().unwrap(),
        )
        .lev(ErrorLevel::ModuleInit)?;

        let mut hv = Self {
            cg,
            schedule,
            major_frame: config.major_frame,
            partitions: Default::default(),
            prev_cg,
            _config: config.clone(),
            sampling_channel: Default::default(),
        };

        for c in config.channel {
            hv.add_channel(c)?;
        }

        for p in config.partitions.iter() {
            if hv.partitions.contains_key(&p.name) {
                return Err(anyhow!("Partition \"{}\" already exists", p.name))
                    .lev_typ(SystemError::PartitionConfig, ErrorLevel::ModuleInit);
            }
            hv.partitions.insert(
                p.name.clone(),
                Partition::new(hv.cg.path(), p.clone(), &hv.sampling_channel)
                    .lev(ErrorLevel::ModuleInit)?,
            );
        }

        Ok(hv)
    }

    fn add_channel(&mut self, channel: Channel) -> LeveledResult<()> {
        match channel {
            Channel::Queuing(_) => todo!(),
            Channel::Sampling(s) => {
                if self.sampling_channel.contains_key(&s.name) {
                    return Err(anyhow!("Sampling Channel \"{}\" already exists", s.name))
                        .lev_typ(SystemError::PartitionConfig, ErrorLevel::ModuleInit);
                }

                let sampling = Sampling::try_from(s).lev(ErrorLevel::ModuleInit)?;
                self.sampling_channel
                    .insert(sampling.name().to_string(), sampling);
            }
        }

        Ok(())
    }

    pub fn run(mut self) -> LeveledResult<()> {
        self.cg
            .add_process(nix::unistd::getpid())
            .lev(ErrorLevel::ModuleInit)?;

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

        let sys_time = SYSTEM_START_TIME
            .get()
            .ok_or_else(|| anyhow!("SystemTime was not set"))
            .lev_typ(SystemError::Panic, ErrorLevel::ModuleInit)?;
        sys_time.write(&frame_start).lev(ErrorLevel::ModuleInit)?;
        sys_time.seal_read_only().lev(ErrorLevel::ModuleInit)?;
        loop {
            for (target_start, target_stop, partition_name) in &self.schedule {
                sleep(target_start.saturating_sub(frame_start.elapsed()));

                self.partitions.get_mut(partition_name).unwrap().run(
                    &mut self.sampling_channel,
                    Timeout::new(frame_start, *target_stop),
                )?;
            }
            sleep(self.major_frame.saturating_sub(frame_start.elapsed()));

            frame_start += self.major_frame;
        }
    }
}

impl Drop for Hypervisor {
    fn drop(&mut self) {
        let now = Instant::now();
        for (_, m) in self.partitions.iter_mut() {
            if let Err(e) = m.freeze() {
                error!("{e}")
            }
        }
        if let Err(e) = CGroup::add_process_to(&self.prev_cg, nix::unistd::getpid()) {
            error!("{e}")
        }
        for (_, m) in self.partitions.drain() {
            if let Err(e) = m.delete(Duration::from_secs(2)) {
                error!("{e}")
            }
        }
        if let Err(e) = self.cg.delete(Duration::from_secs(2)) {
            error!("{e}")
        }
        trace!("Hypervisor clean up took: {:?}", now.elapsed())
    }
}
