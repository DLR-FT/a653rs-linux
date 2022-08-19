use std::fs::{read_to_string, File};
use std::os::unix::prelude::OwnedFd;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use itertools::Itertools;
use nix::unistd::Pid;
use walkdir::WalkDir;

use crate::error::{TypedResult, ResultExt, SystemError, ErrorExt};


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
    const FREEZE_FILE: &'static str = "cgroup.freeze";
    const EVENTS_FILE: &'static str = "cgroup.events";

    pub fn mount_point() -> TypedResult<PathBuf> {
        procfs::process::Process::myself()
            .typ_res(SystemError::Panic)?
            .mountinfo()
            .typ_res(SystemError::Panic)?
            .iter()
            .find(|m| m.fs_type.eq("cgroup2"))
            .ok_or_else(|| anyhow!("no cgroup2 mount found").typ_err(SystemError::Panic))
            .map(|m| m.mount_point.clone())
    }

    pub fn new<P: AsRef<Path>>(root: P, name: &str) -> TypedResult<Self> {
        let path = PathBuf::from(root.as_ref()).join(name);
        //trace!("New CGroup: {path:?}");

        if !path.exists() {
            trace!("Creating Cgroup, {path:?}");
            std::fs::create_dir(&path).typ_res(SystemError::CGroup)?;
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

    pub fn get_procs_fd(&self) -> TypedResult<OwnedFd> {
        File::open(self.path.join(Self::MEMBER_FILE)).typ_res(SystemError::CGroup).map(OwnedFd::from)
    }

    pub fn add_process(&self, pid: Pid) -> TypedResult<()> {
        Self::add_process_to(&self.path, pid).typ_res(SystemError::CGroup)
    }

    pub fn member(&self) -> TypedResult<Vec<Pid>> {
        read_to_string(self.path().join(Self::MEMBER_FILE)).map(|s| {
            s.lines()
                .flat_map(|l| l.parse())
                .map(Pid::from_raw)
                .collect()
        }).typ_res(SystemError::CGroup)
    }

    pub fn add_process_to<P: AsRef<Path>>(path: P, pid: Pid) -> TypedResult<()> {
        std::fs::write(path.as_ref().join(Self::MEMBER_FILE), pid.to_string()).typ_res(SystemError::CGroup)
    }

    pub fn events_file(&self) -> TypedResult<OwnedFd> {
        File::open(self.path().join(Self::EVENTS_FILE)).typ_res(SystemError::CGroup).map(OwnedFd::from)
    }

    // TODO continue
    pub fn is_frozen(&self) -> Result<bool> {
        let frozen = read_to_string(self.path().join(Self::FREEZE_FILE))?;
        let frozen: usize = frozen.trim().parse()?;
        match frozen {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(anyhow!("Unexpected Number")),
        }
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

    pub fn freeze(&self) -> Result<()> {
        // TODO remove debug
        std::fs::write(self.path.join(Self::FREEZE_FILE), "1")?;
        Ok(())
    }

    pub fn unfreeze(&self) -> Result<()> {
        std::fs::write(self.path.join(Self::FREEZE_FILE), "0")?;
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
