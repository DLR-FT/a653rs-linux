use std::process::exit;
use std::thread::sleep;

use a653rs::bindings::*;
use a653rs::prelude::{Name, SystemTime};
use nix::libc::EAGAIN;

use a653rs_linux_core::error::SystemError;
use a653rs_linux_core::queuing::{QueuingDestination, QueuingSource};
use a653rs_linux_core::sampling::{SamplingDestination, SamplingSource};

use crate::partition::ApexLinuxPartition;
use crate::process::Process as LinuxProcess;
use crate::*;

impl ApexPartitionP4 for ApexLinuxPartition {
    fn get_partition_status() -> ApexPartitionStatus {
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

    fn set_partition_mode(operating_mode: OperatingMode) -> Result<(), ErrorReturnCode> {
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
    fn create_process(attributes: &ApexProcessAttribute) -> Result<ProcessId, ErrorReturnCode> {
        // TODO do not unwrap both
        // Check current State (only allowed in warm and cold start)
        let attr = attributes.clone().into();
        Ok(LinuxProcess::create(attr).unwrap())
    }

    fn start(process_id: ProcessId) -> Result<(), ErrorReturnCode> {
        let proc = match process_id {
            1 => APERIODIC_PROCESS.get(),
            2 => PERIODIC_PROCESS.get(),
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
    fn create_sampling_port(
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

    fn write_sampling_message(
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

    unsafe fn read_sampling_message(
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

impl ApexQueuingPortP4 for ApexLinuxPartition {
    fn create_queuing_port(
        queuing_port_name: QueuingPortName,
        max_message_size: MessageSize,
        max_nb_message: MessageRange,
        port_direction: PortDirection,
        queuing_discipline: QueuingDiscipline,
    ) -> Result<QueuingPortId, ErrorReturnCode> {
        // TODO perform necessary checks

        let name = Name::new(queuing_port_name);
        let name = name.to_str().map_err(|e| {
            trace!("yielding InvalidConfig, because queuing port is not valid UTF-8:\n{e}");
            ErrorReturnCode::InvalidConfig
        })?;

        if let Some((i, q)) = CONSTANTS
            .queuing
            .iter()
            .enumerate()
            .find(|(_, q)| q.name.eq(name))
        {
            if q.dir != port_direction {
                trace!("yielding InvalidConfig, because queuing port has mismatching port direction:\nexpected {:?}, got {port_direction:?}", q.dir);
                return Err(ErrorReturnCode::InvalidConfig);
            }

            let ch = i;

            let mut channels = QUEUING_PORTS.read().unwrap();
            if channels.try_push(ch).is_some() {
                trace!(
                    "yielding InvalidConfig, maximum number of queuing ports already reached: {}",
                    channels.len()
                );
                return Err(ErrorReturnCode::InvalidConfig);
            }
            QUEUING_PORTS.write(&channels).unwrap();

            return Ok(channels.len() as QueuingPortId);
        }

        trace!("yielding InvalidConfig, configuration does not declare queuing port {name}");
        Err(ErrorReturnCode::InvalidConfig)
    }

    fn send_queuing_message(
        queuing_port_id: QueuingPortId,
        message: &[ApexByte],
        time_out: ApexSystemTime,
    ) -> Result<(), ErrorReturnCode> {
        if let Some(port) = QUEUING_PORTS
            .read()
            .unwrap()
            .get(queuing_port_id as usize - 1)
        {
            if let Some(port) = CONSTANTS.queuing.get(*port) {
                if message.len() > port.msg_size {
                    return Err(ErrorReturnCode::InvalidConfig);
                } else if message.is_empty() {
                    return Err(ErrorReturnCode::InvalidParam);
                } else if port.dir != PortDirection::Source {
                    return Err(ErrorReturnCode::InvalidMode);
                }
                QueuingSource::try_from(port.fd).unwrap().write(message);
                return Ok(());
            }
        }

        Err(ErrorReturnCode::InvalidParam)
    }

    unsafe fn receive_queuing_message(
        queuing_port_id: QueuingPortId,
        time_out: ApexSystemTime,
        message: &mut [ApexByte],
    ) -> Result<(MessageSize, QueueOverflow), ErrorReturnCode> {
        let read = if let Ok(read) = QUEUING_PORTS.read() {
            read
        } else {
            return Err(ErrorReturnCode::NotAvailable);
        };
        if let Some(port) = read.get(queuing_port_id as usize - 1) {
            if let Some(port) = CONSTANTS.queuing.get(*port) {
                if message.is_empty() {
                    return Err(ErrorReturnCode::InvalidParam);
                } else if port.dir != PortDirection::Destination {
                    return Err(ErrorReturnCode::InvalidMode);
                }
                let msg_len = QueuingDestination::try_from(port.fd).unwrap().read(message);

                // TODO: validity like with sampling ports?

                return Ok((msg_len as MessageSize, false));
            }
        }

        Err(ErrorReturnCode::InvalidParam)
    }

    fn get_queuing_port_status(
        queuing_port_id: QueuingPortId,
    ) -> Result<QueuingPortStatus, ErrorReturnCode> {
        todo!()
    }

    fn clear_queuing_port(queuing_port_id: QueuingPortId) -> Result<(), ErrorReturnCode> {
        todo!()
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
    fn report_application_message(message: &[ApexByte]) -> Result<(), ErrorReturnCode> {
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

    fn raise_application_error(
        error_code: ErrorCode,
        message: &[ApexByte],
    ) -> Result<(), ErrorReturnCode> {
        if let ErrorCode::ApplicationError = error_code {
            Self::report_application_message(message).unwrap();
            Self::raise_system_error(SystemError::ApplicationError);
            Ok(())
        } else {
            Err(ErrorReturnCode::InvalidParam)
        }
    }
}
