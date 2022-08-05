// TODO remove this
#![allow(dead_code)]

use std::process::exit;

use anyhow::Result;
use apex_hal::bindings::ProcessState;
use apex_hal::prelude::{Priority, ProcessAttribute, SystemTime};
use bytemuck::{Pod, Zeroable};
use linux_apex_core::cgroup::{CGroup, DomainCGroup};
use linux_apex_core::file::TempFile;
use memmap2::{Mmap, MmapMut, MmapOptions};
use nix::sched::CloneFlags;
use nix::sys::mman::{mmap, munmap, MapFlags, ProtFlags};
use nix::unistd::{getpid, Pid};

use crate::PROCESSES;

#[repr(C)]
//#[derive(Pod, Zeroable, Clone, Copy)]
pub struct Process {
    name: String,
    stack: MmapMut,
    //stack_size: usize,
    cg: DomainCGroup,
    attr: ProcessAttribute,
    deadline: TempFile<SystemTime>, // Key 0
    priority: TempFile<Priority>,   // Key 1
    state: TempFile<ProcessState>,  // Key 2
    pid: TempFile<Pid>,             // Key 3
}

impl Process {
    pub fn new(attr: ProcessAttribute) -> Result<Self> {
        // munmap if Err
        let name = attr.name.to_str()?.to_string();
        debug!("Create New Process: {name:?}");
        let stack_size = attr.stack_size.try_into()?;
        //let stack_ptr = unsafe {
        //    AtomicPtr::new(mmap(
        //        null_mut(),
        //        stack_size,
        //        ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
        //        MapFlags::MAP_PRIVATE
        //            | MapFlags::MAP_ANONYMOUS
        //            | MapFlags::MAP_GROWSDOWN
        //            | MapFlags::MAP_STACK,
        //        -1,
        //        0,
        //    )?)
        //};
        let stack = MmapOptions::new().stack().len(stack_size).map_anon()?;

        let cg = DomainCGroup::new(CGroup::mount_point()?, &name)?;
        let deadline = TempFile::new(&format!("deadline_{name}"))?;
        let priority = TempFile::new(&format!("priority_{name}"))?;
        let state = TempFile::new(&format!("state_{name}"))?;
        let pid = TempFile::new(&format!("pid_{name}"))?;

        Ok(Self {
            name,
            stack,
            //stack_size,
            cg,
            attr,
            deadline,
            priority,
            state,
            pid,
        })
    }

    pub fn freeze(&mut self) -> Result<()> {
        self.cg.freeze()
    }

    pub fn unfreeze(&mut self) -> Result<()> {
        self.cg.unfreeze()
    }

    pub fn init(&mut self) -> Result<()> {
        self.cg.kill_all().unwrap();

        self.freeze().unwrap();

        self.priority.write(self.attr.base_priority)?;

        // This makes sure that lazy_static triggers at least once before a process was created
        PROCESSES.get_fd();

        safemem::write_bytes(&mut self.stack, 0);
        let cbk = Box::new(|| {
            self.cg.add_process(getpid()).unwrap();
            (self.attr.entry_point)();
            exit(0);
        });
        let child = nix::sched::clone(cbk, &mut self.stack, CloneFlags::empty(), None)?;

        //let child = match unsafe {
        //     Clone3::default()
        //     .flag_into_cgroup(&self.cg.get_fd()?)
        //    .flag_vm(stack)
        //    .call().unwrap()
        //} {
        //    0 => {
        //        (self.attr.entry_point)();
        //        exit(0);
        //    },
        //    child => child,
        //};

        self.pid.write(child)?;
        //self.pid.write(Pid::from_raw(child))?;
        debug!("Created process \"{}\" with id: {}", self.name, child);
        Ok(())
    }
}

//impl Drop for Process {
//    fn drop(&mut self) {
//        unsafe {
//            //// TODO check for other potential memory leaks in other locations
//            //munmap(self.stack_ptr.swap(null_mut(), Ordering::Relaxed), self.attr.stack_size.try_into().unwrap()).unwrap();
//            //self.attr.stack_size = 0;
//        }
//    }
//}
