use std::fs::File;
use std::fs::read_to_string;
use std::os::unix::prelude::AsRawFd;
use std::os::unix::prelude::RawFd;
use std::path::Path;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;
use std::vec;

use anyhow::Result;
use nix::fcntl::OFlag;
use nix::fcntl::open;
use nix::sys::epoll::EpollEvent;
use nix::sys::epoll::EpollFlags;
use nix::sys::epoll::EpollOp;
use nix::sys::epoll::epoll_create;
use nix::sys::epoll::epoll_ctl;
use nix::sys::epoll::epoll_wait;
use nix::sys::stat::Mode;
use nix::unistd::Pid;
use nix::unistd::close;

#[derive(Debug)]
pub(crate) struct CGroup {
    path: PathBuf,
    member: Vec<Pid>,
}

impl CGroup {
    pub fn new<P: AsRef<Path>>(root: P, name: &str) -> Result<Self> {
        let path = PathBuf::from(root.as_ref()).join(name);
        println!("{path:?}");

        if !path.exists() {
            std::fs::create_dir(&path).unwrap();
        }

        // TODO use cpuset with feature
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

    pub fn path(&self) -> PathBuf {
        self.path.clone()
    }

    pub fn add_process(&mut self, pid: Pid) -> Result<()> {
        Self::add_process_to(&self.path, pid)?;
        self.member.push(pid);
        Ok(())
    }

    pub fn add_process_to<P: AsRef<Path>>(path: P, pid: Pid) -> Result<()> {
        std::fs::write(path.as_ref().join("cgroup.procs"), pid.to_string()).unwrap();
        Ok(())
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
        //TODO does this leak to many fd ?
        Ok(File::open(&self.path)?)
    }

    // TODO: Not nice that this doesnt consume.
    // How to drop consuming?!
    pub fn delete(&self) -> Result<()> {
        let epoll = epoll_create()?;
        let pid_path = self.path.join("cgroup.procs");
        let pids = File::open(&pid_path)?;
        let mut info = EpollEvent::new(EpollFlags::EPOLLIN,
            0);
        epoll_ctl(epoll, EpollOp::EpollCtlAdd, pids.as_raw_fd(), &mut info)?;
        
        std::fs::write(self.path.join("cgroup.kill"), "1")?;

        // TODO: what to do if this blocks forever?
        let mut events = [EpollEvent::empty()];
        while !read_to_string(&pid_path)?.is_empty() {
            epoll_wait(epoll, &mut events, 500)?;
        }
        close(epoll)?;
        std::fs::remove_dir(&self.path)?;
        Ok(())
    }
}
