use std::mem::forget;
use std::os::unix::prelude::{FromRawFd, IntoRawFd, OwnedFd};
use std::ptr::null_mut;
use std::sync::Mutex;

use a653rs::bindings::*;
use a653rs::prelude::{ProcessAttribute, SystemTime};
use a653rs_linux_core::cgroup;
use a653rs_linux_core::cgroup::CGroup;
use a653rs_linux_core::error::{
    ErrorLevel, LeveledResult, ResultExt, SystemError, TypedResult, TypedResultExt,
};
use a653rs_linux_core::fd::PidFd;
use a653rs_linux_core::file::TempFile;
use a653rs_linux_core::partition::PartitionConstants;
use anyhow::anyhow;
use memmap2::MmapOptions;
use nix::libc::{stack_t, SIGCHLD};
use nix::sched::CloneFlags;
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
#[derive(Debug, Clone)]
pub(crate) struct Process {
    id: i32,
    attr: ProcessAttribute,
    activated: TempFile<bool>,
    pid: TempFile<Pid>,
    periodic: bool,
}

impl Process {
    pub fn create(attr: ProcessAttribute) -> LeveledResult<ProcessId> {
        let name = attr
            .name
            .to_str()
            .lev_typ(SystemError::Panic, ErrorLevel::Partition)?
            .to_string();
        trace!("Create New Process: {name:?}");
        let stack_size: usize = attr
            .stack_size
            .try_into()
            .lev_typ(SystemError::Panic, ErrorLevel::Partition)?;
        let mut stack = MmapOptions::new()
            .stack()
            .len(stack_size)
            .map_anon()
            .lev_typ(SystemError::Panic, ErrorLevel::Partition)?;

        let periodic = attr.period != SystemTime::Infinite;
        let id = periodic as i32 + 1;

        let guard = SYNC
            .lock()
            .map_err(|e| anyhow!("{e:?}"))
            .lev_typ(SystemError::Panic, ErrorLevel::Partition)?;

        let proc_file = if periodic {
            PERIODIC_PROCESS.clone()
        } else {
            APERIODIC_PROCESS.clone()
        };

        if proc_file.read().lev(ErrorLevel::Partition)?.is_some() {
            return Err(anyhow!("Process type already exists. Periodic: {periodic}"))
                .lev_typ(SystemError::Panic, ErrorLevel::Partition);
        }

        // Files for dropping fd
        let mut fds = Vec::new();
        let activated = TempFile::create(format!("state_{name}")).lev(ErrorLevel::Partition)?;
        fds.push(unsafe { OwnedFd::from_raw_fd(activated.fd()) });
        activated.write(&false).lev(ErrorLevel::Partition)?;
        let pid = TempFile::create(format!("pid_{name}")).lev(ErrorLevel::Partition)?;
        fds.push(unsafe { OwnedFd::from_raw_fd(pid.fd()) });

        let process = Self {
            id,
            attr,
            activated,
            pid,
            periodic,
        };

        proc_file.write(&Some(process)).lev(ErrorLevel::Partition)?;

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

        trace!("Created process \"{name}\" with id: {id}");
        Ok(id as ProcessId)
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

    pub fn name(&self) -> LeveledResult<&str> {
        self.attr
            .name
            .to_str()
            .lev_typ(SystemError::Panic, ErrorLevel::Partition)
    }

    pub fn start(&self) -> LeveledResult<PidFd> {
        let name = self.name()?;
        trace!("Start Process \"{name}\"");
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

        let cg = self.cg().lev(ErrorLevel::Partition)?;
        cg.freeze()
            .typ(SystemError::CGroup)
            .lev(ErrorLevel::Partition)?;

        let stack = unsafe {
            STACKS[self.periodic as usize]
                .get()
                .expect("TODO: Do not expect here")
                .0
                .as_mut()
                .expect("TODO: Do not expect here")
        };

        //let stack_size = self.attr.stack_size as u64;
        safemem::write_bytes(stack, 0);
        let cbk = Box::new(move || {
            let cg = self.cg().unwrap();
            cg.mv(getpid()).unwrap();
            (self.attr.entry_point)();
            0
        });

        // Make extra sure that the process is in the cgroup
        let child = nix::sched::clone(cbk, stack, CloneFlags::empty(), Some(SIGCHLD))
            .lev_typ(SystemError::Panic, ErrorLevel::Partition)?;
        cg.mv(child).unwrap();

        self.pid.write(&child).lev(ErrorLevel::Partition)?;

        let pidfd = PidFd::try_from(child).lev(ErrorLevel::Partition)?;

        trace!("Started process \"{name}\" with pid: {child}");
        Ok(pidfd)
    }

    pub(crate) fn cg(&self) -> TypedResult<CGroup> {
        let cg_name = if self.periodic {
            PartitionConstants::PERIODIC_PROCESS_CGROUP
        } else {
            PartitionConstants::APERIODIC_PROCESS_CGROUP
        };

        let path = cgroup::mount_point().typ(SystemError::CGroup)?;
        let path = path.join(cg_name);

        CGroup::import_root(path).typ(SystemError::CGroup)
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
