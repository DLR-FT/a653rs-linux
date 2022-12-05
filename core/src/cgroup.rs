//! Implementation of the Linux *cgroup* facility
//!
//! This module provides an interface for the Linux cgroup facility.
//! Interfacing applications either create or import a cgroup, which
//! will then be used to build a tree to keep track of all following
//! sub-cgroups.
//!
//! This approach makes it possible to only manage a certain sub-tree
//! of cgroups, thereby saving resources. Alternatively, the root cgroup
//! may be imported, keeping track of all cgroups existing on the host system.
use std::fs;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Ok};
use nix::sys::statfs;
use nix::unistd::Pid;

/// A single cgroup inside our tree of managed cgroups
///
/// The tree is not represented by a traditional tree data structure,
/// as this is very complicated in Rust. Instead, the tree is "calculated"
/// by the path alone.
#[derive(Debug)]
pub struct CGroup {
    path: PathBuf,
}

impl CGroup {
    /// Creates a new cgroup as the root of a sub-tree
    ///
    /// path must be the path of an already existing cgroup
    pub fn new_root<P: AsRef<Path>>(path: P, name: &str) -> anyhow::Result<Self> {
        // Double-checking if path is cgroup does not hurt, as it is
        // better to potentially create a directory at a random location.
        if !is_cgroup(path.as_ref())? {
            bail!("{} is not a valid cgroup", path.as_ref().display());
        }

        let path = PathBuf::from(path.as_ref()).join(name);

        // There is no need to check if the path already exists, as create_dir()
        // will fail under this circumstance.
        fs::create_dir(&path)?;

        Self::import_root(&path)
    }

    /// Imports an already existing cgroup as the root of a sub-tree
    pub fn import_root<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let path = PathBuf::from(path.as_ref());

        if !is_cgroup(&path)? {
            bail!("{} is not a valid cgroup", path.display());
        }

        Ok(CGroup {
            path: path.to_path_buf(),
        })
    }

    /// Creates a sub-cgroup inside this one
    pub fn new(&self, name: &str) -> anyhow::Result<Self> {
        Self::new_root(&self.path, name)
    }

    /// Moves a process to this cgroup
    pub fn mv(&self, pid: Pid) -> anyhow::Result<()> {
        if !is_cgroup(&self.path)? {
            bail!("{} is not a valid cgroup", self.path.display());
        }

        fs::write(self.path.join("cgroup.procs"), pid.to_string())?;
        Ok(())
    }

    /// Returns all PIDs associated with this cgroup
    pub fn get_pids(&self) -> anyhow::Result<Vec<Pid>> {
        if !is_cgroup(&self.path)? {
            bail!("{} is not a valid cgroup", self.path.display());
        }

        let pids: Vec<Pid> = fs::read(self.path.join("cgroup.procs"))?
            .lines()
            .map(|line| Pid::from_raw(line.unwrap().parse().unwrap()))
            .collect();

        Ok(pids)
    }

    /// Checks whether this cgroup is populated
    pub fn populated(&self) -> anyhow::Result<bool> {
        if !is_cgroup(&self.path)? {
            bail!("{} is not a valid cgroup", self.path.display());
        }

        Ok(!fs::read(self.path.join("cgroup.procs"))?.is_empty())
    }

    /// Checks whether this cgroup is frozen
    pub fn frozen(&self) -> anyhow::Result<bool> {
        if !is_cgroup(&self.path)? {
            bail!("{} is not a valid cgroup", self.path.display());
        }

        // We need to check for the existance of cgroup.freeze, because
        // this file does not exist on the root cgroup.
        let path = self.path.join("cgroup.freeze");
        if !path.exists() {
            return Ok(false);
        }

        Ok(fs::read(&path)? == "1\n".as_bytes())
    }

    /// Freezes this cgroup (does nothing if already frozen)
    pub fn freeze(&self) -> anyhow::Result<()> {
        if !is_cgroup(&self.path)? {
            bail!("{} is not a valid cgroup", self.path.display());
        }

        // We need to check for the existance of cgroup.freeze, because
        // this file does not exist on the root cgroup.
        let path = self.path.join("cgroup.freeze");
        if !path.exists() {
            bail!("cannot freeze the root cgroup");
        }

        Ok(fs::write(path, "1")?)
    }

    /// Unfreezes this cgroup (does nothing if not frozen)
    pub fn unfreeze(&self) -> anyhow::Result<()> {
        if !is_cgroup(&self.path)? {
            bail!("{} is not a valid cgroup", self.path.display());
        }

        // We need to check for the existance of cgroup.freeze, because
        // this file does not exist on the root cgroup.
        let path = self.path.join("cgroup.freeze");
        if !path.exists() {
            bail!("cannot unfreeze the root cgroup");
        }

        Ok(fs::write(path, "0")?)
    }

    /// Kills all processes in this cgroup
    pub fn kill(&self) -> anyhow::Result<()> {
        if !is_cgroup(&self.path)? {
            bail!("{} is not a valid cgroup", self.path.display());
        }

        // We need to check for the existance of cgroup.kill, because
        // this file does not exist on the root cgroup.
        let path = self.path.join("cgroup.kill");
        if !path.exists() {
            bail!("cannot kill the root cgroup");
        }

        Ok(fs::write(path, "1")?)
    }

    /// Returns the path of this cgroup
    pub fn get_path(&self) -> PathBuf {
        self.path.clone()
    }

    /// Returns the path of the event file, which may be polled
    pub fn get_events_path(&self) -> PathBuf {
        self.path.join("cgroup.events")
    }

    /// Kills all processes and removes the current cgroup
    pub fn rm(&self) -> anyhow::Result<()> {
        if !is_cgroup(&self.path)? {
            bail!("{} is not a valid cgroup", self.path.display());
        }

        // Recursively delete all sub cgroups
        for entry in fs::read_dir(&self.path)? {
            let entry = entry?;
            let meta = entry.metadata()?;

            if meta.is_dir() {
                // Each sub cgroup is deleted by calling this function recursively
                let child = Self::import_root(&entry.path())?;
                child.rm()?;
            }
        }

        // Remove the actual cgroup
        self.kill()?;
        while self.populated()? {}
        fs::remove_dir(&self.path)?;

        Ok(())
    }

    // TODO: Implement functions to fetch the parents and children
}

