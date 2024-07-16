use std::collections::HashMap;
use std::net::{TcpStream, UdpSocket};
use std::os::unix::prelude::{AsRawFd, FromRawFd, OwnedFd, PermissionsExt, RawFd};
use std::os::unix::process::CommandExt;
use std::path::{self, Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use super::config::PosixSocket;
use super::scheduler::Timeout;
use crate::hypervisor::config::Partition as PartitionConfig;
use crate::hypervisor::SYSTEM_START_TIME;
use crate::problem;

use a653rs::bindings::{PartitionId, PortDirection};
use a653rs::prelude::{OperatingMode, StartCondition};
use a653rs_linux_core::cgroup::{self, CGroup};
use a653rs_linux_core::error::{
    ErrorLevel, LeveledResult, ResultExt, SystemError, TypedError, TypedResult, TypedResultExt,
};
use a653rs_linux_core::file::TempFile;
use a653rs_linux_core::health::{ModuleRecoveryAction, PartitionHMTable, RecoveryAction};
use a653rs_linux_core::health_event::PartitionCall;
use a653rs_linux_core::ipc::{bind_receiver, io_pair, IoReceiver, IoSender, IpcReceiver};
use a653rs_linux_core::partition::{PartitionConstants, QueuingConstant, SamplingConstant};
use a653rs_linux_core::queuing::Queuing;
use a653rs_linux_core::sampling::Sampling;
use anyhow::{anyhow, Context};
use bytesize::ByteSize;
use clone3::Clone3;
use itertools::Itertools;
pub use mounting::FileMounter;
use nix::mount::{umount2, MntFlags};
use nix::unistd::{chdir, close, getpid, pivot_root, setgid, setuid, Gid, Pid, Uid};
use polling::{Event, Events, Poller};
use procfs::process::Process;
use tempfile::{tempdir, TempDir};

mod mounting;

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
    _cgroup_main: CGroup,
    cgroup_aperiodic: CGroup,
    cgroup_periodic: CGroup,

    _main: Pid,
    periodic: bool,
    aperiodic: bool,

    mode: OperatingMode,
    _mode_file_fd: OwnedFd,
    mode_file: TempFile<OperatingMode>,
    call_rx: IpcReceiver<PartitionCall>,
    // We need to keep the struct for the sender's side, so
    // the sockets currently in transmission are not closed
    // before the partition has received them.
    _io_udp_tx: IoSender<UdpSocket>,
    _io_tcp_tx: IoSender<TcpStream>,
}

