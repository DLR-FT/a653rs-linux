use std::collections::HashMap;
use std::fs::File;
use std::os::unix::prelude::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::anyhow;
use apex_rs::bindings::PortDirection;
use apex_rs::prelude::{OperatingMode, StartCondition};
use clone3::Clone3;
use itertools::Itertools;
use linux_apex_core::cgroup::CGroup;
use linux_apex_core::error::{
    ErrorLevel, LeveledResult, ResultExt, SystemError, TypedResult, TypedResultExt,
};
use linux_apex_core::file::TempFile;
use linux_apex_core::health::PartitionHMTable;
use linux_apex_core::health_event::PartitionCall;
use linux_apex_core::ipc::{channel_pair, IpcReceiver};
use linux_apex_core::partition::{PartitionConstants, SamplingConstant};
use linux_apex_core::sampling::Sampling;
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::unistd::{chdir, close, pivot_root, setgid, setuid, Gid, Pid, Uid};
use procfs::process::Process;
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
    _mode_file_fd: OwnedFd,
    mode_file: TempFile<OperatingMode>,
    call_rx: IpcReceiver<PartitionCall>,
}

impl Run {
    pub fn new(base: &Base, condition: StartCondition, warm_start: bool) -> TypedResult<Run> {
        trace!("Create new \"Run\" for \"{}\" partition", base.name());
        let cgroup_main = base.cgroup.new("main").typ(SystemError::CGroup)?;
        let cgroup_periodic = base
            .cgroup
            .new(PartitionConstants::PERIODIC_PROCESS_CGROUP)
            .typ(SystemError::CGroup)?;
        let cgroup_aperiodic = base
            .cgroup
            .new(PartitionConstants::APERIODIC_PROCESS_CGROUP)
            .typ(SystemError::CGroup)?;

        let real_uid = nix::unistd::getuid();
        let real_gid = nix::unistd::getgid();

        let sys_time = SYSTEM_START_TIME
            .get()
            .ok_or_else(|| anyhow!("SystemTime was not set"))
            .typ(SystemError::Panic)?;

        let (call_tx, call_rx) = channel_pair::<PartitionCall>()?;

        let mode = warm_start
            .then_some(OperatingMode::WarmStart)
            .unwrap_or(OperatingMode::ColdStart);
        let mode_file = TempFile::create("operation_mode")?;
        let mode_file_fd = unsafe { OwnedFd::from_raw_fd(mode_file.as_raw_fd()) };
        mode_file.write(&mode)?;

        let pid = match unsafe {
            Clone3::default()
                .flag_newcgroup()
                .flag_newuser()
                .flag_newpid()
                .flag_newns()
                .flag_newipc()
                .flag_newnet()
                .flag_into_cgroup(
                    &std::fs::File::open(base.cgroup.get_path())
                        .typ(SystemError::CGroup)?
                        .as_raw_fd(),
                )
                .call()
        }
        .typ(SystemError::Panic)?
        {
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

                let mut keep = base.sampling_fds();
                keep.push(sys_time.as_raw_fd());
                keep.push(call_tx.as_raw_fd());
                keep.push(mode_file.as_raw_fd());

                Partition::release_fds(&keep).unwrap();

                // Mount working directory as tmpfs
                mount::<str, _, _, str>(
                    None,
                    base.working_dir.path(),
                    Some("tmpfs"),
                    MsFlags::empty(),
                    // TODO config size?
                    Some("size=500k"),
                    //None,
                )
                .unwrap();
                //mount::<_, _, str, str>(
                //    Some(base.working_dir.path()),
                //    base.working_dir.path(),
                //    None,
                //    MsFlags::MS_BIND,
                //    None,
                //).unwrap();

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

                let dev = base.working_dir.path().join("dev");
                std::fs::create_dir_all(&dev).unwrap();
                // TODO bind-mount requested devices

                // Mount /dev/null (for stdio::null)
                let null = dev.join("null");
                File::create(&null).unwrap();
                mount::<_, _, str, str>(
                    Some("/dev/null"),
                    &null,
                    None,
                    MsFlags::MS_RDONLY | MsFlags::MS_BIND,
                    None,
                )
                .unwrap();

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
                //umount("old").unwrap();
                chdir("/").unwrap();

                let constants: RawFd = PartitionConstants {
                    name: base.name.clone(),
                    identifier: base.id,
                    period: base.period,
                    duration: base.duration,
                    start_condition: condition,
                    sender_fd: call_tx.as_raw_fd(),
                    start_time_fd: sys_time.as_raw_fd(),
                    partition_mode_fd: mode_file.as_raw_fd(),
                    sampling: base
                        .sampling_channel
                        .clone()
                        .drain()
                        .map(|(_, s)| s)
                        .collect_vec(),
                }
                .try_into()
                .unwrap();

                // Run binary
                let mut handle = Command::new("/bin")
                    .stdout(Stdio::null())
                    .stdin(Stdio::null())
                    .stderr(Stdio::null())
                    // Set Partition Name Env
                    .env(
                        PartitionConstants::PARTITION_CONSTANTS_FD,
                        constants.to_string(),
                    )
                    .spawn()
                    .unwrap();
                handle.wait().unwrap();

                unsafe { libc::_exit(0) };
            }
            child => child,
        };
        debug!(
            "Successfully created Partition {}. Main Pid: {pid}",
            base.name()
        );

