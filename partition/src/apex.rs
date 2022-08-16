// TODO remove this
#![allow(unused_variables)]

use std::process::exit;

use apex_hal::bindings::*;

use crate::partition::ApexLinuxPartition;
use crate::process::Process as LinuxProcess;
use crate::*;

impl ApexPartition for ApexLinuxPartition {
    fn get_partition_status<L: Locked>() -> ApexPartitionStatus {
        let operating_mode = std::env::var(MODE_ENV)
            .ok()
            .and_then(|m| m.parse::<u32>().ok())
            .and_then(|m| OperatingMode::try_from(m).ok())
            .unwrap_or(OperatingMode::Normal);

        ApexPartitionStatus {
            period: PART_PERIOD.as_nanos() as i64,
            duration: PART_DURATION.as_nanos() as i64,
            identifier: *PART_IDENTIFIER,
            lock_level: 0,
            operating_mode,
            start_condition: *PART_START_CONDITION,
            num_assigned_cores: 1,
        }
    }

    fn set_partition_mode<L: Locked>(operating_mode: OperatingMode) -> Result<(), ErrorReturnCode> {
        // TODO: Handle transitions
        // TODO: Do not unwrap error
        std::env::set_var(MODE_ENV, (operating_mode as u32).to_string());

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
            1 => *APERIODIC_PROCESS,
            2 => *PERIODIC_PROCESS,
            _ => todo!("Return error"),
        };
        // TODO do not unwrap
        let proc = file.read().unwrap().unwrap();
        proc.activate().unwrap();

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
        exit(0)
    }

    fn get_time<L: Locked>() -> ApexSystemTime {
        SYSTEM_TIME
            .elapsed()
            .as_nanos()
            .clamp(0, ApexSystemTime::MAX as u128) as ApexSystemTime
    }
}

impl ApexError for ApexLinuxPartition {
    fn report_application_message<L: Locked>(message: &[ApexByte]) -> Result<(), ErrorReturnCode> {
        if message.len() > MAX_ERROR_MESSAGE_SIZE {
            return Err(ErrorReturnCode::InvalidParam);
        }
        if let Ok(msg) = std::str::from_utf8(message) {
            HEALTH_EVENT_SENDER
                .try_send(&PartitionEvent::Message(msg.to_string()))
                .unwrap();
        }
        Ok(())
    }

    fn raise_application_error<L: Locked>(
        error_code: ErrorCode,
        message: &[ApexByte],
    ) -> Result<(), ErrorReturnCode> {
        Self::report_application_message::<L>(message)
    }
}