impl Run {
    pub fn new(base: &Base, condition: StartCondition, warm_start: bool) -> TypedResult<Run> {
        trace!("Create new \"Run\" for \"{}\" partition", base.name());
        let cgroup_processes = base
            .cgroup
            .new(PartitionConstants::PROCESSES_CGROUP)
            .typ(SystemError::CGroup)?;
        let cgroup_main = cgroup_processes
            .new_threaded(PartitionConstants::MAIN_PROCESS_CGROUP)
            .typ(SystemError::CGroup)?;
        let cgroup_periodic = cgroup_processes
            .new_threaded(PartitionConstants::PERIODIC_PROCESS_CGROUP)
            .typ(SystemError::CGroup)?;
        let cgroup_aperiodic = cgroup_processes
            .new_threaded(PartitionConstants::APERIODIC_PROCESS_CGROUP)
            .typ(SystemError::CGroup)?;

        let real_uid = nix::unistd::getuid();
        let real_gid = nix::unistd::getgid();

        let sys_time = SYSTEM_START_TIME
            .get()
            .ok_or_else(|| anyhow!("SystemTime was not set"))
            .typ(SystemError::Panic)?;

        let ipc_path = base
            .working_dir
            .path()
            .join(PartitionConstants::IPC_SENDER.trim_start_matches('/'));
        std::fs::create_dir_all(ipc_path.parent().unwrap()).typ(SystemError::Panic)?;
        let call_rx = bind_receiver::<PartitionCall>(&ipc_path)?;

        // TODO add a `::new(warm_start: bool)->Self` function to `OperatingMode`, use
        // it here
        let mode = if warm_start {
            OperatingMode::WarmStart
        } else {
            OperatingMode::ColdStart
        };
        let mode_file = TempFile::create("operation_mode")?;
        let mode_file_fd = unsafe { OwnedFd::from_raw_fd(mode_file.as_raw_fd()) };
        mode_file.write(&mode)?;

        let IoTxRx {
            udp_io_tx,
            udp_io_rx,
            tcp_io_tx,
            tcp_io_rx,
        } = send_sockets(base)?;

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
                keep.extend_from_slice(&base.queuing_fds());
                keep.push(sys_time.as_raw_fd());
                keep.push(mode_file.as_raw_fd());
                keep.push(udp_io_rx.as_raw_fd());
                keep.push(tcp_io_rx.as_raw_fd());

                Partition::release_fds(&keep).unwrap();

                let ipc_path_inner: PathBuf = PartitionConstants::IPC_SENDER[1..].into();

                // Mount the required mounts
                let mut mounts = vec![
                    // Mount working directory as tmpfs
                    FileMounter::tmpfs("", ByteSize::kb(500)),
                    // Mount binary
                    FileMounter::bind_ro(&base.bin, "/bin").unwrap(),
                    // Mount /dev/null (for stdio::null)
                    FileMounter::bind_ro("/dev/null", "/dev/null").unwrap(),
                    // Mount proc
                    FileMounter::proc(),
                    // Mount CGroup v2
                    FileMounter::cgroup(),
                    // IPC Socket for Syscalls
                    FileMounter::bind_rw(ipc_path, ipc_path_inner).unwrap(),
                ];

                for (source, target) in base.mounts.iter().cloned() {
                    // make target path relative because they will later be appended to the
                    // partition's base directory by the `FileMounter`
                    let relative_target = target
                        .strip_prefix("/")
                        .context("target paths for mounting must be absolute")
                        .typ(SystemError::Panic)?;

                    let file_mounter = FileMounter::bind_rw(source, relative_target)
                        .context("failed to initialize file mounter")
                        .typ(SystemError::Panic)?;
                    mounts.push(file_mounter);
                }

                // TODO: Check for duplicate mounts

                let tmpfs_path = base.working_dir.path().join("tmpfs");
                for m in mounts {
                    debug!("mounting {:?}", &m);
                    m.mount(&tmpfs_path)
                        .context("failed to mount")
                        .typ(SystemError::Panic)?;
                }

                // Change working directory and root (unmount old root)
                chdir(&tmpfs_path).unwrap();
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
                    start_time_fd: sys_time.as_raw_fd(),
                    partition_mode_fd: mode_file.as_raw_fd(),
                    udp_io_fd: udp_io_rx.as_raw_fd(),
                    tcp_io_fd: tcp_io_rx.as_raw_fd(),
                    sampling: base
                        .sampling_channel
                        .clone()
                        .drain()
                        .map(|(_, s)| s)
                        .collect_vec(),
                    queuing: base
                        .queuing_channel
                        .clone()
                        .drain()
                        .map(|(_, q)| q)
                        .collect_vec(),
                }
                .try_into()
                .unwrap();

