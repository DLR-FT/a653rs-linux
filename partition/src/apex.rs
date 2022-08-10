// TODO remove this
#![allow(unused_variables)]

use apex_hal::bindings::*;

use crate::partition::ApexLinuxPartition;
use crate::process::Process as LinuxProcess;
use crate::*;

impl ApexPartition for ApexLinuxPartition {
    fn get_partition_status<L: Locked>() -> ApexPartitionStatus {
        ApexPartitionStatus {
            period: PART_PERIOD.as_nanos() as i64,
            duration: PART_DURATION.as_nanos() as i64,
            identifier: *PART_IDENTIFIER,
            lock_level: 0,
            operating_mode: PART_OPERATION_MODE.read().unwrap(),
            start_condition: *PART_START_CONDITION,
            num_assigned_cores: 1,
        }
    }

    fn set_partition_mode<L: Locked>(operating_mode: OperatingMode) -> Result<(), ErrorReturnCode> {
        // TODO: Handle transitions
        // TODO: Do not unwrap error
        PART_OPERATION_MODE.write(&operating_mode).unwrap();

        if operating_mode == OperatingMode::Normal {
            // If we transition into Normal Mode, run the scheduler and never return
            scheduler::scheduler();
        }
        Ok(())
    }
}

impl ApexProcess for ApexLinuxPartition {
    fn create_process<L: Locked>(
        attributes: &ApexProcessAttribute,
    ) -> Result<ProcessId, ErrorReturnCode> {
        // TODO do not unwrap both
        // Check current State (only allowed in warm and cold start)
        let attr = (*attributes).try_into().unwrap();
        Ok(LinuxProcess::create(attr).unwrap())
    }

    fn start<L: Locked>(process_id: ProcessId) -> Result<(), ErrorReturnCode> {
        // This more like a reset function for dormant processes
        // TODO check for correct partition operating state
        let file = match process_id {
            0 => *APERIODIC_PROCESS,
            1 => *PERIODIC_PROCESS,
            _ => todo!("Return error"),
        };
        // TODO do not unwrap
        let proc = file.read().unwrap().unwrap();
        proc.start().unwrap();

        Ok(())
    }
}

impl ApexTime for ApexLinuxPartition {
    fn periodic_wait<L: Locked>() -> Result<(), ErrorReturnCode> {
        // TODO do not unwrap() (Maybe raise an error?);
        let proc = LinuxProcess::get_self().unwrap();
        if !proc.periodic() {
            return Err(ErrorReturnCode::InvalidMode);
        }
        todo!("Handle periodic wait")
    }

    fn get_time<L: Locked>() -> ApexSystemTime {
        SYSTEM_TIME
            .elapsed()
            .as_nanos()
            .clamp(0, ApexSystemTime::MAX as u128) as ApexSystemTime
    }
}
