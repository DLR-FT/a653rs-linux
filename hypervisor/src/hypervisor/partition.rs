use std::fs::File;
use std::os::unix::prelude::{AsRawFd, CommandExt, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Ok, Result, bail};
use apex_hal::prelude::{OperatingMode, StartCondition};
use clone3::Clone3;
use inotify::Inotify;
use linux_apex_core::cgroup::CGroup;
use linux_apex_core::error::{LeveledResult, ResultExt, SystemError, TypedResult};
use linux_apex_core::fd::PidFd;
use linux_apex_core::file::TempFile;
use linux_apex_core::health_event::PartitionCall;
use linux_apex_core::ipc::{channel_pair, IpcReceiver, IpcSender};
use linux_apex_core::partition::{
    APERIODIC_PROCESS_CGROUP, DURATION_ENV, IDENTIFIER_ENV, NAME_ENV, PARTITION_MODE_FD_ENV,
    PERIODIC_PROCESS_CGROUP, PERIOD_ENV, SENDER_FD_ENV, START_CONDITION_ENV, SYSTEM_TIME_FD_ENV,
};
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::unistd::{chdir, close, pivot_root, setgid, setuid, Gid, Pid, Uid};
use once_cell::unsync::OnceCell;
use polling::{Event, Poller};
use procfs::process::{Process, FDTarget};
use tempfile::{tempdir, TempDir};

use super::scheduler::{PartitionTimeWindow, Timeout};
use crate::hypervisor::config::Partition as PartitionConfig;
use crate::hypervisor::linux::SYSTEM_START_TIME;

#[derive(Debug, Clone, Copy)]
pub enum TransitionAction {
    Stop,
    Normal,
    Restart,
    Error,
}

// Struct for holding information of a partition which is not in Idle Mode
#[derive(Debug)]
pub(crate) struct Run {
    cgroup_main: CGroup,
    cgroup_aperiodic: CGroup,
    cgroup_periodic: CGroup,

    main: Pid,
    periodic: bool,
    aperiodic: bool,

    mode: OperatingMode,
    mode_file: TempFile<OperatingMode>,
    call_rx: IpcReceiver<PartitionCall>,
}

