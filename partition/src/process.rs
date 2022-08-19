use core::panic;
use std::mem::forget;
use std::os::unix::prelude::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::ptr::null_mut;
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use apex_hal::bindings::*;
use apex_hal::prelude::{ProcessAttribute, SystemTime};
use linux_apex_core::cgroup::CGroup;
use linux_apex_core::error::{SystemError, TypedResult};
use linux_apex_core::fd::PidFd;
use linux_apex_core::file::TempFile;
use linux_apex_core::partition::{APERIODIC_PROCESS_CGROUP, PERIODIC_PROCESS_CGROUP};
use memmap2::MmapOptions;
use nix::libc::{stack_t, SIGCHLD};
use nix::sched::CloneFlags;
use nix::sys::resource::{setrlimit, Resource};
use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, Signal};
use nix::sys::signalfd::SigSet;
use nix::unistd::{getpid, Pid};
use once_cell::sync::{Lazy, OnceCell};

use crate::partition::ApexLinuxPartition;
use crate::{APERIODIC_PROCESS, PERIODIC_PROCESS, SIGNAL_STACK};

//use crate::{APERIODIC_PROCESS, PERIODIC_PROCESS};

#[derive(Debug, Clone, Copy)]
struct StackPtr(*mut [u8]);

unsafe impl Sync for StackPtr {}
unsafe impl Send for StackPtr {}

static STACKS: [OnceCell<StackPtr>; 2] = [OnceCell::new(), OnceCell::new()];

static SYNC: Lazy<Mutex<u8>> = Lazy::new(|| Mutex::new(Default::default()));

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct Process {
    id: i32,
    attr: ProcessAttribute,
    activated: TempFile<bool>,
    pid: TempFile<Pid>,
    periodic: bool,
}

impl Process {
    pub fn create(attr: ProcessAttribute) -> Result<ProcessId> {
        let name = attr.name.to_str()?.to_string();
        debug!("Create New Process: {name:?}");
        let stack_size: usize = attr.stack_size.try_into()?;
        let mut stack = MmapOptions::new().stack().len(stack_size).map_anon()?;

        let periodic = attr.period != SystemTime::Infinite;
        let id = periodic as i32 + 1;

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
        let mut fds = Vec::new();
        let activated = TempFile::new(&format!("state_{name}"))?;
        fds.push(unsafe { OwnedFd::from_raw_fd(activated.fd()) });
        activated.write(&false)?;
        let pid = TempFile::new(&format!("pid_{name}"))?;
        fds.push(unsafe { OwnedFd::from_raw_fd(pid.fd()) });

        let process = Self {
            id,
            attr,
            activated,
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
        for f in fds {
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

    pub fn start(&self) -> Result<PidFd> {
        unsafe {
            let stack = stack_t {
                ss_sp: SIGNAL_STACK.as_ptr() as *mut nix::libc::c_void,
                ss_flags: 0,
                ss_size: nix::libc::SIGSTKSZ,
            };
            nix::libc::sigaltstack(&stack, null_mut());

            let report_sigsegv_action = SigAction::new(
                SigHandler::Handler(handle_sigsegv),
                SaFlags::SA_ONSTACK,
                SigSet::empty(),
            );
            sigaction(Signal::SIGSEGV, &report_sigsegv_action).unwrap();

            let report_sigfpe_action = SigAction::new(
                SigHandler::Handler(handle_sigfpe),
                SaFlags::SA_ONSTACK,
                SigSet::empty(),
            );
            sigaction(Signal::SIGFPE, &report_sigfpe_action).unwrap();
        }

        let name = self.name()?;

        let mut cg = self.cg()?;
        cg.freeze()?;

        let stack = unsafe {
            STACKS[self.periodic as usize]
                .get()
                .expect("TODO: Do not expect here")
                .0
                .as_mut()
                .expect("TODO: Do not expect here")
        };

        let stack_size = self.attr.stack_size as u64;
        safemem::write_bytes(stack, 0);
        let cbk = Box::new(move || {
            // TODO
            //setrlimit(Resource::RLIMIT_STACK, stack_size - 2000, stack_size - 2000).unwrap();

            let mut cg = self.cg().unwrap();
            cg.add_process(getpid()).unwrap();
            (self.attr.entry_point)();
            0
        });

        // Make extra sure that the process is in the cgroup
        let child = nix::sched::clone(cbk, stack, CloneFlags::empty(), Some(SIGCHLD as i32))?;
        cg.add_process(child).unwrap();

        self.pid.write(&child)?;

        let pidfd = PidFd::try_from(child)?;

        trace!("Started process \"{name}\" with pid: {child}");
        Ok(pidfd)
    }

    pub(crate) fn cg(&self) -> TypedResult<CGroup> {
        let cg_name = if self.periodic {
            PERIODIC_PROCESS_CGROUP
        } else {
            APERIODIC_PROCESS_CGROUP
        };
        CGroup::new(CGroup::mount_point()?, cg_name)
    }

    pub fn periodic(&self) -> bool {
        self.periodic
    }
}

extern "C" fn handle_sigsegv(_: i32) {
    ApexLinuxPartition::raise_system_error(SystemError::Segmentation);
}

extern "C" fn handle_sigfpe(_: i32) {
    ApexLinuxPartition::raise_system_error(SystemError::FloatingPoint);
}
