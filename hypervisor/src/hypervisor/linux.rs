use std::collections::HashMap;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

use a653rs::bindings::PartitionId;
use anyhow::anyhow;
use once_cell::sync::OnceCell;

use a653rs_linux_core::cgroup::CGroup;
use a653rs_linux_core::error::{ErrorLevel, LeveledResult, ResultExt, SystemError, TypedResultExt};
use a653rs_linux_core::file::TempFile;
use a653rs_linux_core::queuing::Queuing;
use a653rs_linux_core::sampling::Sampling;

use super::config::{Channel, Config};
use super::partition::Partition;
use super::scheduler::{Scheduler, Timeout};

pub static SYSTEM_START_TIME: OnceCell<TempFile<Instant>> = OnceCell::new();

//#[derive(Debug)]
pub struct Hypervisor {
    cg: CGroup,
    major_frame: Duration,
    scheduler: Scheduler,
    partitions: HashMap<PartitionId, Partition>,
    sampling_channel: HashMap<String, Sampling>,
    queuing_channel: HashMap<String, Queuing>,
    prev_cg: PathBuf,
    _config: Config,
    terminate_after: Option<Duration>,
    t0: Option<Instant>,
}

impl Hypervisor {
    pub fn new(config: Config, terminate_after: Option<Duration>) -> LeveledResult<Self> {
        // Init SystemTime
        SYSTEM_START_TIME
            .get_or_try_init(|| TempFile::create("system_time").lev(ErrorLevel::ModuleInit))?;

        let prev_cg = PathBuf::from(config.cgroup.parent().unwrap());

        let schedule = config.generate_schedule().lev(ErrorLevel::ModuleInit)?;
        let pid = std::process::id();
        let file_name = config.cgroup.file_name().unwrap().to_str().unwrap();
        let cg_name = format!("{file_name}-{pid}");
        let cg = CGroup::new_root(&prev_cg, cg_name.as_str())
            .typ(SystemError::CGroup)
            .lev(ErrorLevel::ModuleInit)?;

        let mut hv = Self {
            cg,
            scheduler: Scheduler::new(schedule),
            major_frame: config.major_frame,
            partitions: Default::default(),
            prev_cg,
            _config: config.clone(),
            sampling_channel: Default::default(),
            queuing_channel: Default::default(),
            terminate_after,
            t0: None,
        };

        for c in config.channel {
            hv.add_channel(c)?;
        }

        for p in config.partitions.iter() {
            if hv.partitions.contains_key(&p.id) {
                return Err(anyhow!("Partition \"{}\" already exists", p.name))
                    .lev_typ(SystemError::PartitionConfig, ErrorLevel::ModuleInit);
            }
            hv.partitions.insert(
                p.id,
                Partition::new(
                    hv.cg.get_path(),
                    p.clone(),
                    &hv.sampling_channel,
                    &hv.queuing_channel,
                )
                .lev(ErrorLevel::ModuleInit)?,
            );
        }

        Ok(hv)
    }

    fn add_channel(&mut self, channel: Channel) -> LeveledResult<()> {
        match channel {
            Channel::Queuing(q) => {
                if self.queuing_channel.contains_key(q.name()) {
                    return Err(anyhow!("Queuing Channel \"{}\" already exists", q.name()))
                        .lev_typ(SystemError::PartitionConfig, ErrorLevel::ModuleInit);
                }
                let queuing = Queuing::try_from(q).lev(ErrorLevel::ModuleInit)?;
                self.queuing_channel.insert(queuing.name(), queuing);
            }
            Channel::Sampling(s) => {
                if self.sampling_channel.contains_key(s.name()) {
                    return Err(anyhow!("Sampling Channel \"{}\" already exists", s.name()))
                        .lev_typ(SystemError::PartitionConfig, ErrorLevel::ModuleInit);
                }

                let sampling = Sampling::try_from(s).lev(ErrorLevel::ModuleInit)?;
                self.sampling_channel.insert(sampling.name(), sampling);
            }
        }

        Ok(())
    }

    // pub fn run(mut self) -> LeveledResult<()> {
    //     self.cg
    //         .mv_proc(nix::unistd::getpid())
    //         .typ(SystemError::CGroup)
    //         .lev(ErrorLevel::ModuleInit)?;

