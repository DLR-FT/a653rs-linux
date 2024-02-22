use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::Builder;

use a653rs::bindings::*;
use a653rs::prelude::{ProcessAttribute, SystemTime};
use anyhow::anyhow;
use nix::unistd::{gettid, Pid};

use a653rs_linux_core::cgroup;
use a653rs_linux_core::cgroup::CGroup;
use a653rs_linux_core::error::{
    ErrorLevel, LeveledResult, ResultExt, SystemError, TypedResult, TypedResultExt,
};
use a653rs_linux_core::partition::PartitionConstants;

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
            let id = p.pid.load(Ordering::SeqCst);
            if id == nix::unistd::gettid().as_raw() {
                return Some(p.clone());
            }
        }

        if let Some(p) = PERIODIC_PROCESS.get() {
            let id = p.pid.load(Ordering::SeqCst);
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

        let entry = self.attr.entry_point;

        // A mutex required for freezing the thread right before execution of `entry`.
        let sync = Arc::new(Mutex::new(()));
        let sync2 = Arc::clone(&sync);
        // A channel for the thread to send its thread id.
        let (pid_tx, pid_rx) = oneshot::channel();

        // Before spawning the thread, lock the `sync` mutex so the thread cannot
        // execute `entry` yet.
        let lock = sync.lock().unwrap();
        let _thread = Builder::new()
            .name(name.to_string())
            .stack_size(self.stack_size)
            .spawn(move || {
                pid_tx.send(gettid().as_raw()).unwrap();

                // We want this thread to be frozen right here before the entry function gets
                // executed. To do that, we wait for the `sync` mutex to unlock. During the wait
                // period this thread is then moved to the frozen cgroup.
                drop(sync2.lock().unwrap());
                (entry)();
            })
            .lev_typ(SystemError::Panic, ErrorLevel::Partition)?;
        // Receive thread id and store it
        let pid_raw = pid_rx.recv().unwrap();
        self.pid.store(pid_raw, Ordering::SeqCst);
        let pid = Pid::from_raw(pid_raw);
        // Freeze thread by moving it to the cgroup
        cg.mv_thread(pid).unwrap();
        // Now unlock the `sync` mutex, so the thread can continue execution when the
        // cgroup is unfrozen.
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
