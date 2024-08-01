use a653rs_linux_core::syscall::{SyscallRequest, SyscallResponse};
use anyhow::Result;

use crate::SYSCALL;

pub fn execute(request: SyscallRequest) -> Result<SyscallResponse> {
    SYSCALL.execute(request)
}
