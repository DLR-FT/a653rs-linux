//! Implementation of the mechanism to perform system calls

use std::os::fd::AsFd;

use a653rs_linux_core::syscall::{SyscallRequest, SyscallResponse};
use anyhow::Result;

use crate::SYSCALL;

pub fn execute(request: SyscallRequest) -> Result<SyscallResponse> {
    a653rs_linux_core::syscall::sender::execute_fd(SYSCALL.as_fd(), request)
}