                // Run binary
                let mut command = Command::new("/bin");
                let mut command = command
                    .stdout(Stdio::null())
                    .stdin(Stdio::null())
                    .stderr(Stdio::null())
                    // Set Partition Name Env
                    .env(
                        PartitionConstants::PARTITION_CONSTANTS_FD,
                        constants.to_string(),
                    );
                unsafe {
                    let path = cgroup::mount_point().typ(SystemError::CGroup)?;
                    let path = path
                        .join(PartitionConstants::PROCESSES_CGROUP)
                        .join(PartitionConstants::MAIN_PROCESS_CGROUP);
                    let cgroup_main = CGroup::import_root(path).typ(SystemError::CGroup).unwrap();

                    command = command.pre_exec(move || {
                        cgroup_main
                            .mv_proc(getpid())
                            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
                    });
                }
                command.exec();
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
            _cgroup_main: cgroup_main,
            cgroup_aperiodic,
            cgroup_periodic,
            _main: pid,
            mode,
            mode_file,
            call_rx,
            _io_udp_tx: udp_io_tx,
            _io_tcp_tx: tcp_io_tx,
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
        Ok(std::fs::File::open(self.cgroup_periodic.get_events_path())
            .typ(SystemError::CGroup)?
            .into())
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

struct IoTxRx {
    udp_io_tx: IoSender<UdpSocket>,
    udp_io_rx: IoReceiver<UdpSocket>,
    tcp_io_tx: IoSender<TcpStream>,
    tcp_io_rx: IoReceiver<TcpStream>,
}

fn send_sockets(base: &Base) -> Result<IoTxRx, a653rs_linux_core::error::TypedError> {
    let (udp_io_tx, udp_io_rx) = io_pair::<UdpSocket>()?;
    let (tcp_io_tx, tcp_io_rx) = io_pair::<TcpStream>()?;
    for addr in base.sockets.iter() {
        match addr {
            PosixSocket::TcpConnect { address } => tcp_io_tx
                .try_send(TcpStream::connect(address.clone()).typ(SystemError::Panic)?)
                .typ(SystemError::Panic)?,
            PosixSocket::Udp { address } => udp_io_tx
                .try_send(UdpSocket::bind(address.clone()).typ(SystemError::Panic)?)
                .typ(SystemError::Panic)?,
        }
    }
    Ok(IoTxRx {
        udp_io_tx,
        udp_io_rx,
        tcp_io_tx,
        tcp_io_rx,
    })
}

#[derive(Debug)]
pub(crate) struct Base {
    name: String,
    hm: PartitionHMTable,
    id: PartitionId,
    bin: PathBuf,
    mounts: Vec<(PathBuf, PathBuf)>,
    cgroup: CGroup,
    sampling_channel: HashMap<String, SamplingConstant>,
    queuing_channel: HashMap<String, QueuingConstant>,
    duration: Duration,
    period: Duration,
    working_dir: TempDir,
    sockets: Vec<PosixSocket>,
}

impl Base {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn unfreeze(&self) -> TypedResult<()> {
        self.cgroup.unfreeze().typ(SystemError::CGroup)
    }

    pub fn sampling_fds(&self) -> Vec<RawFd> {
        self.sampling_channel.values().map(|s| s.fd).collect_vec()
    }

    pub fn queuing_fds(&self) -> Vec<RawFd> {
        self.queuing_channel.values().map(|q| q.fd).collect_vec()
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
        queuing: &HashMap<String, Queuing>,
    ) -> TypedResult<Self> {
        // Todo implement drop for cgroup (in error case)
        let cgroup = CGroup::new_root(cgroup_root, &config.name).typ(SystemError::PartitionInit)?;

        let sampling_channel = sampling
            .iter()
            .filter_map(|(n, s)| s.constant(&config.name).map(|s| (n.clone(), s)))
            .collect();

        let queuing_channel = queuing
            .iter()
            .filter_map(|(n, q)| q.constant(&config.name).map(|q| (n.clone(), q)))
            .collect();

        let working_dir = tempdir().typ(SystemError::PartitionInit)?;
        trace!("CGroup Working directory: {:?}", working_dir.path());
        let bin = config.get_partition_bin()?;

        let base = Base {
            name: config.name,
            id: config.id,
            cgroup,
            bin,
            mounts: config.mounts,
            duration: config.duration,
            period: config.period,
            working_dir,
            hm: config.hm_table,
            sampling_channel,
            sockets: config.sockets,
            queuing_channel,
        };
        // TODO use StartCondition::HmModuleRestart in case of a ModuleRestart!!
        let run =
            Run::new(&base, StartCondition::NormalStart, false).typ(SystemError::PartitionInit)?;

        Ok(Self { base, run })
    }

