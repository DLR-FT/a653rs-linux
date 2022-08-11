use std::fs::File;
use std::os::unix::prelude::{AsRawFd, CommandExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::Result;
use apex_hal::prelude::{OperatingMode, StartCondition};
use clone3::Clone3;
use linux_apex_core::cgroup::CGroup;
use linux_apex_core::fd::{Fd, PidFd};
use linux_apex_core::file::{get_memfd, TempFile};
use linux_apex_core::partition::{
    DURATION_ENV, IDENTIFIER_ENV, NAME_ENV, PARTITION_STATE_FILE, PERIOD_ENV, START_CONDITION_ENV,
    SYSTEM_TIME_FILE,
};
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::sys::eventfd::{eventfd, EfdFlags};
use nix::unistd::{chdir, close, pivot_root, setgid, setuid, Gid, Pid, Uid};
use procfs::process::Process;
use tempfile::{tempdir, TempDir};

//#[derive(Debug)]
pub(crate) struct Partition {
    name: String,
    id: usize,
    cg: CGroup,
    wd: TempDir,
    he: Fd,
    state: TempFile<OperatingMode>,
    //shmem: Shmem<[u8; 2]>,
    bin: PathBuf,
}

impl Partition {
    pub fn from_cgroup<P1: AsRef<Path>, P2: AsRef<Path>>(
        cgroup_root: P1,
        name: &str,
        id: usize,
        bin: P2,
    ) -> Result<Self> {
        // Todo implement drop for cgroup? (in error case)
        let cg = CGroup::new(cgroup_root, name)?;

        let wd = tempdir()?;
        trace!("CGroup Working directory: {:?}", wd.path());

        let he = eventfd(0, EfdFlags::empty())?.try_into()?;
        let state = TempFile::new(PARTITION_STATE_FILE)?;

        Ok(Self {
            name: name.to_string(),
            id,
            cg,
            wd,
            he,
            state,
            //shmem,
            bin: PathBuf::from(bin.as_ref()),
        })
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
        self.cg.freeze()
    }

    pub fn unfreeze(&mut self) -> Result<()> {
        self.cg.unfreeze()
    }

    pub fn delete(self) -> Result<()> {
        self.cg.delete()
    }

    pub fn id(&self) -> usize {
        self.id
    }

    // TODO Declare cpus differently and add effect to this variable
    pub fn restart(&mut self, args: PartitionStartArgs) -> Result<PidFd> {
        self.cg.kill_all_wait()?;

        self.freeze()?;

        let pid = match unsafe {
            Clone3::default()
                .flag_newcgroup()
                .flag_newuser()
                .flag_newpid()
                .flag_newns()
                .flag_newipc()
                .flag_newnet()
                .flag_into_cgroup(&self.cg.get_fd()?.as_raw_fd())
                .call()
        }? {
            0 => {
                setuid(Uid::from_raw(0)).unwrap();
                setgid(Gid::from_raw(0)).unwrap();

                let sys_time = get_memfd(SYSTEM_TIME_FILE).unwrap();
                Self::print_fds();

                Self::release_fds(&[sys_time, self.he.as_raw_fd(), self.state.fd()]).unwrap();

                // Set to cold_start
                self.state.write(&OperatingMode::ColdStart).unwrap();

                // Mount working directory as tmpfs (TODO with size?)
                mount::<str, _, _, str>(
                    None,
                    self.wd.path(),
                    Some("tmpfs"),
                    MsFlags::empty(),
                    //Some("mode=0700,uid=0,size=50000k"),
                    None,
                )
                .unwrap();

                // Mount binary
                let bin = self.wd.path().join("bin");
                File::create(&bin).unwrap();
                mount::<_, _, str, str>(
                    Some(&self.bin),
                    &bin,
                    None,
                    MsFlags::MS_RDONLY | MsFlags::MS_BIND,
                    None,
                )
                .unwrap();

                // Mount proc
                let proc = self.wd.path().join("proc");
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
                let cgroup = self.wd.path().join("sys/fs/cgroup");
                std::fs::create_dir_all(&cgroup).unwrap();
                mount::<str, _, str, str>(
                    //Some(self.child_cg.path().to_str().unwrap()),
                    None,
                    //Some("sys/fs/cgroup"),
                    cgroup.as_path(),
                    Some("cgroup2"),
                    MsFlags::empty(),
                    None,
                )
                .unwrap();

                // Change working directory and root (unmount old root)
                chdir(self.wd.path()).unwrap();
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
                    .exec();
                error!("{err:?}");

                unsafe { libc::_exit(0) };
            }
            child => child,
        };
        debug!("Child Pid: {pid}");

        std::fs::write(
            PathBuf::from("/proc").join(pid.to_string()).join("uid_map"),
            format!("0 {} 1", nix::unistd::getuid().as_raw()),
        )?;
        std::fs::write(
            PathBuf::from("/proc")
                .join(pid.to_string())
                .join("setgroups"),
            b"deny",
        )?;
        std::fs::write(
            PathBuf::from("/proc").join(pid.to_string()).join("gid_map"),
            format!("0 {} 1", nix::unistd::getgid().as_raw()).as_bytes(),
        )?;

        PidFd::try_from(Pid::from_raw(pid))
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PartitionStartArgs {
    pub condition: StartCondition,
    pub duration: Duration,
    pub period: Duration,
}