        //let pid_fd = PidFd::try_from(Pid::from_raw(pid));
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
            _mode_file_fd: mode_file_fd,
        })
    }

    pub fn mode(&self) -> OperatingMode {
        self.mode
    }

    pub fn receiver(&self) -> &IpcReceiver<PartitionCall> {
        &self.call_rx
    }

    pub fn unfreeze_aperiodic(&self) -> TypedResult<bool> {
        if self.aperiodic {
            self.cgroup_aperiodic.unfreeze().typ(SystemError::CGroup)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn freeze_aperiodic(&self) -> TypedResult<bool> {
        if self.aperiodic {
            self.cgroup_aperiodic.freeze().typ(SystemError::CGroup)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn unfreeze_periodic(&self) -> TypedResult<bool> {
        if self.periodic {
            self.cgroup_periodic.unfreeze().typ(SystemError::CGroup)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn periodic_events(&self) -> TypedResult<OwnedFd> {
        OwnedFd::try_from(
            std::fs::File::open(self.cgroup_periodic.get_events_path()).typ(SystemError::CGroup)?,
        )
        .typ(SystemError::CGroup)
    }

    pub fn is_periodic_frozen(&self) -> TypedResult<bool> {
        self.cgroup_periodic.frozen().typ(SystemError::CGroup)
    }

    pub fn freeze_periodic(&self) -> TypedResult<bool> {
        if self.periodic {
            self.cgroup_periodic.freeze().typ(SystemError::CGroup)?;
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
                self.idle_transition(base)?;
                TypedResult::Ok(Some(OperatingMode::Idle))
            }
            (OperatingMode::ColdStart, _) => {
                self.start_transition(base, false, StartCondition::PartitionRestart)?;
                TypedResult::Ok(Some(OperatingMode::ColdStart))
            }
            (OperatingMode::WarmStart, _) => {
                self.start_transition(base, true, StartCondition::PartitionRestart)?;
                TypedResult::Ok(Some(OperatingMode::WarmStart))
            }
            (OperatingMode::Normal, _) => {
                self.normal_transition(base)?;
                TypedResult::Ok(Some(OperatingMode::Normal))
            }
        }
    }

    fn normal_transition(&mut self, base: &Base) -> TypedResult<()> {
        if base.is_frozen()? {
            return Err(anyhow!("May not transition while in a frozen state"))
                .typ(SystemError::Panic);
        }

        base.freeze()?;

        if self.cgroup_aperiodic.populated().typ(SystemError::CGroup)? {
            self.aperiodic = true;
        }

        if self.cgroup_periodic.populated().typ(SystemError::CGroup)? {
            self.periodic = true;
        }

        // Move main process to own cgroup
        self.cgroup_main.freeze().typ(SystemError::CGroup)?;
        self.cgroup_main.mv(self.main).typ(SystemError::CGroup)?;

        self.freeze_aperiodic()?;
        self.freeze_periodic()?;

        self.mode = OperatingMode::Normal;
        self.mode_file.write(&self.mode)?;

        self.cgroup_aperiodic.unfreeze().typ(SystemError::CGroup)?;
        base.unfreeze()?;
        Ok(())
    }

    pub fn start_transition(
        &mut self,
        base: &Base,
        warm_start: bool,
        cond: StartCondition,
    ) -> TypedResult<()> {
        if base.is_frozen()? {
            return Err(anyhow!("May not transition while in a frozen state"))
                .typ(SystemError::Panic);
        }

        base.freeze()?;
        base.kill()?;

        *self = Run::new(base, cond, warm_start).typ(SystemError::PartitionInit)?;

        Ok(())
    }

    pub fn idle_transition(&mut self, base: &Base) -> TypedResult<()> {
        if base.is_frozen()? {
            return Err(anyhow!("May not transition while in a frozen state"))
                .typ(SystemError::Panic);
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
    hm: PartitionHMTable,
    id: i64,
    bin: PathBuf,
    cgroup: CGroup,
    sampling_channel: HashMap<String, SamplingConstant>,
    duration: Duration,
    period: Duration,
    working_dir: TempDir,
}

impl Base {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn unfreeze(&self) -> TypedResult<()> {
        self.cgroup.unfreeze().typ(SystemError::CGroup)
    }

    pub fn sampling_fds(&self) -> Vec<RawFd> {
        self.sampling_channel
            .iter()
            .map(|(_, s)| s.fd)
            .collect_vec()
    }

    pub fn freeze(&self) -> TypedResult<()> {
        self.cgroup.freeze().typ(SystemError::CGroup)
    }

    pub fn is_frozen(&self) -> TypedResult<bool> {
        self.cgroup.frozen().typ(SystemError::CGroup)
    }

    pub fn part_hm(&self) -> &PartitionHMTable {
        &self.hm
    }

    pub fn kill(&self) -> TypedResult<()> {
        self.cgroup.kill().typ(SystemError::CGroup)
    }
}

#[derive(Debug)]
pub(crate) struct Partition {
    base: Base,
    run: Run,
}

impl Partition {
    pub(crate) fn new<P: AsRef<Path>>(
        cgroup_root: P,
        config: PartitionConfig,
        sampling: &HashMap<String, Sampling>,
    ) -> TypedResult<Self> {
        // Todo implement drop for cgroup (in error case)
        let cgroup = CGroup::new_root(cgroup_root, &config.name).typ(SystemError::PartitionInit)?;

        let sampling_channel = sampling
            .iter()
            .filter_map(|(n, s)| s.constant(&config.name).map(|s| (n.clone(), s)))
            .collect();

        let working_dir = tempdir().typ(SystemError::PartitionInit)?;
        trace!("CGroup Working directory: {:?}", working_dir.path());

        let base = Base {
            name: config.name,
            id: config.id,
            cgroup,
            bin: config.image,
            duration: config.duration,
            period: config.period,
            working_dir,
            hm: config.hm_table,
            sampling_channel,
        };
        // TODO use StartCondition::HmModuleRestart in case of a ModuleRestart!!
        let run =
            Run::new(&base, StartCondition::NormalStart, false).typ(SystemError::PartitionInit)?;

        Ok(Self { base, run })
    }

    fn release_fds(keep: &[RawFd]) -> TypedResult<()> {
        let proc = Process::myself().typ(SystemError::Panic)?;
        for fd in proc
            .fd()
            .typ(SystemError::Panic)?
            .skip(3)
            .flatten()
            .filter(|fd| !keep.contains(&fd.fd))
        //TODO this fails in debug mode
        {
            trace!("Close FD: {}", fd.fd);
            close(fd.fd).typ(SystemError::Panic)?
        }

        Ok(())
    }

    #[allow(dead_code)]
    fn print_fds() {
        if let Ok(proc) = Process::myself() {
            if let Ok(fds) = proc.fd() {
                for fd in fds {
                    trace!("Open FD: {fd:?}")
                }
            }
        }
    }

    #[allow(dead_code)]
    fn print_mountinfo() {
        if let Ok(proc) = Process::myself() {
            if let Ok(mi) = proc.mountinfo() {
                for i in mi {
                    trace!("Existing MountInfo: {i:#?}")
                }
            }
        }
    }

    pub fn run(
        &mut self,
        sampling: &mut HashMap<String, Sampling>,
        timeout: Timeout,
    ) -> LeveledResult<()> {
        PartitionTimeWindow::new(&self.base, &mut self.run, timeout).run()?;
        // TODO Error handling and freeze if err
        self.base.freeze().lev(ErrorLevel::Partition)?;

        for (name, _) in self
            .base
            .sampling_channel
            .iter()
            .filter(|(_, s)| s.dir == PortDirection::Source)
        {
            sampling.get_mut(name).unwrap().swap();
        }

        Ok(())
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

    fn _verify() -> TypedResult<()> {
        todo!("Verify integrity of Partition")
    }

    pub(crate) fn freeze(&self) -> TypedResult<()> {
        self.base.cgroup.freeze().typ(SystemError::CGroup)
    }

    pub(crate) fn rm(self) -> TypedResult<()> {
        self.base.cgroup.rm().typ(SystemError::CGroup)
    }
}