    pub(crate) fn name(&self) -> &str {
        self.base.name()
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

    pub fn get_base_run(&mut self) -> (&Base, &mut Run) {
        (&self.base, &mut self.run)
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

    pub fn run_post_timeframe(
        &mut self,
        sampling_channels: &mut HashMap<String, Sampling>,
        queuing: &mut HashMap<String, Queuing>,
    ) {
        // TODO remove because a base freeze is not necessary here, as all run_* methods
        // should freeze base themself after execution. Before removal of this, check
        // all run_* methods.
        let _ = self.base.freeze();

        for (name, _) in self
            .base
            .sampling_channel
            .iter()
            .filter(|(_, s)| s.dir == PortDirection::Source)
        {
            sampling_channels.get_mut(name).unwrap().swap();
        }

        for (name, _) in self
            .base
            .queuing_channel
            .iter()
            .filter(|(_, q)| q.dir == PortDirection::Source)
        {
            queuing.get_mut(name).unwrap().swap();
        }
    }

    /// Executes the periodic process for a maximum duration specified through
    /// the `timeout` parameter. Returns whether the periodic process exists
    /// and was run.
    pub fn run_periodic_process(&mut self, timeout: Timeout) -> TypedResult<bool> {
        match self.run.unfreeze_periodic() {
            Ok(true) => {}
            other => return other,
        }

        let mut poller = PeriodicPoller::new(&self.run)?;

        self.base.unfreeze()?;

        while timeout.has_time_left() {
            let event = poller.wait_timeout(&mut self.run, timeout)?;
            match &event {
                PeriodicEvent::Timeout => {}
                PeriodicEvent::Frozen => {
                    self.base.freeze()?;

                    return Ok(true);
                }
                // TODO Error Handling with HM
                PeriodicEvent::Call(e @ PartitionCall::Error(se)) => {
                    e.print_partition_log(self.base.name());
                    match self.base.part_hm().try_action(*se) {
                        Some(RecoveryAction::Module(ModuleRecoveryAction::Ignore)) => {}
                        Some(_) => {
                            return Err(TypedError::new(*se, anyhow!("Received Partition Error")))
                        }
                        None => {
                            return Err(TypedError::new(
                                SystemError::Panic,
                                anyhow!(
                                "Could not get recovery action for requested partition error: {se}"
                            ),
                            ))
                        }
                    };
                }
                PeriodicEvent::Call(c @ PartitionCall::Message(_)) => {
                    c.print_partition_log(self.base.name())
                }
                PeriodicEvent::Call(PartitionCall::Transition(mode)) => {
                    // Only exit run_periodic, if we changed our mode
                    if self.run.handle_transition(&self.base, *mode)?.is_some() {
                        return Ok(true);
                    }
                }
            }
        }

        // TODO being here means that we exceeded the timeout
        // So we should return a SystemError stating that the time was exceeded
        Ok(true)
    }

    pub fn run_aperiodic_process(&mut self, timeout: Timeout) -> TypedResult<bool> {
        match self.run.unfreeze_aperiodic() {
            Ok(true) => {}
            other => return other,
        }

        // Did we even need to unfreeze aperiodic?
        self.base.unfreeze()?;

        while timeout.has_time_left() {
            match &self
                .run
                .receiver()
                .try_recv_timeout(timeout.remaining_time())?
            {
                Some(m @ PartitionCall::Message(_)) => m.print_partition_log(self.base.name()),
                Some(e @ PartitionCall::Error(se)) => {
                    e.print_partition_log(self.base.name());
                    match self.base.part_hm().try_action(*se) {
                        Some(RecoveryAction::Module(ModuleRecoveryAction::Ignore)) => {}
                        Some(_) => {
                            return Err(TypedError::new(*se, anyhow!("Received Partition Error")))
                        }
                        None => {
                            return Err(TypedError::new(
                                SystemError::Panic,
                                anyhow!(
                                "Could not get recovery action for requested partition error: {se}"
                            ),
                            ))
                        }
                    };
                }
                Some(t @ PartitionCall::Transition(mode)) => {
                    // In case of a transition to idle, just sleep. Do not care for the rest
                    t.print_partition_log(self.base.name());
                    if let Some(OperatingMode::Idle) =
                        self.run.handle_transition(&self.base, *mode)?
                    {
                        sleep(timeout.remaining_time());
                        return Ok(true);
                    }
                }
                None => {}
            }
        }

        self.run.freeze_aperiodic()?;

        Ok(true)
    }

    /// Currently the same as run_aperiodic
    pub fn run_start(&mut self, timeout: Timeout, _warm_start: bool) -> TypedResult<()> {
        self.base.unfreeze()?;

        while timeout.has_time_left() {
            match &self
                .run
                .receiver()
                .try_recv_timeout(timeout.remaining_time())?
            {
                Some(m @ PartitionCall::Message(_)) => m.print_partition_log(self.base.name()),
                Some(e @ PartitionCall::Error(se)) => {
                    e.print_partition_log(self.base.name());
                    match self.base.part_hm().try_action(*se) {
                        Some(RecoveryAction::Module(ModuleRecoveryAction::Ignore)) => {}
                        Some(_) => {
                            return Err(TypedError::new(*se, anyhow!("Received Partition Error")))
                        }
                        None => {
                            return Err(TypedError::new(
                                SystemError::Panic,
                                anyhow!(
                                "Could not get recovery action for requested partition error: {se}"
                            ),
                            ))
                        }
                    };
                }
                Some(t @ PartitionCall::Transition(mode)) => {
                    // In case of a transition to idle, just sleep. Do not care for the rest
                    t.print_partition_log(self.base.name());
                    if let Some(OperatingMode::Idle) =
                        self.run.handle_transition(&self.base, *mode)?
                    {
                        sleep(timeout.remaining_time());
                        return Ok(());
                    }
                }
                None => {}
            }
        }

        self.base.freeze()
    }

    /// Handles an error that occurred during self.run_* methods.
    pub fn handle_error(&mut self, err: TypedError) -> LeveledResult<()> {
        debug!("Partition \"{}\" received err: {err:?}", self.base.name());

        let now = Instant::now();

        let action = match self.base.part_hm().try_action(err.err()) {
            None => {
                warn!("Could not map \"{err:?}\" to action. Using Panic action instead");
                match self.base.part_hm().panic {
                    // We do not Handle Module Recovery actions here
                    RecoveryAction::Module(_) => {
                        return TypedResult::Err(err).lev(ErrorLevel::Partition)
                    }
                    RecoveryAction::Partition(action) => action,
                }
            }
            // We do not Handle Module Recovery actions here
            Some(RecoveryAction::Module(_)) => {
                return TypedResult::Err(err).lev(ErrorLevel::Partition)
            }
            Some(RecoveryAction::Partition(action)) => action,
        };

        debug!("Handling: {err:?}");
        debug!("Apply Partition Recovery Action: {action:?}");

        // TODO do not unwrap/expect these errors. Maybe raise Module Level
        // PartitionInit Error?
        match action {
            a653rs_linux_core::health::PartitionRecoveryAction::Idle => self
                .run
                .idle_transition(&self.base)
                .expect("Idle Transition Failed"),
            a653rs_linux_core::health::PartitionRecoveryAction::ColdStart => self
                .run
                .start_transition(&self.base, false, StartCondition::HmPartitionRestart)
                .expect("Start(Cold) Transition Failed"),
            a653rs_linux_core::health::PartitionRecoveryAction::WarmStart => self
                .run
                .start_transition(&self.base, false, StartCondition::HmPartitionRestart)
                .expect("Start(Warm) Transition Failed"),
        }

        trace!("Partition Error Handling took: {:?}", now.elapsed());
        Ok(())
    }
}

impl PartitionConfig {
    /// Get the path to a partition binary
    ///
    /// The [PartitionConfig::image] field must either:
    /// - not contain any path separators, in which case `$PATH` is searched for
    ///   a matching executable (like the `which` command in the shell would do)
    /// - be an absolute path, in which case the path is used verbatim after
    ///   verification that it:
    ///   - exists
    ///   - is a file
    ///   - is executable
    /// - be a relative path starting with `./`, in which case it is resolved
    ///   relative to the hypervisors current workind directory
    fn get_partition_bin(&self) -> TypedResult<PathBuf> {
        let PartitionConfig { image, name, .. } = self;

        // if image is either an absolute path or starts with ./ , it is left as is
        let bin = if image.is_absolute() || image.starts_with(path::Component::CurDir) {
            // verify image exists
            if !image.exists() {
                problem!(Panic, "partition image {image:?} does not exist");
            } else if !image.is_file() {
                problem!(Panic, "partition image {image:?} is not a file");
            } else if image
                .metadata()
                .typ(SystemError::Panic)?
                .permissions()
                .mode()
                & 0b100
                == 0
            {
                problem!(Panic, "partition image {image:?} is not executable")
            } else {
                image.clone()
            }
        }
        // if image does not contain any path separators, try to search it in $PATH
        else if image.components().count() == 1 {
            if let Ok(image_from_path) = which::which(image) {
                image_from_path
            } else {
                problem!(
                    Panic,
                    "could not find image {image:?} for partition {name} in path"
                );
            }
        // other cases are **not** supported
        } else {
            problem!(
                Panic,
                "image {image:?} for partition {name} must start with / or ./",
            );
        };

        Ok(bin)
    }
}

pub(crate) struct PeriodicPoller {
    poll: Poller,
    events: OwnedFd,
}

pub enum PeriodicEvent {
    Timeout,
    Frozen,
    Call(PartitionCall),
}

impl PeriodicPoller {
    const EVENTS_ID: usize = 1;
    const RECEIVER_ID: usize = 2;

    pub fn new(run: &Run) -> TypedResult<PeriodicPoller> {
        let events = run.periodic_events()?;

        let poll = Poller::new().typ(SystemError::Panic)?;
        unsafe {
            poll.add(events.as_raw_fd(), Event::readable(Self::EVENTS_ID))
                .typ(SystemError::Panic)?;
            poll.add(
                run.receiver().as_raw_fd(),
                Event::readable(Self::RECEIVER_ID),
            )
            .typ(SystemError::Panic)?;
        }

        Ok(PeriodicPoller { poll, events })
    }

    pub fn wait_timeout(&mut self, run: &mut Run, timeout: Timeout) -> TypedResult<PeriodicEvent> {
        if run.is_periodic_frozen()? {
            return Ok(PeriodicEvent::Frozen);
        }

        while timeout.has_time_left() {
            let mut events = Events::new();
            self.poll
                .wait(&mut events, Some(timeout.remaining_time()))
                .typ(SystemError::Panic)?;

            for e in events.iter() {
                match e.key {
                    // Got a Frozen event
                    Self::EVENTS_ID => {
                        // Re-sub the readable event
                        self.poll
                            .modify(&mut self.events, Event::readable(Self::EVENTS_ID))
                            .typ(SystemError::Panic)?;

                        // Then check if the cg is actually frozen
                        if run.is_periodic_frozen()? {
                            return Ok(PeriodicEvent::Frozen);
                        }
                    }
                    // got a call events
                    Self::RECEIVER_ID => {
                        // Re-sub the readable event
                        // This will result in the event instantly being ready again should we have
                        // something to read, but that is better than
                        // accidentally missing an event (at the expense of one extra loop per
                        // receive)
                        self.poll
                            .modify(run.receiver(), Event::readable(Self::RECEIVER_ID))
                            .typ(SystemError::Panic)?;

                        // Now receive anything
                        if let Some(call) = run.receiver().try_recv()? {
                            return Ok(PeriodicEvent::Call(call));
                        }
                    }
                    _ => {
                        return Err(anyhow!("Unexpected Event Received: {e:?}"))
                            .typ(SystemError::Panic)
                    }
                }
            }
        }

        Ok(PeriodicEvent::Timeout)
    }
}