impl Run {
    pub fn new(base: &Base, condition: StartCondition, warm_start: bool) -> Result<Run> {
        let cgroup_main = CGroup::new(&base.cgroup.path(), "main")?;
        let cgroup_periodic = CGroup::new(&base.cgroup.path(), PERIODIC_PROCESS_CGROUP)?;
        let cgroup_aperiodic = CGroup::new(&base.cgroup.path(), APERIODIC_PROCESS_CGROUP)?;

        let real_uid = nix::unistd::getuid();
        let real_gid = nix::unistd::getgid();

        let (call_tx, call_rx) = channel_pair::<PartitionCall>()?;

        let mode = warm_start
            .then_some(OperatingMode::WarmStart)
            .unwrap_or(OperatingMode::ColdStart);
        let mode_file = TempFile::new("operation_mode")?;
        mode_file.write(&mode)?;

        let pid = match unsafe {
            Clone3::default()
                .flag_newcgroup()
                .flag_newuser()
                .flag_newpid()
                .flag_newns()
                .flag_newipc()
                .flag_newnet()
                .flag_into_cgroup(&base.cgroup.get_fd()?.as_raw_fd())
                .call()
        }? {
            0 => {
                // Map User and user group (required for tmpfs mounts)
                std::fs::write(
                    PathBuf::from("/proc/self").join("uid_map"),
                    format!("0 {} 1", real_uid.as_raw()),
                )
                .unwrap();
                std::fs::write(PathBuf::from("/proc/self").join("setgroups"), b"deny").unwrap();
                std::fs::write(
                    PathBuf::from("/proc/self").join("gid_map"),
                    format!("0 {} 1", real_gid.as_raw()).as_bytes(),
                )
                .unwrap();

                // Set uid and gid to the map user above (0)
                setuid(Uid::from_raw(0)).unwrap();
                setgid(Gid::from_raw(0)).unwrap();

                Partition::print_fds();
                // Release all unneeded fd's
                let system_start = SYSTEM_START_TIME.as_raw_fd();
                Partition::release_fds(&[
                    system_start,
                    //self.health_event.as_raw_fd()
                    call_tx.as_raw_fd(),
                    mode_file.as_raw_fd(),
                ])
                .unwrap();

                // Mount working directory as tmpfs (TODO with size?)
                mount::<str, _, _, str>(
                    None,
                    base.working_dir.path(),
                    Some("tmpfs"),
                    MsFlags::empty(),
                    Some("size=500k"),
                    //None,
                )
                .unwrap();

                // Mount binary
                let bin = base.working_dir.path().join("bin");
                File::create(&bin).unwrap();
                mount::<_, _, str, str>(
                    Some(&base.bin),
                    &bin,
                    None,
                    MsFlags::MS_RDONLY | MsFlags::MS_BIND,
                    None,
                )
                .unwrap();

                // TODO bind-mount requested devices

                // Mount proc
                let proc = base.working_dir.path().join("proc");
                std::fs::create_dir(proc.as_path()).unwrap();
                mount::<str, _, str, str>(
                    Some("/proc"),
                    proc.as_path(),
                    Some("proc"),
                    MsFlags::empty(),
                    None,
                )
                .unwrap();

                //// Mount CGroup V2
                let cgroup = base.working_dir.path().join("sys/fs/cgroup");
                std::fs::create_dir_all(&cgroup).unwrap();
                mount::<str, _, str, str>(
                    None,
                    cgroup.as_path(),
                    Some("cgroup2"),
                    MsFlags::empty(),
                    None,
                )
                .unwrap();

                // Change working directory and root (unmount old root)
                chdir(base.working_dir.path()).unwrap();
                pivot_root(".", ".").unwrap();
                umount2(".", MntFlags::MNT_DETACH).unwrap();
                chdir("/").unwrap();

                // Run binary
                // TODO detach stdio
                let err = Command::new("/bin")
                    //.stdout(Stdio::null())
                    //.stdin(Stdio::null())
                    //.stderr(Stdio::null())
                    // Set Partition Name Env
                    .env(NAME_ENV, &base.name)
                    .env(PERIOD_ENV, base.period.as_nanos().to_string())
                    .env(DURATION_ENV, base.duration.as_nanos().to_string())
                    .env(IDENTIFIER_ENV, base.id.to_string())
                    .env(START_CONDITION_ENV, (condition as u32).to_string())
                    .env(PARTITION_MODE_FD_ENV, mode_file.as_raw_fd().to_string())
                    .env(SYSTEM_TIME_FD_ENV, system_start.to_string())
                    .env(SENDER_FD_ENV, call_tx.as_raw_fd().to_string())
                    .exec();
                error!("{err:?}");

                unsafe { libc::_exit(0) };
            }
            child => child,
        };
        debug!("Child Pid: {pid}");

        let pid_fd = PidFd::try_from(Pid::from_raw(pid));
        let pid = Pid::from_raw(pid);

        Ok(Run {
            cgroup_main,
            cgroup_aperiodic,
            cgroup_periodic,
            main: pid,
            mode,
            mode_file,
            call_rx,
            periodic: false,
            aperiodic: false,
        })
    }

    pub fn mode(&self) -> OperatingMode {
        self.mode
    }

    pub fn receiver(&self) -> &IpcReceiver<PartitionCall> {
        &self.call_rx
    }

