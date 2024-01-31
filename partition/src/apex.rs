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
        _queuing_discipline: QueuingDiscipline,
    ) -> Result<QueuingPortId, ErrorReturnCode> {
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
            // check max message size
            if max_message_size != q.msg_size as MessageSize {
                trace!("yielding InvalidConfig, because the queuing port max message size ({}) mismatches the configuration table value ({})", max_message_size, q.msg_size);
                return Err(ErrorReturnCode::InvalidConfig);
            } else if max_message_size <= 0 {
                trace!("yielding InvalidConfig, because the queuing port max message size ({}) has to be larger than 0", max_message_size);
                return Err(ErrorReturnCode::InvalidConfig);
            }

            // check max number of messages
            if max_nb_message != q.max_num_msg as MessageRange {
                trace!("yielding InvalidConfig, because the queuing port max number of messages ({}) mismatches the configuration table value ({})", max_nb_message, q.max_num_msg);
                return Err(ErrorReturnCode::InvalidConfig);
            } else if max_nb_message <= 0 {
                trace!("yielding InvalidConfig, because the queuing port max number of messages ({}) has to be larger than 0", max_nb_message);
                return Err(ErrorReturnCode::InvalidConfig);
            }

            // check correct port direction
            if q.dir != port_direction {
                trace!("yielding InvalidConfig, because queuing port has mismatching port direction:\nexpected {:?}, got {port_direction:?}", q.dir);
                return Err(ErrorReturnCode::InvalidConfig);
            }

            // check partition mode
            if let OperatingMode::Normal = PARTITION_MODE.read().unwrap() {
                trace!("yielding InvalidMode, because queuing port creation is not allowed in normal mode");
                return Err(ErrorReturnCode::InvalidMode);
            }

            let ch = i;

            let mut channels = QUEUING_PORTS.read().unwrap();

            // check if channel already exists
            if channels.contains(&ch) {
                trace!("yielding NoAction, because queuing port has already been created");
                return Err(ErrorReturnCode::NoAction);
            }

            // check if max number of channels is reached
            if channels.try_push(ch).is_some() {
                trace!(
                    "yielding InvalidConfig, maximum number of queuing ports (={}) already reached",
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
        _time_out: ApexSystemTime,
    ) -> Result<(), ErrorReturnCode> {
        let port = QUEUING_PORTS
            .read()
            .ok()
            .and_then(|ports| ports.into_iter().nth(queuing_port_id as usize - 1))
            .and_then(|port| CONSTANTS.queuing.get(port))
            .ok_or(ErrorReturnCode::InvalidParam)?;

        if message.len() > port.msg_size {
            return Err(ErrorReturnCode::InvalidConfig);
        } else if message.is_empty() {
            return Err(ErrorReturnCode::InvalidParam);
        } else if port.dir != PortDirection::Source {
            return Err(ErrorReturnCode::InvalidMode);
        }

        let written_bytes = QueuingSource::try_from(port.fd)
            .unwrap()
            .write(message, *SYSTEM_TIME)
            .ok_or(ErrorReturnCode::NotAvailable)?; // Queue is overflowed

        if written_bytes < message.len() {
            warn!(
                "Tried to write {} bytes to queuing port, but only {} bytes could be written",
                message.len(),
                written_bytes
            );
        }

        Ok(())
    }

    unsafe fn receive_queuing_message(
        queuing_port_id: QueuingPortId,
        _time_out: ApexSystemTime,
        message: &mut [ApexByte],
    ) -> Result<(MessageSize, QueueOverflow), ErrorReturnCode> {
        let port = QUEUING_PORTS
            .read()
            .ok()
            .and_then(|ports| ports.into_iter().nth(queuing_port_id as usize - 1))
            .and_then(|port| CONSTANTS.queuing.get(port))
            .ok_or(ErrorReturnCode::InvalidParam)?;

        if message.is_empty() {
            return Err(ErrorReturnCode::InvalidParam);
        } else if port.dir != PortDirection::Destination {
            return Err(ErrorReturnCode::InvalidMode);
        }
        let (msg_len, has_overflowed) = QueuingDestination::try_from(port.fd)
            .unwrap()
            .read(message)
            .ok_or(ErrorReturnCode::NotAvailable)?; // standard states that a length of 0 should also be set here, which the API
                                                    // does not allow

        if has_overflowed {
            // TODO: Also return the message length here. For now just return the error.
            return Err(ErrorReturnCode::InvalidConfig);
        }

        return Ok((msg_len as MessageSize, has_overflowed));
    }

    fn get_queuing_port_status(
        queuing_port_id: QueuingPortId,
    ) -> Result<QueuingPortStatus, ErrorReturnCode> {
        let port = QUEUING_PORTS
            .read()
            .ok()
            .and_then(|ports| ports.into_iter().nth(queuing_port_id as usize - 1))
            .and_then(|port| CONSTANTS.queuing.get(port))
            .ok_or(ErrorReturnCode::InvalidParam)?;

        let num_msgs = match port.dir {
            PortDirection::Source => QueuingSource::try_from(port.fd)
                .unwrap()
                .get_current_num_messages(),
            PortDirection::Destination => QueuingDestination::try_from(port.fd)
                .unwrap()
                .get_current_num_messages(),
        };

        let status = QueuingPortStatus {
            nb_message: num_msgs as MessageRange,
            max_nb_message: port.max_num_msg as MessageRange,
            max_message_size: port.msg_size as MessageSize,
            port_direction: port.dir,
            waiting_processes: 0,
        };

        Ok(status)
    }

    fn clear_queuing_port(queuing_port_id: QueuingPortId) -> Result<(), ErrorReturnCode> {
        let port = QUEUING_PORTS
            .read()
            .ok()
            .and_then(|ports| ports.into_iter().nth(queuing_port_id as usize - 1))
            .and_then(|port| CONSTANTS.queuing.get(port))
            .ok_or(ErrorReturnCode::InvalidParam)?;

        if port.dir != PortDirection::Destination {
            return Err(ErrorReturnCode::InvalidMode);
        }

        QueuingDestination::try_from(port.fd)
            .unwrap()
            .clear(*SYSTEM_TIME);

        Ok(())
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
