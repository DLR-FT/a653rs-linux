use std::fs::read_to_string;
use std::fs::File;
use std::os::unix::prelude::AsRawFd;
use std::path::Path;
use std::path::PathBuf;
use std::vec;

use anyhow::Result;
use nix::sys::epoll::epoll_create;
use nix::sys::epoll::epoll_ctl;
use nix::sys::epoll::epoll_wait;
use nix::sys::epoll::EpollEvent;
use nix::sys::epoll::EpollFlags;
use nix::sys::epoll::EpollOp;
use nix::unistd::close;
use nix::unistd::Pid;

pub type DomainCGroup = CGroup<true>;
pub type ThreadedCGroup = CGroup<false>;

// TODO think about completely changing this.
// Because CGroups are a hierarchy and Parents need to consider their children,
// A different representation may be necessary.
// Also maybe we dont even need to delete the cgroups after we are done
// We may need to verify "Domain" "Domain Threaded" and "Thread" state of CGroups
// TODO it should be possible to use an already created cgroup incase the user provides us a group with cpuset-enabled
#[derive(Debug)]
pub struct CGroup<const DOMAIN: bool> {
    path: PathBuf,
    member: Vec<Pid>,
}

impl<const DOMAIN: bool> CGroup<DOMAIN> {
    pub fn path(&self) -> PathBuf {
        self.path.clone()
    }

    pub fn freeze(&mut self) -> Result<()> {
        std::fs::write(self.path.join("cgroup.freeze"), "1")?;
        Ok(())
    }

    pub fn unfreeze(&mut self) -> Result<()> {
        std::fs::write(self.path.join("cgroup.freeze"), "0")?;
        Ok(())
    }

    pub fn get_fd(&self) -> Result<File> {
        Ok(File::open(&self.path)?)
    }

    // TODO Add timeout ?
    fn kill_all_with_file(&self, file: &str) -> Result<()> {
        let epoll = epoll_create()?;
        let pid_path = self.path.join(file);
        let pids = File::open(&pid_path)?;
        let mut info = EpollEvent::new(EpollFlags::EPOLLIN, 0);
        epoll_ctl(epoll, EpollOp::EpollCtlAdd, pids.as_raw_fd(), &mut info)?;

        std::fs::write(self.path.join("cgroup.kill"), "1")?;

        let mut events = [EpollEvent::empty()];
        while !read_to_string(&pid_path)?.is_empty() {
            epoll_wait(epoll, &mut events, 500)?;
        }
        close(epoll)?;

        Ok(())
    }
}

impl CGroup<true> {
    const MEMBER_FILE: &'static str = "cgroup.procs";

    pub fn new<P: AsRef<Path>>(root: P, name: &str) -> Result<Self> {
        let path = PathBuf::from(root.as_ref()).join(name);
        trace!("New CGroup: {path:?}");

        if !path.exists() {
            std::fs::create_dir(&path).unwrap();
        }

        // TODO use cpuset with feature opt in
        //let cont = path.join("cgroup.subtree_control");
        //println!("{cont:?}");
        //unsafe {exit(1)};
        //std::fs::write(path.join("cgroup.subtree_control"), b"+pids")?;
        //sleep(Duration::from_secs(1));
        //sleep(Duration::from_secs(1));

        Ok(CGroup {
            path,
            member: vec![],
        })
    }

    pub fn add_process(&mut self, pid: Pid) -> Result<()> {
        Self::add_process_to(&self.path, pid)?;
        self.member.push(pid);
        Ok(())
    }

    pub fn add_process_to<P: AsRef<Path>>(path: P, pid: Pid) -> Result<()> {
        std::fs::write(path.as_ref().join(Self::MEMBER_FILE), pid.to_string()).unwrap();
        Ok(())
    }

    pub fn kill_all(&self) -> Result<()> {
        self.kill_all_with_file(Self::MEMBER_FILE)
    }

    pub fn delete(&self) -> Result<()> {
        self.kill_all()?;
        std::fs::remove_dir(&self.path)?;
        Ok(())
    }
}

impl CGroup<false> {
    const MEMBER_FILE: &'static str = "cgroup.threads";

    pub fn new<P: AsRef<Path>>(root: P, name: &str) -> Result<Self> {
        let path = PathBuf::from(root.as_ref()).join(name);
        trace!("New CGroup: {path:?}");

        if !path.exists() {
            std::fs::create_dir(&path).unwrap();
        }

        std::fs::write(path.join("cgroup.type"), b"threaded")?;

        Ok(CGroup {
            path,
            member: vec![],
        })
    }

    pub fn add_thread(&mut self, pid: Pid) -> Result<()> {
        Self::add_thread_to(&self.path, pid)?;
        self.member.push(pid);
        Ok(())
    }

    pub fn add_thread_to<P: AsRef<Path>>(path: P, pid: Pid) -> Result<()> {
        std::fs::write(path.as_ref().join(Self::MEMBER_FILE), pid.to_string()).unwrap();
        Ok(())
    }

    pub fn kill_all(&self) -> Result<()> {
        self.kill_all_with_file(Self::MEMBER_FILE)
    }

    pub fn delete(&self) -> Result<()> {
        self.kill_all()?;
        std::fs::remove_dir(&self.path)?;
        Ok(())
    }
}