/// Returns the first cgroup2 mount point found on the host system
pub fn mount_point() -> anyhow::Result<PathBuf> {
    // TODO: This is an awful old function, replace it!
    procfs::process::Process::myself()?
        .mountinfo()?
        .iter()
        .find(|m| m.fs_type.eq("cgroup2")) // TODO A process can have several cgroup mounts
        .ok_or_else(|| anyhow!("no cgroup2 mount found"))
        .map(|m| m.mount_point.clone())
}

/// Returns the path relative to the cgroup mount to
/// which cgroup this process belongs to
pub fn current_cgroup() -> anyhow::Result<PathBuf> {
    let path = procfs::process::Process::myself()?
        .cgroups()?
        .first()
        .ok_or(anyhow!("cannot obtain cgroup"))?
        .pathname
        .clone();
    let path = &path[1..path.len()]; // Remove the leading '/'

    Ok(PathBuf::from(path))
}

/// Checks if path is a valid cgroup by comparing the device id
fn is_cgroup(path: &Path) -> anyhow::Result<bool> {
    let st = statfs::statfs(path)?;
    Ok(st.filesystem_type() == statfs::CGROUP2_SUPER_MAGIC)
}

#[cfg(test)]
mod tests {
    // The tests must be run as root with --test-threads=1

    use std::{io, process};

    use super::*;

