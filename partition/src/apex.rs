// TODO remove this
#![allow(unused_variables)]
use std::process::exit;

use apex_hal::bindings::*;
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;

use crate::partition::Partition;
use crate::{scheduler, PARTITION_STATE, SYSTEM_TIME};

impl ApexPartition for Partition {
    fn get_partition_status<L: Locked>() -> ApexPartitionStatus {
        todo!()
    }

    fn set_partition_mode<L: Locked>(operating_mode: OperatingMode) -> Result<(), ErrorReturnCode> {
        // TODO: Handle transitions
        // TODO: Max error
        PARTITION_STATE.write(operating_mode).unwrap();

        if operating_mode == OperatingMode::Normal {
            // If we transition into Normal Mode, run the scheduler and never return
            scheduler::scheduler();
        }
        Ok(())
    }
}

impl ApexProcess for Partition {
    fn create_process<L: Locked>(
        attributes: &ApexProcessAttribute,
    ) -> Result<ProcessId, ErrorReturnCode> {
        todo!()
    }

    fn set_priority<L: Locked>(
        process_id: ProcessId,
        priority: Priority,
    ) -> Result<(), ErrorReturnCode> {
        todo!()
    }

    fn suspend_self<L: Locked>(time_out: ApexSystemTime) -> Result<(), ErrorReturnCode> {
        todo!()
    }

    fn suspend<L: Locked>(process_id: ProcessId) -> Result<(), ErrorReturnCode> {
        todo!()
    }

    fn resume<L: Locked>(process_id: ProcessId) -> Result<(), ErrorReturnCode> {
        todo!()
    }

    fn stop_self<L: Locked>() {
        // TODO Root process needs to notice this somehow
        exit(0)
    }

    fn stop<L: Locked>(process_id: ProcessId) -> Result<(), ErrorReturnCode> {
        // TODO Root process needs to notice this somehow
        // What to do if raise fails?
        // Max error to NO_ACTION if target process is in DORMANT State
        signal::kill(Pid::from_raw(process_id), Signal::SIGKILL)
            .map_err(|_e| ErrorReturnCode::InvalidParam)
    }

    fn start<L: Locked>(process_id: ProcessId) -> Result<(), ErrorReturnCode> {
        // This more like a reset function for dormant processes
        todo!()
    }

    fn delayed_start<L: Locked>(
        process_id: ProcessId,
        delay_time: ApexSystemTime,
    ) -> Result<(), ErrorReturnCode> {
        todo!()
    }

    fn lock_preemption<L: Locked>() -> Result<LockLevel, ErrorReturnCode> {
        todo!()
    }

    fn unlock_preemption<L: Locked>() -> Result<LockLevel, ErrorReturnCode> {
        todo!()
    }

    fn get_my_id<L: Locked>() -> Result<ProcessId, ErrorReturnCode> {
        Ok(nix::unistd::getpid().as_raw())
    }

    fn get_process_id<L: Locked>(process_name: ProcessName) -> Result<ProcessId, ErrorReturnCode> {
        todo!()
    }

    fn get_process_status<L: Locked>(
        process_id: ProcessId,
    ) -> Result<ApexProcessStatus, ErrorReturnCode> {
        todo!()
    }

    fn initialize_process_core_affinity<L: Locked>(
        process_id: ProcessId,
        processor_core_id: ProcessorCoreId,
    ) -> Result<(), ErrorReturnCode> {
        todo!()
    }

    fn get_my_processor_core_id<L: Locked>() -> ProcessorCoreId {
        todo!()
    }

    fn get_my_index<L: Locked>() -> Result<ProcessIndex, ErrorReturnCode> {
        todo!()
    }
}

impl ApexTime for Partition {
    fn timed_wait<L: Locked>(delay_time: ApexSystemTime) {
        todo!()
    }

    fn periodic_wait<L: Locked>() -> Result<(), ErrorReturnCode> {
        todo!()
    }

    fn get_time<L: Locked>() -> ApexSystemTime {
        SYSTEM_TIME
            .elapsed()
            .as_nanos()
            .clamp(0, ApexSystemTime::MAX as u128) as ApexSystemTime
    }

    fn replenish<L: Locked>(budget_time: ApexSystemTime) -> Result<(), ErrorReturnCode> {
        todo!()
    }
}
