use std::fs::{read_to_string, File};
use std::os::unix::prelude::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Result};
use inotify::{Inotify, WatchMask};
use itertools::Itertools;
use nix::unistd::Pid;
use polling::{Event, Poller};
use walkdir::WalkDir;

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
        let pid_path = self.path.join(file);
        let mut notify = Inotify::init()?;
        notify.add_watch(&pid_path, WatchMask::MODIFY)?;
        let poller = Poller::new()?;
        poller.add(notify.as_raw_fd(), Event::readable(0))?;

        std::fs::write(self.path.join("cgroup.kill"), "1")?;
        while !read_to_string(&pid_path)?.is_empty() {
            poller.wait(Vec::new().as_mut(), Some(Duration::from_millis(500)))?;
            poller.modify(notify.as_raw_fd(), Event::readable(0))?;
        }

        Ok(())
    }
}

impl CGroup<true> {
    const MEMBER_FILE: &'static str = "cgroup.procs";

    pub fn mount_point() -> Result<PathBuf> {
        let mnt = procfs::process::Process::myself()?
            .mountinfo()?
            .iter()
            .find(|m| m.fs_type.eq("cgroup2"))
            .ok_or_else(|| anyhow!("no cgroup2 mount found"))
            .map(|m| m.mount_point.clone());

        trace!("cgroups mount point is {mnt:?}");
        mnt
    }

    pub fn new<P: AsRef<Path>>(root: P, name: &str) -> Result<Self> {
        let path = PathBuf::from(root.as_ref()).join(name);
        trace!("New CGroup: {path:?}");

        if !path.exists() {
            trace!("Creating Cgroup, {path:?}");
            std::fs::create_dir(&path)?;
        }

        // TODO use cpuset with feature opt in
        //let cont = path.join("cgroup.subtree_control");
        //println!("{cont:?}");
        //unsafe {exit(1)};
        //std::fs::write(path.join("cgroup.subtree_control"), b"+pids")?;
        //sleep(Duration::from_secs(1));
        //sleep(Duration::from_secs(1));

        Ok(CGroup { path })
    }

    pub fn add_process(&mut self, pid: Pid) -> Result<()> {
        Self::add_process_to(&self.path, pid)?;
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
        for d in WalkDir::new(&self.path)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_dir())
            .sorted_by(|a, b| a.depth().cmp(&b.depth()).reverse())
        {
            std::fs::remove_dir(&d.path())?;
            trace!("Removed {:?}", d.path().as_os_str())
        }
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

        Ok(CGroup { path })
    }

    pub fn add_thread(&mut self, pid: Pid) -> Result<()> {
        Self::add_thread_to(&self.path, pid)?;
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
