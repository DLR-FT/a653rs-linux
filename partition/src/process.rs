// TODO remove this
#![allow(dead_code, mutable_transmutes)]

use std::cell::UnsafeCell;
use std::fs::File;
use std::mem::{forget, transmute};
use std::os::unix::prelude::{FromRawFd, IntoRawFd};
use std::process::exit;
use std::ptr::slice_from_raw_parts_mut;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once};

use anyhow::{anyhow, Result};
use apex_hal::bindings::*;
use apex_hal::prelude::{ProcessAttribute, SystemTime};
use linux_apex_core::cgroup::CGroup;
use linux_apex_core::file::{get_fd, TempFile};
use linux_apex_core::shmem::TypedMmapMut;
use memmap2::MmapOptions;
use nix::sched::CloneFlags;
use nix::unistd::{getpid, Pid};
use once_cell::sync::{Lazy, OnceCell};

use crate::{APERIODIC_PROCESS, PERIODIC_PROCESS};

//use crate::{APERIODIC_PROCESS, PERIODIC_PROCESS};

#[derive(Debug, Clone, Copy)]
struct StackPtr(*mut [u8]);

unsafe impl Sync for StackPtr {}
unsafe impl Send for StackPtr {}

static STACKS: [OnceCell<StackPtr>; 2] = [OnceCell::new(), OnceCell::new()];

static SYNC: Lazy<Mutex<u8>> = Lazy::new(|| Mutex::new(Default::default()));

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Process {
    id: i32,
    attr: ProcessAttribute,
    deadline: TempFile<SystemTime>,
    state: TempFile<ProcessState>,
    pid: TempFile<Pid>,
    periodic: bool,
}

impl Process {
    pub fn create(attr: ProcessAttribute) -> Result<ProcessId> {
        let name = attr.name.to_str()?.to_string();
        debug!("Create New Process: {name:?}");
        let stack_size = attr.stack_size.try_into()?;
        let mut stack = MmapOptions::new().stack().len(stack_size).map_anon()?;

        let periodic = attr.period != SystemTime::Infinite;
        let id = periodic as i32 + 1;

        // Create Cgroup
        let mut cg = CGroup::new(CGroup::mount_point()?, &name)?;
        cg.freeze().unwrap();

        warn!("{}", stack.len());
        let guard = SYNC.lock().map_err(|e| anyhow!("{e:?}"))?;

        let proc_file = if periodic {
            *PERIODIC_PROCESS
        } else {
            *APERIODIC_PROCESS
        };
        if proc_file.read()?.is_some() {
            return Err(anyhow!("Process type already exists. Periodic: {periodic}"));
        }

        // Files for dropping fd
        let mut files = Vec::new();
        let deadline = TempFile::new(&format!("deadline_{name}"))?;
        files.push(unsafe { File::from_raw_fd(deadline.fd()) });
        let state = TempFile::new(&format!("state_{name}"))?;
        files.push(unsafe { File::from_raw_fd(state.fd()) });
        let pid = TempFile::new(&format!("pid_{name}"))?;
        files.push(unsafe { File::from_raw_fd(pid.fd()) });

        let process = Self {
            id,
            attr,
            deadline,
            state,
            pid,
            periodic,
        };

        proc_file.write(&Some(process))?;

        // We can unwrap because it was already checked that the cell is empty
        STACKS[periodic as usize]
            .set(StackPtr(stack.as_mut()))
            .unwrap();

        drop(guard);

        // dissolve files into fds
        for f in files {
            f.into_raw_fd();
        }
        // forget stack ptr so we do not call munmap
        forget(stack);

        debug!("Created process \"{name}\" with id: {id}");
        Ok(id)
    }

    pub(crate) fn get_self() -> Option<Self> {
        if let Ok(Some(p)) = APERIODIC_PROCESS.read() {
            if let Ok(id) = p.pid.read() {
                if id == nix::unistd::getpid() {
                    return Some(p);
                }
            }
        }

        if let Ok(Some(p)) = PERIODIC_PROCESS.read() {
            if let Ok(id) = p.pid.read() {
                if id == nix::unistd::getpid() {
                    return Some(p);
                }
            }
        }

        None
    }

    pub fn name(&self) -> Result<&str> {
        Ok(self.attr.name.to_str()?)
    }

    pub fn attr(&self) -> &ProcessAttribute {
        &self.attr
    }

    pub fn start(&self) -> Result<()> {
        let name = self.name()?;
        let mut cg = CGroup::new(CGroup::mount_point()?, name)?;

        cg.freeze().unwrap();
        cg.kill_all_wait()?;

        let stack = unsafe {
            STACKS[self.periodic as usize]
                .get()
                .expect("TODO: Do not expect here")
                .0
                .as_mut()
                .expect("TODO: Do not expect here")
        };

        safemem::write_bytes(stack, 0);
        let cbk = Box::new(|| {
            cg.add_process(getpid()).unwrap();
            (self.attr.entry_point)();
            exit(0);
        });
        let child = nix::sched::clone(cbk, stack, CloneFlags::empty(), None)?;
        self.pid.write(&child)?;

        trace!("Started process \"{name}\" with pid: {child}");
        Ok(())
    }

    fn cg(&self) -> Result<CGroup> {
        Ok(CGroup::from(CGroup::mount_point()?.join(self.name()?)))
    }

    pub fn periodic(&self) -> bool {
        self.periodic
    }

    pub fn freeze(&mut self) -> Result<()> {
        self.cg()?.freeze()
    }

    pub fn unfreeze(&mut self) -> Result<()> {
        self.cg()?.unfreeze()
    }
}
