use std::collections::HashMap;
use std::fs;
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
use linux_apex_core::error::{ResultExt, SystemError, TypedResult};
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

// Information about the files that are to be mounted
#[derive(Debug)]
pub struct FileMounter {
    pub source: Option<PathBuf>,
    pub target: PathBuf,
    pub fstype: Option<String>,
    pub flags: MsFlags,
    pub data: Option<String>,
    // TODO: Find a way to get rid of this boolean
    pub is_dir: bool, // Use File::create or fs::create_dir_all
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

impl FileMounter {
    // Mount (and consume) a device
    pub fn mount(self, base_dir: &Path) -> anyhow::Result<()> {
        let target: &PathBuf = &base_dir.join(self.target);
        let fstype = self.fstype.map(|x| PathBuf::from(x));
        let data = self.data.map(|x| PathBuf::from(x));

        if self.is_dir {
            trace!("Creating directory {}", target.display());
            fs::create_dir_all(target)?;
        } else {
            // It is okay to use .unwrap() here.
            // It will only fail due to a developer mistake, not due to a user mistake.
            let parent = target.parent().unwrap();
            trace!("Creating directory {}", parent.display());
            fs::create_dir_all(parent)?;

            trace!("Creating file {}", target.display());
            fs::File::create(target)?;
        }

        mount::<PathBuf, PathBuf, PathBuf, PathBuf>(
            self.source.as_ref(),
            target,
            fstype.as_ref(),
            self.flags,
            data.as_ref(),
        )?;

        anyhow::Ok(())
    }
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

                // Mount the required mounts
                let mounts = [
                    // Mount working directory as tmpfs
                    FileMounter {
                        source: None,
                        target: "".into(),
                        fstype: Some("tmpfs".into()),
                        flags: MsFlags::empty(),
                        data: Some("size=500k".into()),
                        is_dir: true,
                    },
                    // Mount binary
                    FileMounter {
                        source: Some(base.bin.clone()),
                        target: "bin".into(),
                        fstype: None,
                        flags: MsFlags::MS_RDONLY | MsFlags::MS_BIND,
                        data: None,
                        is_dir: false,
                    },
                    // Mount /dev/null (for stdio::null)
                    FileMounter {
                        source: Some("/dev/null".into()),
                        target: "dev/null".into(),
                        fstype: None,
                        flags: MsFlags::MS_RDONLY | MsFlags::MS_BIND,
                        data: None,
                        is_dir: false,
                    },
                    // Mount proc
                    FileMounter {
                        source: Some("/proc".into()),
                        target: "proc".into(),
                        fstype: Some("proc".into()),
                        flags: MsFlags::empty(),
                        data: None,
                        is_dir: true,
                    },
                    // Mount CGroup v2
                    FileMounter {
                        source: None,
                        target: "sys/fs/cgroup".into(),
                        fstype: Some("cgroup2".into()),
                        flags: MsFlags::empty(),
                        data: None,
                        is_dir: true,
                    },
                ];

                for m in mounts {
                    debug!("mounting {:?}", &m);
                    m.mount(base.working_dir.path()).unwrap();
                }

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
    ) -> TypedResult<()> {
        PartitionTimeWindow::new(&self.base, &mut self.run, timeout).run()?;
        // TODO Error handling and freeze if err
        self.base.freeze()?;

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
