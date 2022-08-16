use std::fs::File;
use std::os::unix::prelude::{AsRawFd, CommandExt, RawFd};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::Result;
use apex_hal::prelude::{OperatingMode, StartCondition};
use clone3::Clone3;
use linux_apex_core::cgroup::CGroup;
use linux_apex_core::fd::PidFd;
use linux_apex_core::health_event::PartitionEvent;
use linux_apex_core::ipc::{channel_pair, IpcReceiver, IpcSender};
use linux_apex_core::partition::{
    DURATION_ENV, HEALTH_SENDER_FD_ENV, IDENTIFIER_ENV, MODE_ENV, NAME_ENV, PERIOD_ENV,
    START_CONDITION_ENV, SYSTEM_TIME_FD_ENV,
};
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::unistd::{chdir, close, pivot_root, setgid, setuid, Gid, Pid, Uid};
use procfs::process::{FDTarget, Process};
use tempfile::{tempdir, TempDir};

//#[derive(Debug)]
pub(crate) struct Partition {
    name: String,
    id: usize,
    cgroup: CGroup,
    working_dir: TempDir,

    bin: PathBuf,
    health_rx: IpcReceiver<PartitionEvent>,
    health_tx: IpcSender<PartitionEvent>,
}

impl Partition {
    pub fn from_cgroup<P1: AsRef<Path>, P2: AsRef<Path>>(
        cgroup_root: P1,
        name: &str,
        id: usize,
        bin: P2,
    ) -> Result<Self> {
        // Todo implement drop for cgroup? (in error case)
        let cgroup = CGroup::new(cgroup_root, name)?;

        let working_dir = tempdir()?;
        trace!("CGroup Working directory: {:?}", working_dir.path());

        //let health_event = unsafe { OwnedFd::from_raw_fd(eventfd(0, EfdFlags::empty())?) };

        let (sender, receiver) = channel_pair::<PartitionEvent>()?;

        Ok(Self {
            name: name.to_string(),
            id,
            cgroup,
            working_dir,
            //health_event,
            //shmem,
            bin: PathBuf::from(bin.as_ref()),
            health_rx: receiver,
            health_tx: sender,
        })
    }

    pub fn wait_event_timeout(&self, timeout: Duration) -> Result<Option<PartitionEvent>> {
        self.health_rx.try_recv_timeout(timeout)
    }

    fn release_fds(keep: &[i32]) -> Result<()> {
        let proc = Process::myself()?;
        for fd in proc
            .fd()?
            .skip(3)
            .flatten()
            .filter(|fd| !keep.contains(&fd.fd))
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

    pub fn freeze(&mut self) -> Result<()> {
        self.cgroup.freeze()
    }

    pub fn unfreeze(&mut self) -> Result<()> {
        self.cgroup.unfreeze()
    }

    pub fn delete(self) -> Result<()> {
        self.cgroup.delete()
    }

    pub fn id(&self) -> usize {
        self.id
    }

    // TODO Declare cpus differently and add effect to this variable
    pub fn restart(&mut self, args: PartitionStartArgs) -> Result<PidFd> {
        self.cgroup.kill_all_wait()?;

        self.freeze()?;
        let real_uid = nix::unistd::getuid();
        let real_gid = nix::unistd::getgid();

        let pid = match unsafe {
            Clone3::default()
                .flag_newcgroup()
                .flag_newuser()
                .flag_newpid()
                .flag_newns()
                .flag_newipc()
                .flag_newnet()
                .flag_into_cgroup(&self.cgroup.get_fd()?.as_raw_fd())
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

                Self::print_fds();
                // Release all unneeded fd's
                Self::release_fds(&[
                    args.system_time,
                    //self.health_event.as_raw_fd()
                    self.health_tx.as_raw_fd(),
                ])
                .unwrap();

                // Mount working directory as tmpfs (TODO with size?)
                mount::<str, _, _, str>(
                    None,
                    self.working_dir.path(),
                    Some("tmpfs"),
                    MsFlags::empty(),
                    Some("size=500k"),
                    //None,
                )
                .unwrap();

                // Mount binary
                let bin = self.working_dir.path().join("bin");
                File::create(&bin).unwrap();
                mount::<_, _, str, str>(
                    Some(&self.bin),
                    &bin,
                    None,
                    MsFlags::MS_RDONLY | MsFlags::MS_BIND,
                    None,
                )
                .unwrap();

                // TODO bind-mount requested devices

                // Mount proc
                let proc = self.working_dir.path().join("proc");
                std::fs::create_dir(proc.as_path()).unwrap();
                mount::<str, _, str, str>(
                    Some("/proc"),
                    proc.as_path(),
                    Some("proc"),
                    MsFlags::empty(),
                    None,
                )
                .unwrap();

                // Mount CGroup V2
                let cgroup = self.working_dir.path().join("sys/fs/cgroup");
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
                chdir(self.working_dir.path()).unwrap();
                pivot_root(".", ".").unwrap();
                umount2(".", MntFlags::MNT_DETACH).unwrap();
                chdir("/").unwrap();

                // Run binary
                // TODO detach stdio
                let err = Command::new("/bin")
                    // Set Partition Name Env
                    .env(NAME_ENV, &self.name)
                    .env(PERIOD_ENV, args.period.as_nanos().to_string())
                    .env(DURATION_ENV, args.duration.as_nanos().to_string())
                    .env(IDENTIFIER_ENV, self.id.to_string())
                    .env(START_CONDITION_ENV, (args.condition as u32).to_string())
                    .env(MODE_ENV, (args.mode as u32).to_string())
                    .env(SYSTEM_TIME_FD_ENV, args.system_time.to_string())
                    .env(HEALTH_SENDER_FD_ENV, self.health_tx.as_raw_fd().to_string())
                    .exec();
                error!("{err:?}");

                unsafe { libc::_exit(0) };
            }
            child => child,
        };
        debug!("Child Pid: {pid}");

        PidFd::try_from(Pid::from_raw(pid))
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PartitionStartArgs {
    pub condition: StartCondition,
    pub mode: OperatingMode,
    pub duration: Duration,
    pub period: Duration,
    pub system_time: RawFd,
}
