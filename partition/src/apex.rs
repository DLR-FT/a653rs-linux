use std::process::exit;
use std::thread::sleep;

use a653rs::bindings::*;
use a653rs::prelude::{Name, SystemTime};
use a653rs_linux_core::error::SystemError;
use a653rs_linux_core::sampling::{SamplingDestination, SamplingSource};
use nix::libc::EAGAIN;

use crate::partition::ApexLinuxPartition;
use crate::process::Process as LinuxProcess;
use crate::*;

impl ApexPartitionP4 for ApexLinuxPartition {
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

impl ApexProcessP4 for ApexLinuxPartition {
    fn create_process<L: Locked>(
        attributes: &ApexProcessAttribute,
    ) -> Result<ProcessId, ErrorReturnCode> {
        // TODO do not unwrap both
        // Check current State (only allowed in warm and cold start)
        let attr = attributes.clone().into();
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

impl ApexSamplingPortP4 for ApexLinuxPartition {
    fn create_sampling_port<L: Locked>(
        sampling_port_name: SamplingPortName,
        // TODO Return ErrorCode for wrong max message size
        _max_message_size: MessageSize,
        port_direction: PortDirection,
        refresh_period: ApexSystemTime,
    ) -> Result<SamplingPortId, ErrorReturnCode> {
        if refresh_period <= 0 {
            trace!("yielding InvalidConfig, because refresh period <= 0");
            return Err(ErrorReturnCode::InvalidConfig);
        }

        let name = Name::new(sampling_port_name);
        let name = name.to_str().map_err(|e| {
            trace!("yielding InvalidConfig, because sampling port is not valid UTF-8:\n{e}");
            ErrorReturnCode::InvalidConfig
        })?;
        if let Some((i, s)) = CONSTANTS
            .sampling
            .iter()
            .enumerate()
            .find(|(_, s)| s.name.eq(name))
        {
            if s.dir != port_direction {
                trace!("yielding InvalidConfig, because mismatching port direction:\nexpected {:?}, got {port_direction:?}", s.dir);
                return Err(ErrorReturnCode::InvalidConfig);
            }

            let refresh = SystemTime::new(refresh_period).unwrap_duration();
            let ch = (i, refresh);

            let mut channels = SAMPLING_PORTS.read().unwrap();
            if channels.try_push(ch).is_some() {
                trace!(
                    "yielding InvalidConfig, maximum number of sampling ports already reached: {}",
                    channels.len()
                );
                return Err(ErrorReturnCode::InvalidConfig);
            }
            SAMPLING_PORTS.write(&channels).unwrap();

            return Ok(channels.len() as SamplingPortId);
        }

        trace!("yielding InvalidConfig, configuration does not declare sampling port {name}");
        Err(ErrorReturnCode::InvalidConfig)
    }

    fn write_sampling_message<L: Locked>(
        sampling_port_id: SamplingPortId,
        message: &[ApexByte],
    ) -> Result<(), ErrorReturnCode> {
        if let Some((port, _)) = SAMPLING_PORTS
            .read()
            .unwrap()
            .get(sampling_port_id as usize - 1)
        {
            if let Some(port) = CONSTANTS.sampling.get(*port) {
                if message.len() > port.msg_size {
                    return Err(ErrorReturnCode::InvalidConfig);
                } else if message.is_empty() {
                    return Err(ErrorReturnCode::InvalidParam);
                } else if port.dir != PortDirection::Source {
                    return Err(ErrorReturnCode::InvalidMode);
                }
                SamplingSource::try_from(port.fd).unwrap().write(message);
                return Ok(());
            }
        }

        Err(ErrorReturnCode::InvalidParam)
    }

    unsafe fn read_sampling_message<L: Locked>(
        sampling_port_id: SamplingPortId,
        message: &mut [ApexByte],
    ) -> Result<(Validity, MessageSize), ErrorReturnCode> {
        let read = if let Ok(read) = SAMPLING_PORTS.read() {
            read
        } else {
            return Err(ErrorReturnCode::NotAvailable);
        };
        if let Some((port, val)) = read.get(sampling_port_id as usize - 1) {
            if let Some(port) = CONSTANTS.sampling.get(*port) {
                if message.is_empty() {
                    return Err(ErrorReturnCode::InvalidParam);
                } else if port.dir != PortDirection::Destination {
                    return Err(ErrorReturnCode::InvalidMode);
                }
                let (msg_len, copied) = SamplingDestination::try_from(port.fd)
                    .unwrap()
                    .read(message);

                if msg_len == 0 {
                    return Err(ErrorReturnCode::NoAction);
                }

                let valid = if copied.elapsed() <= *val {
                    Validity::Valid
                } else {
                    Validity::Invalid
                };

                return Ok((valid, msg_len as u32));
            }
        }

        Err(ErrorReturnCode::InvalidParam)
    }
}

impl ApexTimeP4 for ApexLinuxPartition {
    fn periodic_wait() -> Result<(), ErrorReturnCode> {
        // TODO do not unwrap() (Maybe raise an error?);
        let proc = LinuxProcess::get_self().unwrap();
        if !proc.periodic() {
            return Err(ErrorReturnCode::InvalidMode);
        }

        proc.cg().unwrap().freeze().unwrap();
        Ok(())
    }

    fn get_time() -> ApexSystemTime {
        SYSTEM_TIME
            .elapsed()
            .as_nanos()
            .clamp(0, ApexSystemTime::MAX as u128) as ApexSystemTime
    }
}

impl ApexErrorP4 for ApexLinuxPartition {
    fn report_application_message<L: Locked>(message: &[ApexByte]) -> Result<(), ErrorReturnCode> {
        if message.len() > MAX_ERROR_MESSAGE_SIZE {
            return Err(ErrorReturnCode::InvalidParam);
        }
        if let Ok(msg) = std::str::from_utf8(message) {
            // Logging may fail temporarily, because the resource can not be written to
            // (e.g. queue is full), but the API does not allow us any other
            // return code than INVALID_PARAM.
            if let Err(e) = SENDER.try_send(&PartitionCall::Message(msg.to_string())) {
                if let Some(e) = e.source().downcast_ref::<std::io::Error>() {
                    if e.raw_os_error() == Some(EAGAIN) {
                        return Ok(());
                    }
                }
                panic!("Failed to report application message: {}", e);
            }
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