    //     //for p in self.partitions.values_mut() {
    //     //    let part = &self.config.partitions[p.id() - 1];
    //     //    //let args = PartitionStartArgs {
    //     //    //    condition: StartCondition::NormalStart,
    //     //    //    duration: part.duration,
    //     //    //    period: part.period,
    //     //    //    warm_start: false,
    //     //    //};
    //     //
    //     //    p.init().unwrap();
    //     //}

    //     let mut frame_start = Instant::now();

    //     // retain the first frame start as our sytems t0
    //     if self.t0.is_none() {
    //         self.t0 = Some(frame_start);
    //     }

    //     let sys_time = SYSTEM_START_TIME
    //         .get()
    //         .ok_or_else(|| anyhow!("SystemTime was not set"))
    //         .lev_typ(SystemError::Panic, ErrorLevel::ModuleInit)?;
    //     sys_time.write(&frame_start).lev(ErrorLevel::ModuleInit)?;
    //     sys_time.seal_read_only().lev(ErrorLevel::ModuleInit)?;
    //     loop {
    //         // if we are not ment to execute any longer, terminate here
    //         match self.terminate_after {
    //             Some(terminate_after) if frame_start - self.t0.unwrap() >
    // terminate_after => {                 info!(
    //                     "quitting, as a run-time of {} was reached",
    //                     humantime::Duration::from(terminate_after)
    //                 );
    //                 quit::with_code(0)
    //             }
    //             _ => {}
    //         }

    //         for (target_start, target_stop, partition_name) in &self.schedule {
    //             sleep(target_start.saturating_sub(frame_start.elapsed()));

    //             self.partitions.get_mut(partition_name).unwrap().run(
    //                 &mut self.sampling_channel,
    //                 &mut self.queuing_channel,
    //                 Timeout::new(frame_start, *target_stop),
    //             )?;
    //         }
    //         sleep(self.major_frame.saturating_sub(frame_start.elapsed()));

    //         frame_start += self.major_frame;
    //     }
    // }
    pub fn run(mut self) -> LeveledResult<()> {
        self.cg
            .mv_proc(nix::unistd::getpid())
            .typ(SystemError::CGroup)
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

        // retain the first frame start as our sytems t0
        let t0 = self.t0.unwrap_or(frame_start);

        let terminate_after_timeout = self
            .terminate_after
            .map(|duration| Timeout::new(t0, duration));

        let sys_time = SYSTEM_START_TIME
            .get()
            .ok_or_else(|| anyhow!("SystemTime was not set"))
            .lev_typ(SystemError::Panic, ErrorLevel::ModuleInit)?;
        sys_time.write(&frame_start).lev(ErrorLevel::ModuleInit)?;
        sys_time.seal_read_only().lev(ErrorLevel::ModuleInit)?;
        loop {
            // terminate hypervisor now if timeout is over
            if let Some(timeout) = &terminate_after_timeout {
                if !timeout.has_time_left() {
                    info!(
                        "quitting, as a run-time of {} was reached",
                        humantime::Duration::from(timeout.total_duration())
                    );
                    quit::with_code(0)
                }
            }

            self.scheduler.run_major_frame(
                frame_start,
                &mut self.partitions,
                &mut self.sampling_channel,
                &mut self.queuing_channel,
            )?;

            sleep(self.major_frame.saturating_sub(frame_start.elapsed()));

            frame_start += self.major_frame;
        }
    }
}

impl Drop for Hypervisor {
    fn drop(&mut self) {
        let now = Instant::now();
        for (p, m) in self.partitions.iter_mut() {
            trace!("freezing partition {p}");
            if let Err(e) = m.freeze() {
                error!("{e}")
            }
        }

        trace!(
            "moving own process to previous cgroup {:?}",
            self.prev_cg.as_path()
        );
        // Using unwrap in this context is probably safe, as a failure in import_root
        // requires that the cgroup must have been deleted externally
        if let Err(e) = CGroup::import_root(&self.prev_cg)
            .unwrap()
            .mv_proc(nix::unistd::getpid())
        {
            error!("{e}")
        }

        for (p, m) in self.partitions.drain() {
            trace!("deleting partition {p}");
            if let Err(e) = m.rm() {
                error!("{e}")
            }
        }

        trace!("deleting former own cgroup");
        if let Err(e) = self.cg.rm() {
            error!("{e}")
        }
        trace!("Hypervisor clean up took: {:?}", now.elapsed())
    }
}
