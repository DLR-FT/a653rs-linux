use std::os::unix::prelude::RawFd;
use std::path::Path;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;
use std::vec;

use anyhow::Result;
use nix::fcntl::OFlag;
use nix::sys::stat::Mode;
use nix::unistd::Pid;

#[derive(Debug)]
pub(crate) struct CGroup {
    path: PathBuf,
    member: Vec<Pid>,
}

impl CGroup {
    pub fn new<P: AsRef<Path>>(root: P, name: &str) -> Result<Self> {
        let path = PathBuf::from(root.as_ref()).join(name);

        if !path.exists() {
            std::fs::create_dir(&path)?;
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
        std::fs::write(self.path.join("cgroup.procs"), pid.to_string()).unwrap();
        self.member.push(pid);
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

    pub fn get_fd(&self) -> Result<RawFd> {
        Ok(nix::fcntl::open(
            &self.path,
            OFlag::O_RDONLY,
            Mode::empty(),
        )?)
    }

    pub fn delete(self) -> Result<()> {
        //for (_, c) in self.children{
        //  c.delete()?
        //}

        //let pid = nix::unistd::getpid();
        //if self.member.contains(&pid){
        //  if let Some(path) = self.path.parent(){
        //    std::fs::write(path.join("cgroup.procs"), pid.to_string()).unwrap();
        //    sleep(Duration::from_secs(1))
        //  }
        //}

        std::fs::write(self.path.join("cgroup.kill"), "1")?;
        //TODO await file change instead of fixed duration
        sleep(Duration::from_millis(100));
        std::fs::remove_dir(self.path)?;
        Ok(())
    }
}