    #[test]
    fn new_root() {
        let path = get_path().join("cgroup_test");
        assert!(!path.exists()); // Ensure, that it does not already exist

        let cg = CGroup::new_root(get_path(), "cgroup_test").unwrap();
        assert!(path.exists() && path.is_dir());

        cg.rm().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn import_root() {
        let path = get_path().join("cgroup_test");
        assert!(!path.exists()); // Ensure, that it does not already exist
        fs::create_dir(&path).unwrap();

        let cg = CGroup::import_root(&path).unwrap();

        cg.rm().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn new() {
        let path_cg1 = get_path().join("cgroup_test");
        let path_cg2 = path_cg1.join("cgroup_test2");
        assert!(!path_cg1.exists()); // Ensure, that it does not already exist

        let cg1 = CGroup::new_root(get_path(), "cgroup_test").unwrap();
        assert!(path_cg1.exists() && path_cg1.is_dir());
        assert!(!path_cg2.exists());

        let _cg2 = cg1.new("cgroup_test2").unwrap();
        assert!(path_cg2.exists() && path_cg2.is_dir());

        cg1.rm().unwrap();

        assert!(!path_cg2.exists());
        assert!(!path_cg1.exists());
    }

    #[test]
    fn mv() {
        let mut proc = spawn_proc().unwrap();
        let pid = Pid::from_raw(proc.id() as i32);

        let cg1 = CGroup::new_root(get_path(), "cgroup_test").unwrap();
        let cg2 = cg1.new("cgroup_test2").unwrap();

        cg1.mv(pid).unwrap();
        cg2.mv(pid).unwrap();
        proc.kill().unwrap();

        cg1.rm().unwrap();
    }

    #[test]
    fn get_pids() {
        let mut proc = spawn_proc().unwrap();
        let pid = Pid::from_raw(proc.id() as i32);

        let cg1 = CGroup::new_root(get_path(), "cgroup_test").unwrap();
        let cg2 = cg1.new("cgroup_test2").unwrap();

        assert!(cg1.get_pids().unwrap().is_empty());
        assert!(cg2.get_pids().unwrap().is_empty());

        cg1.mv(pid).unwrap();
        let pids = cg1.get_pids().unwrap();
        assert!(!pids.is_empty());
        assert!(cg2.get_pids().unwrap().is_empty());
        assert_eq!(pids.len(), 1);
        assert_eq!(pids[0], pid);

        cg2.mv(pid).unwrap();
        let pids = cg2.get_pids().unwrap();
        assert!(!pids.is_empty());
        assert!(cg1.get_pids().unwrap().is_empty());
        assert_eq!(pids.len(), 1);
        assert_eq!(pids[0], pid);

        proc.kill().unwrap();

        cg1.rm().unwrap();
    }

    #[test]
    fn populated() {
        let mut proc = spawn_proc().unwrap();
        let pid = Pid::from_raw(proc.id() as i32);
        let cg = CGroup::new_root(get_path(), "cgroup_test").unwrap();

        assert!(!cg.populated().unwrap());
        assert_eq!(cg.populated().unwrap(), cg.get_pids().unwrap().len() > 0);

        cg.mv(pid).unwrap();
        assert!(cg.populated().unwrap());
        assert_eq!(cg.populated().unwrap(), cg.get_pids().unwrap().len() > 0);

        proc.kill().unwrap();

        cg.rm().unwrap();
    }

    #[test]
    fn frozen() {
        let mut proc = spawn_proc().unwrap();
        let pid = Pid::from_raw(proc.id() as i32);
        let cg = CGroup::new_root(get_path(), "cgroup_test").unwrap();

        // Freeze an empty cgroup
        assert!(!cg.frozen().unwrap());
        cg.freeze().unwrap();
        assert!(cg.frozen().unwrap());

        // Unfreeze the empty cgroup
        cg.unfreeze().unwrap();
        assert!(!cg.frozen().unwrap());

        // Do the same with a non-empty cgroup
        cg.mv(pid).unwrap();
        cg.freeze().unwrap();
        assert!(cg.frozen().unwrap());
        cg.unfreeze().unwrap();
        assert!(!cg.frozen().unwrap());

        proc.kill().unwrap();

        cg.rm().unwrap();
    }

    #[test]
    fn kill() {
        let proc = spawn_proc().unwrap();
        let pid = Pid::from_raw(proc.id() as i32);
        let cg = CGroup::new_root(get_path(), "cgroup_test").unwrap();

        // Kill an empty cgroup
        cg.kill().unwrap();

        // Do the same with a non-empty cgroup
        cg.mv(pid).unwrap();
        assert!(cg.populated().unwrap());
        cg.kill().unwrap();

        cg.rm().unwrap();

        // TODO: Check if the previous PID still exists (although unstable because the OS may re-assign)
    }

    #[test]
    fn is_cgroup() {
        assert!(super::is_cgroup(&get_path()).unwrap());
        assert!(!super::is_cgroup(Path::new("/tmp")).unwrap());
    }

    /// Spawns a child process of yes(1)
    fn spawn_proc() -> io::Result<process::Child> {
        process::Command::new("yes")
            .stdout(process::Stdio::null())
            .spawn()
    }

    /// Returns the path of the current cgorup inside the mount
    fn get_path() -> PathBuf {
        super::mount_point()
            .unwrap()
            .join(super::current_cgroup().unwrap())
    }
}
