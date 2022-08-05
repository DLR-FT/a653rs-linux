use std::fs::File;
use std::os::unix::prelude::{AsRawFd, CommandExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use apex_hal::prelude::OperatingMode;
use clone3::Clone3;
use linux_apex_core::cgroup::DomainCGroup;
use linux_apex_core::file::{get_fd, TempFile};
use linux_apex_core::partition::{
    HEALTH_STATE_FILE, NAME_ENV, PARTITION_STATE_FILE, SYSTEM_TIME_FILE,
};
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::unistd::{chdir, close, pivot_root, setgid, setuid, Gid, Uid};
use procfs::process::Process;
use tempfile::{tempdir, TempDir};

//#[derive(Debug)]
pub(crate) struct Partition {
    name: String,
    cg: DomainCGroup,
    wd: TempDir,
    hm: TempFile<u8>,
    state: TempFile<OperatingMode>,
    //shmem: Shmem<[u8; 2]>,
    bin: PathBuf,
}

impl Partition {
    pub fn from_cgroup<P1: AsRef<Path>, P2: AsRef<Path>>(
        cgroup_root: P1,
        name: &str,
        bin: P2,
    ) -> Result<Self> {
        // Todo implement drop for cgroup? (in error case)
        let cg = DomainCGroup::new(cgroup_root, name)?;

        let wd = tempdir()?;
        trace!("CGroup Working directory: {:?}", wd.path());

        let hm = TempFile::new(HEALTH_STATE_FILE)?;
        hm.lock_trunc()?;
        let state = TempFile::new(PARTITION_STATE_FILE)?;
        state.lock_trunc()?;

        Ok(Self {
            name: name.to_string(),
            cg,
            wd,
            hm,
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

    pub fn initialize(&mut self) {
        self.cg.kill_all().unwrap();

        self.freeze().unwrap();

        let pid = match unsafe {
            Clone3::default()
                .flag_newcgroup()
                .flag_newuser()
                .flag_newpid()
                .flag_newns()
                .flag_newipc()
                .flag_newnet()
                .flag_into_cgroup(&self.cg.get_fd().unwrap().as_raw_fd())
                .call()
        }
        .unwrap()
        {
            0 => {
                setuid(Uid::from_raw(0)).unwrap();
                setgid(Gid::from_raw(0)).unwrap();

                let sys_time = get_fd(SYSTEM_TIME_FILE).unwrap();

                Self::release_fds(&[sys_time, self.hm.get_fd(), self.state.get_fd()]).unwrap();

                Self::print_fds();
                debug!("Stdout fd: {}", std::io::stdout().as_raw_fd());

                // Set to cold_start
                self.state.write(OperatingMode::ColdStart).unwrap();

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
        )
        .unwrap();
        std::fs::write(
            PathBuf::from("/proc")
                .join(pid.to_string())
                .join("setgroups"),
            b"deny",
        )
        .unwrap();
        std::fs::write(
            PathBuf::from("/proc").join(pid.to_string()).join("gid_map"),
            format!("0 {} 1", nix::unistd::getgid().as_raw()).as_bytes(),
        )
        .unwrap();
        //forget(client);
    }
}
