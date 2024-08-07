use a653rs::prelude::PartitionId;
use a653rs_linux_core::syscall::syscalls::{self, Syscall};
use a653rs_linux_core::syscall::{self, SyscallType};
use anyhow::Result;

// Temporary replacement until the new hypervisor architecture allows for a
// modular and mutable hypervisor state during partition execution
type HypervisorState = ();

trait SyscallHandler<'params>: Syscall<'params> + Sized {
    fn handle_with_serialization(
        serialized_params: &'params [u8],
        hv_state: &mut HypervisorState,
        current_partition: PartitionId,
    ) -> Result<Vec<u8>> {
        a653rs_linux_core::syscall::receiver::wrap_serialization::<Self, _>(
            serialized_params,
            |params| Self::handle(params, hv_state, current_partition),
        )
    }

    fn handle(
        params: Self::Params,
        hv_state: &mut HypervisorState,
        current_partition: PartitionId,
    ) -> Result<Self::Returns, a653rs::bindings::ErrorReturnCode>;
}

pub fn handle_syscall(
    ty: SyscallType,
    serialized_params: &[u8],
    hypervisor_state: &mut HypervisorState,
    current_partition: PartitionId,
) -> Result<Vec<u8>> {
    use syscalls::*;

    let handler_fn = match ty {
        SyscallType::GetPartitionStatus => GetPartitionStatus::handle_with_serialization,
        SyscallType::SetPartitionMode => SetPartitionMode::handle_with_serialization,
        SyscallType::Start => Start::handle_with_serialization,
        SyscallType::CreateSamplingPort => CreateSamplingPort::handle_with_serialization,
        SyscallType::WriteSamplingMessage => WriteSamplingMessage::handle_with_serialization,
        SyscallType::ReadSamplingMessage => ReadSamplingMessage::handle_with_serialization,
        SyscallType::CreateQueuingPort => CreateQueuingPort::handle_with_serialization,
        SyscallType::SendQueuingMessage => SendQueuingMessage::handle_with_serialization,
        SyscallType::ReceiveQueuingMessage => ReceiveQueuingMessage::handle_with_serialization,
        SyscallType::GetQueuingPortStatus => GetQueuingPortStatus::handle_with_serialization,
        SyscallType::ClearQueuingPort => ClearQueuingPort::handle_with_serialization,
        SyscallType::PeriodicWait => PeriodicWait::handle_with_serialization,
        SyscallType::GetTime => GetTime::handle_with_serialization,
        SyscallType::ReportApplicationMessage => {
            ReportApplicationMessage::handle_with_serialization
        }
        SyscallType::RaiseApplicationError => RaiseApplicationError::handle_with_serialization,
        other_ty => {
            todo!("Implement syscall {other_ty:?}")
        }
    };

    handler_fn(serialized_params, hypervisor_state, current_partition)
}

// --------------- HANDLER IMPLEMENTATIONS ---------------

impl SyscallHandler<'_> for syscalls::GetPartitionStatus {
    fn handle(
        _params: Self::Params,
        _hv_state: &mut HypervisorState,
        _current_partition: PartitionId,
    ) -> Result<Self::Returns, a653rs::bindings::ErrorReturnCode> {
        todo!("handle syscall GetPartitionStatus")
    }
}

impl<'params> SyscallHandler<'params> for syscalls::SetPartitionMode {
    fn handle(
        _params: Self::Params,
        _hv_state: &mut HypervisorState,
        _current_partition: PartitionId,
    ) -> Result<Self::Returns, a653rs::bindings::ErrorReturnCode> {
        todo!("handle syscall SetPartitionMode")
    }
}

impl<'params> SyscallHandler<'params> for syscalls::Start {
    fn handle(
        _params: Self::Params,
        _hv_state: &mut HypervisorState,
        _current_partition: PartitionId,
    ) -> Result<Self::Returns, a653rs::bindings::ErrorReturnCode> {
        todo!("handle syscall Start")
    }
}

impl<'params> SyscallHandler<'params> for syscalls::CreateSamplingPort {
    fn handle(
        _params: Self::Params,
        _hv_state: &mut HypervisorState,
        _current_partition: PartitionId,
    ) -> Result<Self::Returns, a653rs::bindings::ErrorReturnCode> {
        todo!("handle syscall CreateSamplingPort")
    }
}

