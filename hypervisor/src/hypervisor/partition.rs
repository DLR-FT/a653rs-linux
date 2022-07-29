use std::fs::File;
use std::os::unix::prelude::{AsRawFd, CommandExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::Result;
use clone3::Clone3;
use linux_apex_core::cgroup::DomainCGroup;
use linux_apex_core::partition::{get_fd, SYSTEM_TIME};
use linux_apex_core::shmem::Shmem;
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::unistd::{chdir, close, dup, pivot_root, setgid, setuid, Gid, Uid};
use procfs::process::Process;
use tempfile::{tempdir, TempDir};

//#[derive(Debug)]
pub(crate) struct Partition {
    name: String,
    cg: DomainCGroup,
    wd: TempDir,
    shmem: Shmem<[u8; 2]>,
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
        let shmem = unsafe { Shmem::new("shmem", [0, 0])? };

        //let sys_call = ipc_channel::ipc::channel()?;

        Ok(Self {
            cg,
            shmem,
            bin: PathBuf::from(bin.as_ref()),
            name: name.to_string(),
            wd,
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
                //forget(server);
                //println!("{:#?}", procfs::process::Process::myself().unwrap().fd_count().unwrap());
                //for fd in procfs::process::Process::myself().unwrap().fd().unwrap(){
                //    println!("{:#?}", fd);
                //}
                //for fd in procfs::process::Process::myself().unwrap().fd().unwrap().skip(3){
                //    close(fd.unwrap().fd).unwrap();
                //}

                setuid(Uid::from_raw(0)).unwrap();
                setgid(Gid::from_raw(0)).unwrap();

                let sys_time = get_fd(SYSTEM_TIME).unwrap();
                Self::release_fds(&[self.shmem.fd(), sys_time]).unwrap();
                //Self::print_fds();

                mount::<str, _, _, str>(
                    None,
                    self.wd.path(),
                    Some("tmpfs"),
                    MsFlags::empty(),
                    //Some("mode=0700,uid=0,size=50000k"),
                    None,
                )
                .unwrap();

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

                chdir(self.wd.path()).unwrap();
                pivot_root(".", ".").unwrap();
                umount2(".", MntFlags::MNT_DETACH).unwrap();
                chdir("/").unwrap();

                //Self::print_mountinfo();

                let err = Command::new("/bin").arg(&self.name).exec();
                error!("{err:?}");

                unsafe { libc::_exit(0) };
            }
            child => child,
        };
        info!("Child Pid: {pid}");
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
