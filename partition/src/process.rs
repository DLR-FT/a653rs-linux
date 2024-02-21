use std::sync::atomic::{AtomicBool, AtomicI32};
use std::sync::{Arc, Barrier, Mutex};
use std::thread::Builder;

use a653rs::bindings::*;
use a653rs::prelude::{ProcessAttribute, SystemTime};
use a653rs_linux_core::cgroup;
use a653rs_linux_core::cgroup::CGroup;
use a653rs_linux_core::error::{
    ErrorLevel, LeveledResult, ResultExt, SystemError, TypedResult, TypedResultExt,
};
use a653rs_linux_core::partition::PartitionConstants;
use anyhow::anyhow;
use nix::unistd::{gettid, Pid};

use crate::{APERIODIC_PROCESS, PERIODIC_PROCESS};

#[repr(C)]
#[derive(Debug, Clone)]
pub(crate) struct Process {
    id: i32,
    attr: ProcessAttribute,
    activated: Arc<AtomicBool>,
    pid: Arc<AtomicI32>,
    periodic: bool,
    stack_size: usize,
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

        let periodic = attr.period != SystemTime::Infinite;
        let id = periodic as i32 + 1;

        let proc_file = if periodic {
            &PERIODIC_PROCESS
        } else {
            &APERIODIC_PROCESS
        };

        let res = proc_file.try_insert(Arc::new(Self {
            id,
            attr,
            activated: Arc::new(AtomicBool::new(false)),
            pid: Arc::new(AtomicI32::new(0)),
            periodic,
            stack_size,
        }));
        if res.is_ok() {
            trace!("Created process \"{name}\" with id: {id}");
            Ok(id as ProcessId)
        } else {
            Err(anyhow!("Process type already exists. Periodic: {periodic}"))
                .lev_typ(SystemError::Panic, ErrorLevel::Partition)
        }
    }

    pub(crate) fn get_self() -> Option<Arc<Self>> {
        if let Some(p) = APERIODIC_PROCESS.get() {
            let id = p.pid.load(std::sync::atomic::Ordering::Relaxed);
            if id == nix::unistd::gettid().as_raw() {
                return Some(p.clone());
            }
        }

        if let Some(p) = PERIODIC_PROCESS.get() {
            let id = p.pid.load(std::sync::atomic::Ordering::Relaxed);
            if id == nix::unistd::gettid().as_raw() {
                return Some(p.clone());
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

    pub fn start(&self) -> LeveledResult<()> {
        let name = self.name()?;
        trace!("Start Process \"{name}\"");

        let cg = self.cg().lev(ErrorLevel::Partition)?;
        cg.freeze()
            .typ(SystemError::CGroup)
            .lev(ErrorLevel::Partition)?;

        let sync = Arc::new((Barrier::new(2), Mutex::new(())));
        let s = sync.clone();
        let entry = self.attr.entry_point;
        let proc = self.pid.clone();
        let _thread = Builder::new()
            .name(name.to_string())
            .stack_size(self.stack_size)
            .spawn(move || {
                proc.store(gettid().as_raw(), std::sync::atomic::Ordering::Relaxed);
                s.0.wait();
                drop(s.1.lock().unwrap());
                (entry)();
            })
            .lev_typ(SystemError::Panic, ErrorLevel::Partition)?;
        let lock = sync.1.lock().unwrap();
        sync.0.wait();
        let pid = Pid::from_raw(self.pid.load(std::sync::atomic::Ordering::Relaxed));
        cg.mv_thread(pid).unwrap();
        drop(lock);

        Ok(())
    }

    pub(crate) fn cg(&self) -> TypedResult<CGroup> {
        let cg_name = if self.periodic {
            PartitionConstants::PERIODIC_PROCESS_CGROUP
        } else {
            PartitionConstants::APERIODIC_PROCESS_CGROUP
        };

        let path = cgroup::mount_point().typ(SystemError::CGroup)?;
        let path = path
            .join(PartitionConstants::PROCESSES_CGROUP)
            .join(cg_name);

        CGroup::import_root(path).typ(SystemError::CGroup)
    }

    pub fn periodic(&self) -> bool {
        self.periodic
    }
}
