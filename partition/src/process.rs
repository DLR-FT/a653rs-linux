// TODO remove this
#![allow(dead_code)]

use std::fs::File;
use std::mem::forget;
use std::os::unix::prelude::{FromRawFd, IntoRawFd};
use std::process::exit;
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use apex_hal::bindings::ProcessState;
use apex_hal::prelude::{Priority, ProcessAttribute, ProcessId, SystemTime};
use linux_apex_core::cgroup::CGroup;
use linux_apex_core::file::TempFile;
use linux_apex_core::shmem::TypedMmapMut;
use memmap2::MmapOptions;
use nix::sched::CloneFlags;
use nix::unistd::{getpid, Pid};

use crate::{ProcessesType, PROCESSES};

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Process {
    // This pointer is only valid inside the main process of a partition
    stack: &'static [u8],
    attr: ProcessAttribute,
    deadline: TempFile<SystemTime>,
    priority: TempFile<Priority>,
    state: TempFile<ProcessState>,
    pid: TempFile<Pid>,
}

lazy_static! {
    // Sync mutex
    static ref SYNC: Mutex<u8> = Mutex::new(0);
}

impl Process {
    pub fn create(attr: ProcessAttribute) -> Result<ProcessId> {
        let name = attr.name.to_str()?.to_string();
        debug!("Create New Process: {name:?}");
        let stack_size = attr.stack_size.try_into()?;
        let stack = MmapOptions::new().stack().len(stack_size).map_anon()?;

        // Create Cgroup
        CGroup::new(CGroup::mount_point()?, &name)?;

        // Files for dropping fd
        let mut files = Vec::new();

        let deadline = TempFile::new(&format!("deadline_{name}"))?;
        files.push(unsafe { File::from_raw_fd(deadline.fd()) });
        let priority = TempFile::new(&format!("priority_{name}"))?;
        files.push(unsafe { File::from_raw_fd(priority.fd()) });
        let state = TempFile::new(&format!("state_{name}"))?;
        files.push(unsafe { File::from_raw_fd(state.fd()) });
        let pid = TempFile::new(&format!("pid_{name}"))?;
        files.push(unsafe { File::from_raw_fd(pid.fd()) });

        let process = Self {
            // TODO this is disgusting
            stack: unsafe { (stack.as_ref() as *const [u8]).as_ref::<'static>().unwrap() },
            attr,
            deadline,
            priority,
            state,
            pid,
        };

        let guard = SYNC.lock().map_err(|e| anyhow!("{e:?}"))?;
        let mut procs: TypedMmapMut<ProcessesType> =
            (&PROCESSES as &TempFile<ProcessesType>).try_into()?;
        //Get Index of process in array
        let process_id = procs.as_ref().len().try_into()?;
        //Insert into array
        procs.as_mut().try_push(Some(process));
        drop(guard);

        // dissolve files into fds
        for f in files {
            f.into_raw_fd();
        }
        // dissolve stack ptr
        forget(stack);

        Ok(process_id)
    }

    pub fn name(&self) -> Result<&str> {
        Ok(self.attr.name.to_str()?)
    }

    fn cg(&self) -> Result<CGroup> {
        Ok(CGroup::from(CGroup::mount_point()?.join(self.name()?)))
    }

    pub fn freeze(&mut self) -> Result<()> {
        self.cg()?.freeze()
    }

    pub fn unfreeze(&mut self) -> Result<()> {
        self.cg()?.unfreeze()
    }

    pub fn start(&mut self) -> Result<()> {
        let mut cg = self.cg()?;
        cg.kill_all().unwrap();

        self.freeze().unwrap();

        self.priority.write(&self.attr.base_priority)?;

        // TODO this is disgusting (but is somehow keeps information on slice length)
        let stack = unsafe { (self.stack as *const _ as *mut [u8]).as_mut() }.unwrap();

        safemem::write_bytes(stack, 0);
        let cbk = Box::new(|| {
            cg.add_process(getpid()).unwrap();
            (self.attr.entry_point)();
            exit(0);
        });
        let child = nix::sched::clone(cbk, stack, CloneFlags::empty(), None)?;

        self.pid.write(&child)?;

        debug!("Created process \"{}\" with id: {}", self.name()?, child);
        Ok(())
    }
}