    pub fn unfreeze_aperiodic(&self) -> Result<bool> {
        if self.aperiodic {
            self.cgroup_aperiodic.unfreeze()?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn freeze_aperiodic(&self) -> Result<bool> {
        if self.aperiodic {
            self.cgroup_aperiodic.freeze()?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn unfreeze_periodic(&self) -> Result<bool> {
        if self.periodic {
            self.cgroup_periodic.unfreeze()?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn periodic_events(&self) -> TypedResult<OwnedFd> {
        self.cgroup_periodic.events_file()
    }

    pub fn is_periodic_frozen(&self) -> Result<bool> {
        self.cgroup_periodic.is_frozen()
    }

    pub fn freeze_periodic(&self) -> Result<bool> {
        if self.periodic {
            self.cgroup_periodic.freeze()?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Return error if invalid transition was requested
    /// Return Ok(None) if no action was taken
    pub fn handle_transition(
        &mut self,
        base: &Base,
        mode: OperatingMode,
    ) -> TypedResult<Option<OperatingMode>> {
        match (mode, self.mode) {
            // TODO this should be an error
            (_, OperatingMode::Idle) => panic!(),
            // TODO this should be an error
            (OperatingMode::WarmStart, OperatingMode::ColdStart) => panic!(),
            (OperatingMode::Normal, OperatingMode::Normal) => TypedResult::Ok(None),
            (OperatingMode::Idle, _) => {
                self.idle_transition(base).unwrap();
                TypedResult::Ok(Some(OperatingMode::Idle))
            }
            (OperatingMode::ColdStart, _) => {
                self.start_transition(base, false).unwrap();
                TypedResult::Ok(Some(OperatingMode::ColdStart))
            }
            (OperatingMode::WarmStart, _) => {
                self.start_transition(base, true).unwrap();
                TypedResult::Ok(Some(OperatingMode::WarmStart))
            }
            (OperatingMode::Normal, _) => {
                self.normal_transition(base).unwrap();
                    TypedResult::Ok(Some(OperatingMode::Normal))
            }
        }
    }

    
    fn normal_transition(&mut self, base: &Base) -> Result<()> {
        if base.is_frozen().unwrap() {
            bail!("May not transition while in a frozen state");
        }

        base.freeze().unwrap();

        if !self.cgroup_aperiodic.member()?.is_empty() {
            self.aperiodic = true;
        }

        if !self.cgroup_periodic.member()?.is_empty() {
            self.periodic = true;
        }

        // Move main process to own cgroup
        self.cgroup_main.freeze().unwrap();
        self.cgroup_main.add_process(self.main)?;

        self.freeze_aperiodic().unwrap();
        self.freeze_periodic().unwrap();

        self.mode = OperatingMode::Normal;
        self.mode_file.write(&self.mode).unwrap();

        self.cgroup_aperiodic.unfreeze().unwrap();
        base.unfreeze().unwrap();
        Ok(())
    }

    fn start_transition(&mut self, base: &Base, warm_start: bool) -> Result<()> {
        if base.is_frozen()? {
            return Err(anyhow!("May not transition while in a frozen state"));
        }

        base.freeze()?;
        base.kill_all_wait()?;

        *self = Run::new(base, StartCondition::PartitionRestart, warm_start)?;

        Ok(())
    }

    fn idle_transition(&mut self, base: &Base) -> Result<()> {
        if base.is_frozen()? {
            return Err(anyhow!("May not transition while in a frozen state"));
        }

        base.freeze()?;
        self.freeze_aperiodic()?;
        self.freeze_periodic()?;

        self.mode = OperatingMode::Idle;
        self.mode_file.write(&self.mode)?;

        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct Base {
    name: String,
    id: usize,
    bin: PathBuf,
    cgroup: CGroup,
    duration: Duration,
    period: Duration,
    working_dir: TempDir,
}

impl Base {
    pub fn cgroup(&self) -> &CGroup {
        &self.cgroup
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn unfreeze(&self) -> Result<()> {
        self.cgroup.unfreeze()
    }

    pub fn freeze(&self) -> Result<()> {
        self.cgroup.freeze()
    }

    pub fn is_frozen(&self) -> Result<bool> {
        self.cgroup.is_frozen()
    }

    pub fn kill_all_wait(&self) -> Result<()> {
        self.cgroup.kill_all_wait()
    }
}

//impl Base{
//    // TODO Declare cpus differently and add effect to this variable
//}

#[derive(Debug)]
pub(crate) struct Partition {
    base: Base,
    run: Option<Run>,
}

impl Partition {
    pub(crate) fn new<P: AsRef<Path>>(cgroup_root: P, config: PartitionConfig) -> Result<Self> {
        // Todo implement drop for cgroup? (in error case)
        let cgroup = CGroup::new(cgroup_root, &config.name)?;

        let working_dir = tempdir()?;
        trace!("CGroup Working directory: {:?}", working_dir.path());

        //let health_event = unsafe { OwnedFd::from_raw_fd(eventfd(0, EfdFlags::empty())?) };
        //let mode_file = TempFile::new("operation_mode")?;

        Ok(Self {
            base: Base {
                name: config.name,
                id: config.id,
                cgroup,
                bin: config.image,
                duration: config.duration,
                period: config.period,
                working_dir,
            },
            run: None,
        })
    }

    fn release_fds(keep: &[i32]) -> Result<()> {
        let proc = Process::myself()?;
        for fd in proc
            .fd()?
            .skip(3)
            .flatten()
            .filter(|fd| !keep.contains(&fd.fd))
            //TODO this fails in debug mode
        {
            trace!("Close FD: {}", fd.fd);
            close(fd.fd)?
        }

        Ok(())
    }

    #[allow(dead_code)]
    fn print_fds() {
        let fds = Process::myself().unwrap().fd().unwrap();
        for f in fds.flatten() {
            debug!("Open File Descriptor: {f:#?}")
        }
    }

    #[allow(dead_code)]
    fn print_mountinfo() {
        let mi = Process::myself().unwrap().mountinfo().unwrap();
        for i in mi {
            debug!("Existing MountInfo: {i:#?}")
        }
    }

    pub fn run(&mut self, timeout: Timeout) -> Result<()> {
        PartitionTimeWindow::new(&mut self.base, &mut self.run, timeout).run()?;
        // TODO Error handling and freeze if err
        self.base.freeze()
    }

    //fn idle_transition(mut self) -> Result<()> {
    //    self.cgroup.freeze();
    //    self.cgroup.kill_all_wait()?;
    //    if let Some(take)
    //    self.cgroup_main.delete().ok();
    //    self.cgroup_periodic.delete().ok();
    //    self.cgroup_aperiodic.delete().ok();
    //    Ok(())
    //}

    fn verify() -> Result<()> {
        todo!("Verify integrity of Partition")
    }

    fn wait_event(&self) {}

    //fn active_run(active: &mut Active, start: Instant, stop: Duration) -> Result<()>{

    //    active.unfreeze()?;
    //
    //    let mut leftover = stop.saturating_sub(start.elapsed());
    //    while leftover >= Duration::ZERO{
    //        if let Some(call) = a.wait_event_timeout(leftover).map_err(|e| (self, e.into()))? {
    //            match call {
    //                PartitionCall::Transition(mode) => {active.handle_transition(mode);},
    //                PartitionCall::Error(_) => {call.print_partition_log(&self.borrow_base().name); },
    //                PartitionCall::Message(_) => {call.print_partition_log(&self.borrow_base().name);},
    //            }
    //        }

    //        leftover = stop.saturating_sub(start.elapsed());
    //    }
    //
    //    active.freeze()?;
    //    todo!()
    //}

    fn stop(&mut self) -> Result<()> {
        todo!()
    }

    pub(crate) fn freeze(&self) -> Result<()> {
        self.base.cgroup.freeze()
    }
    pub(crate) fn unfreeze(&self) -> Result<()> {
        self.base.cgroup.unfreeze()
    }

    //fn start(&mut self, mode: OperatingMode) -> Result<()>{
    //    todo!()
    //}

    pub(crate) fn delete(self) -> Result<()> {
        self.base.cgroup.delete()
    }

    pub(crate) fn id(&self) -> usize {
        self.base.id
    }
}
