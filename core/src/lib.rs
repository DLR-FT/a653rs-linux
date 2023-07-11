//! This crate is a library both used by the partition and the hypervisor side
//! of the Linux based ARINC 653 hypervisor.
//!
//! The pivot for interaction between the hypervisor and the partitions is
//! formed by a Unix Domain Socket, which is exposed under a well-known path
//! ([syscall::SYSCALL_SOCKET_PATH]) by the hypervisor
//! prior to invocation of a partition.

#[macro_use]
extern crate log;
#[macro_use]
extern crate enum_primitive;

pub mod cgroup;
pub mod channel;
pub mod error;
pub mod fd;
pub mod file;
pub mod health;
pub mod health_event;
pub mod ipc;
pub mod mfd;
pub mod partition;
pub mod queuing;
pub mod sampling;
pub mod shmem;
pub mod syscall;
