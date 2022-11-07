//! Implementation of the Linux *cgroup* feature
//!
//! This mdoule itself assumes a good understanding of the relevant technology,
//! meaning that certain concepts won't be explained here.
//! For more information regarding cgroup, have a look at the accompanying
//! man-page and [this](https://docs.kernel.org/admin-guide/cgroup-v2.html) resource.
use std::collections::HashMap;
use std::fs::{read_to_string, File};
use std::os::unix::prelude::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::anyhow;
use itertools::Itertools;
use nix::unistd::Pid;
use polling::{Event, Poller};
use walkdir::WalkDir;

use crate::error::{ResultExt, SystemError, TypedResult};

/// An internal reference to a certain cgroup
// TODO think about completely changing this.
// Because CGroups are a hierarchy and Parents need to consider their children,
// A different representation may be necessary, probably a tree with n children.
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

    /// Returns the first mount point on the host filesystem of the cgroup that this process belongs to
    pub fn mount_point() -> TypedResult<PathBuf> {
        procfs::process::Process::myself()
            .typ(SystemError::Panic)?
            .mountinfo()
            .typ(SystemError::Panic)?
            .iter()
            .find(|m| m.fs_type.eq("cgroup2")) // TODO A process can have several cgroup mounts
            .ok_or_else(|| anyhow!("no cgroup2 mount found"))
            .typ(SystemError::Panic)
            .map(|m| m.mount_point.clone())
    }

    /// Create a cgroup inside another one
    // TODO: Switch to a tree data structure for cgroups.
    pub fn new<P: AsRef<Path>>(parent: P, child_name: &str) -> TypedResult<Self> {
        let path = PathBuf::from(parent.as_ref()).join(child_name);
        //trace!("New CGroup: {path:?}");

        if !path.exists() {
            trace!("Creating Cgroup, {path:?}");
            std::fs::create_dir(&path).typ(SystemError::CGroup)?;
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

    /// Returns a file descriptor to the list of the processes that belong to this cgroup
    /// TODO: Does this need to be public?
    pub fn get_procs_fd(&self) -> TypedResult<OwnedFd> {
        File::open(self.path.join(Self::MEMBER_FILE))
            .typ(SystemError::CGroup)
            .map(OwnedFd::from)
    }

    /// Moves a process to this cgroup
    pub fn add_process(&self, pid: Pid) -> TypedResult<()> {
        Self::add_process_to(&self.path, pid)
    }

    /// Returns a Vec containing the PIDs of all processes beloning to this cgroup
    pub fn member(&self) -> TypedResult<Vec<Pid>> {
        read_to_string(self.path().join(Self::MEMBER_FILE))
            .map(|s| {
                s.lines()
                    .flat_map(|l| l.parse())
                    .map(Pid::from_raw)
                    .collect()
            })
            .typ(SystemError::CGroup)
    }

    /// Moves a process to any cgroup
    pub fn add_process_to<P: AsRef<Path>>(path: P, pid: Pid) -> TypedResult<()> {
        std::fs::write(path.as_ref().join(Self::MEMBER_FILE), pid.to_string())
            .typ(SystemError::CGroup)
    }

    /// Returns a file descriptor to the events file of this cgroup
    // TODO: Does this need to be public?
    pub fn events_file(&self) -> TypedResult<OwnedFd> {
        File::open(self.path().join(Self::EVENTS_FILE))
            .typ(SystemError::CGroup)
            .map(OwnedFd::from)
    }

    /// Returns a Vec containing the events of this cgroup
    // TODO: It's weird that member() and this function do a similar thing but have so different names.
    pub fn read_event_file(&self) -> TypedResult<HashMap<String, bool>> {
        let ctn = read_to_string(self.path().join(Self::EVENTS_FILE)).typ(SystemError::CGroup)?;
        Ok(ctn
            .lines()
            .filter(|l| !l.is_empty())
            .filter_map(|l| l.split_once(' '))
            .map(|(k, b)| (k.to_string(), b.eq("1")))
            .collect())
    }

    /// Checks if this cgroup is frozen
    pub fn is_frozen(&self) -> TypedResult<bool> {
        let event = self.read_event_file()?;
        event
            .get("frozen")
            .ok_or_else(|| anyhow!("No \"frozen\" in event file"))
            .typ(SystemError::CGroup)
            .map(|b| *b)
    }

    /// Checks if this cgroup contains any active processes
    pub fn is_populated(&self) -> TypedResult<bool> {
        let event = self.read_event_file()?;
        event
            .get("populated")
            .ok_or_else(|| anyhow!("No \"populated\" in event file"))
            .typ(SystemError::CGroup)
            .map(|b| *b)
    }

    /// Kills all processes in this cgroup with a given timeout
    ///
    /// Returns
    /// Ok(true) in case of success
    /// Ok(false) in case of a timeout
    /// Err(_) in case an error
    pub fn kill_all_timeout(&self, timeout: Duration) -> TypedResult<bool> {
        let start = Instant::now();

        std::fs::write(self.path.join("cgroup.kill"), "1").typ(SystemError::CGroup)?;

        let event_file = self.events_file()?;
        let poller = Poller::new()
            .map_err(anyhow::Error::from)
            .typ(SystemError::Panic)?;
        poller
            .add(event_file.as_raw_fd(), Event::readable(42))
            .map_err(anyhow::Error::from)
            .typ(SystemError::Panic)?;

        // We cannot use timeout directly and pass it to poller, as it may report a change of the
        // event file that is unrelated to the populated property.
        let mut leftover_time = timeout.saturating_sub(start.elapsed());
        loop {
            // Stop loop if time is exceeded
            if leftover_time <= Duration::ZERO {
                return Ok(false);
            };

            // Check if cgroup is populated (check may take time)
            if !self.is_populated()? {
                return Ok(true);
            }

            // Check again if time is exceeded
            leftover_time = timeout.saturating_sub(start.elapsed());
            if leftover_time <= Duration::ZERO {
                return Ok(false);
            };

            // Poll wait for event with given leftover time as timeout
            poller
                .wait(Vec::new().as_mut(), Some(leftover_time))
                .typ(SystemError::Panic)?;
            poller
                .modify(event_file.as_raw_fd(), Event::readable(42))
                .map_err(anyhow::Error::from)
                .typ(SystemError::Panic)?;

            // determine new leftover time
            leftover_time = timeout.saturating_sub(start.elapsed());
        }
    }

    /// Deletes a cgroup by killing it's processes and removing the directory afterwards
    pub fn delete(&self, timeout: Duration) -> TypedResult<()> {
        self.kill_all_timeout(timeout)?;
        for d in WalkDir::new(&self.path)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_dir())
            .sorted_by(|a, b| a.depth().cmp(&b.depth()).reverse())
        {
            std::fs::remove_dir(&d.path()).typ(SystemError::CGroup)?;
            trace!("Removed {:?}", d.path().as_os_str())
        }
        Ok(())
    }

    /// Returns the absolute path of this cgroup on the host file system
    pub fn path(&self) -> PathBuf {
        self.path.clone()
    }

    /// Freezes this cgroup
    pub fn freeze(&self) -> TypedResult<()> {
        std::fs::write(self.path.join(Self::FREEZE_FILE), "1").typ(SystemError::CGroup)
    }

    /// Unfreezes this cgroup
    pub fn unfreeze(&self) -> TypedResult<()> {
        std::fs::write(self.path.join(Self::FREEZE_FILE), "0").typ(SystemError::CGroup)
    }

    /// Returns a directory file descriptor to this cgroup
    pub fn get_fd(&self) -> TypedResult<File> {
        File::open(&self.path).typ(SystemError::CGroup)
    }
}

impl From<PathBuf> for CGroup {
    fn from(path: PathBuf) -> Self {
        Self { path }
    }
}