impl<'params> SyscallHandler<'params> for syscalls::WriteSamplingMessage {
    fn handle(
        _params: Self::Params,
        _hv_state: &mut HypervisorState,
        _current_partition: PartitionId,
    ) -> Result<Self::Returns, a653rs::bindings::ErrorReturnCode> {
        todo!("handle syscall WriteSamplingMessage")
    }
}

impl<'params> SyscallHandler<'params> for syscalls::ReadSamplingMessage {
    fn handle(
        _params: Self::Params,
        _hv_state: &mut HypervisorState,
        _current_partition: PartitionId,
    ) -> Result<Self::Returns, a653rs::bindings::ErrorReturnCode> {
        todo!("handle syscall ReadSamplingMessage")
    }
}

impl<'params> SyscallHandler<'params> for syscalls::CreateQueuingPort {
    fn handle(
        _params: Self::Params,
        _hv_state: &mut HypervisorState,
        _current_partition: PartitionId,
    ) -> Result<Self::Returns, a653rs::bindings::ErrorReturnCode> {
        todo!("handle syscall CreateQueuingPort")
    }
}

impl<'params> SyscallHandler<'params> for syscalls::SendQueuingMessage {
    fn handle(
        _params: Self::Params,
        _hv_state: &mut HypervisorState,
        _current_partition: PartitionId,
    ) -> Result<Self::Returns, a653rs::bindings::ErrorReturnCode> {
        todo!("handle syscall SendQueuingMessage")
    }
}

impl<'params> SyscallHandler<'params> for syscalls::ReceiveQueuingMessage {
    fn handle(
        _params: Self::Params,
        _hv_state: &mut HypervisorState,
        _current_partition: PartitionId,
    ) -> Result<Self::Returns, a653rs::bindings::ErrorReturnCode> {
        todo!("handle syscall ReceiveQueuingMessage")
    }
}

impl<'params> SyscallHandler<'params> for syscalls::GetQueuingPortStatus {
    fn handle(
        _params: Self::Params,
        _hv_state: &mut HypervisorState,
        _current_partition: PartitionId,
    ) -> Result<Self::Returns, a653rs::bindings::ErrorReturnCode> {
        todo!("handle syscall GetQueuingPortStatus")
    }
}

impl<'params> SyscallHandler<'params> for syscalls::ClearQueuingPort {
    fn handle(
        _params: Self::Params,
        _hv_state: &mut HypervisorState,
        _current_partition: PartitionId,
    ) -> Result<Self::Returns, a653rs::bindings::ErrorReturnCode> {
        todo!("handle syscall ClearQueuingPort")
    }
}

impl<'params> SyscallHandler<'params> for syscalls::PeriodicWait {
    fn handle(
        _params: Self::Params,
        _hv_state: &mut HypervisorState,
        _current_partition: PartitionId,
    ) -> Result<Self::Returns, a653rs::bindings::ErrorReturnCode> {
        todo!("handle syscall PeriodicWait")
    }
}

impl<'params> SyscallHandler<'params> for syscalls::GetTime {
    fn handle(
        _params: Self::Params,
        _hv_state: &mut HypervisorState,
        _current_partition: PartitionId,
    ) -> Result<Self::Returns, a653rs::bindings::ErrorReturnCode> {
        todo!("handle syscall GetTime")
    }
}

impl<'params> SyscallHandler<'params> for syscalls::ReportApplicationMessage {
    fn handle(
        _params: Self::Params,
        _hv_state: &mut HypervisorState,
        _current_partition: PartitionId,
    ) -> Result<Self::Returns, a653rs::bindings::ErrorReturnCode> {
        todo!("handle syscall ReportApplicationMessage")
    }
}

impl<'params> SyscallHandler<'params> for syscalls::RaiseApplicationError {
    fn handle(
        _params: Self::Params,
        _hv_state: &mut HypervisorState,
        _current_partition: PartitionId,
    ) -> Result<Self::Returns, a653rs::bindings::ErrorReturnCode> {
        todo!("handle syscall RaiseApplicationError")
    }
}
