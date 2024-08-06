use a653rs::prelude::PartitionId;
use a653rs_linux_core::syscall::syscalls::{ReceiveQueuingMessage, SendQueuingMessage, Syscall};
use a653rs_linux_core::syscall::SyscallType;
use anyhow::Result;

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
        hv_state: &mut (),
        current_partition: PartitionId,
    ) -> Result<Self::Returns, a653rs::bindings::ErrorReturnCode>;
}

fn handle_syscall(
    ty: SyscallType,
    params: &[u8],
    hypervisor_state: &mut HypervisorState,
    current_partition: PartitionId,
) -> Result<Vec<u8>> {
    match ty {
        SyscallType::SendQueuingMessage => SendQueuingMessage::handle_with_serialization(
            params,
            hypervisor_state,
            current_partition,
        ),
        SyscallType::ReceiveQueuingMessage => ReceiveQueuingMessage::handle_with_serialization(
            params,
            hypervisor_state,
            current_partition,
        ),
        other_ty => {
            todo!("Implement syscall {other_ty:?}")
        }
    }
}

// --------------- HANDLER IMPLEMENTATIONS ---------------

impl<'params> SyscallHandler<'params> for SendQueuingMessage {
    fn handle(
        _params: Self::Params,
        _hv_state: &mut HypervisorState,
        _current_partition: PartitionId,
    ) -> Result<Self::Returns, a653rs::bindings::ErrorReturnCode> {
        todo!("Handle SendQueuingMessage")
    }
}

impl<'params> SyscallHandler<'params> for ReceiveQueuingMessage {
    fn handle(
        _params: Self::Params,
        _hv_state: &mut HypervisorState,
        _current_partition: PartitionId,
    ) -> Result<Self::Returns, a653rs::bindings::ErrorReturnCode> {
        todo!("Handle ReceiveQueuingMessage")
    }
}
