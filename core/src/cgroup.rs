use std::fs::{read_to_string, File};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use itertools::Itertools;
use nix::unistd::Pid;
use walkdir::WalkDir;

// TODO think about completely changing this.
// Because CGroups are a hierarchy and Parents need to consider their children,
// A different representation may be necessary.
// Also maybe we dont even need to delete the cgroups after we are done
// We may need to verify "Domain" "Domain Threaded" and "Thread" state of CGroups
// TODO it should be possible to use an already created cgroup incase the user provides us a group with cpuset-enabled
#[derive(Debug, Clone)]
pub struct CGroup {
    path: PathBuf,
}

impl CGroup {
    const MEMBER_FILE: &'static str = "cgroup.procs";

    pub fn mount_point() -> Result<PathBuf> {
        let mnt = procfs::process::Process::myself()?
            .mountinfo()?
            .iter()
            .find(|m| m.fs_type.eq("cgroup2"))
            .ok_or_else(|| anyhow!("no cgroup2 mount found"))
            .map(|m| m.mount_point.clone());

        //trace!("cgroups mount point is {mnt:?}");
        mnt
    }

    pub fn new<P: AsRef<Path>>(root: P, name: &str) -> Result<Self> {
        let path = PathBuf::from(root.as_ref()).join(name);
        //trace!("New CGroup: {path:?}");

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
        std::fs::write(path.as_ref().join(Self::MEMBER_FILE), pid.to_string())?;
        Ok(())
    }

    pub fn kill_all_wait(&self) -> Result<()> {
        let pid_path = self.path.join(Self::MEMBER_FILE);
        std::fs::write(self.path.join("cgroup.kill"), "1")?;
        while !read_to_string(&pid_path)?.is_empty() {}
        Ok(())
    }

    pub fn delete(&self) -> Result<()> {
        self.kill_all_wait()?;
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
}

impl From<PathBuf> for CGroup {
    fn from(path: PathBuf) -> Self {
        Self { path }
    }
}
