// TODO remove this
#![allow(unused_variables)]

use std::process::exit;
use std::thread::sleep;

use apex_hal::bindings::*;
use linux_apex_core::error::SystemError;

use crate::partition::ApexLinuxPartition;
use crate::process::Process as LinuxProcess;
use crate::*;

impl ApexPartition for ApexLinuxPartition {
    fn get_partition_status<L: Locked>() -> ApexPartitionStatus {
        let operating_mode = PARTITION_MODE.read().unwrap();

        ApexPartitionStatus {
            period: CONSTANTS.period.as_nanos() as i64,
            duration: CONSTANTS.duration.as_nanos() as i64,
            identifier: CONSTANTS.identifier,
            lock_level: 0,
            operating_mode,
            start_condition: CONSTANTS.start_condition,
            num_assigned_cores: 1,
        }
    }

    fn set_partition_mode<L: Locked>(operating_mode: OperatingMode) -> Result<(), ErrorReturnCode> {
        let current_mode = PARTITION_MODE.read().unwrap();

        if let OperatingMode::Idle = current_mode {
            panic!()
        }

        match (operating_mode, current_mode) {
            (OperatingMode::Normal, OperatingMode::Normal) => Err(ErrorReturnCode::NoAction),
            (OperatingMode::WarmStart, OperatingMode::ColdStart) => {
                Err(ErrorReturnCode::InvalidMode)
            }
            (OperatingMode::Normal, _) => {
                SENDER
                    .try_send(&PartitionCall::Transition(operating_mode))
                    .unwrap();
                loop {
                    sleep(Duration::from_secs(500))
                }
            }
            (_, _) => {
                SENDER
                    .try_send(&PartitionCall::Transition(operating_mode))
                    .unwrap();
                exit(0)
            }
        }
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
        let proc = match process_id {
            1 => APERIODIC_PROCESS.read().unwrap(),
            2 => PERIODIC_PROCESS.read().unwrap(),
            _ => None,
        };

        let proc = match proc {
            Some(proc) => proc,
            None => return Err(ErrorReturnCode::InvalidParam),
        };

        // TODO use a bigger result which contains both panic and non-panic errors
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

        proc.cg().unwrap().freeze().unwrap();
        Ok(())
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
            SENDER
                .try_send(&PartitionCall::Message(msg.to_string()))
                .unwrap();
        }
        Ok(())
    }

    fn raise_application_error<L: Locked>(
        error_code: ErrorCode,
        message: &[ApexByte],
    ) -> Result<(), ErrorReturnCode> {
        if let ErrorCode::ApplicationError = error_code {
            Self::report_application_message::<L>(message).unwrap();
            Self::raise_system_error(SystemError::ApplicationError);
            Ok(())
        } else {
            Err(ErrorReturnCode::InvalidParam)
        }
    }
}
