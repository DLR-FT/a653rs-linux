//! Partition side of the ARINC 653 Linux hypervisor
//!
//! This crate is a library, implementing and providing the ARINC 653 API meant
//! to be used from within a partition running on the Linux hypervisor.

#![deny(dead_code)]

#[macro_use]
extern crate log;

#[cfg(feature = "socket")]
use std::net::{TcpStream, UdpSocket};
#[cfg(feature = "socket")]
use std::os::fd::FromRawFd;

#[cfg(feature = "socket")]
use a653rs_linux_core::ipc::IoReceiver;

use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;
use std::time::{Duration, Instant};

use a653rs::prelude::OperatingMode;
use a653rs_linux_core::file::{get_memfd, TempFile};
use a653rs_linux_core::health_event::PartitionCall;
use a653rs_linux_core::ipc::{self, IpcSender};
use a653rs_linux_core::partition::*;
use a653rs_linux_core::syscall::SYSCALL_SOCKET_PATH;
use nix::sys::socket::{self, connect, AddressFamily, SockFlag, SockType, UnixAddr};
use once_cell::sync::{Lazy, OnceCell};
use process::Process;
use tinyvec::ArrayVec;

pub mod apex;
pub mod partition;
pub mod syscall;
//mod scheduler;
pub(crate) mod process;

const SAMPLING_PORTS_FILE: &str = "sampling_channels";
// const MAX_SAMPLING_PORTS: usize = 32;
const QUEUING_PORTS_FILE: &str = "queuing_channels";

pub(crate) static CONSTANTS: Lazy<PartitionConstants> =
    Lazy::new(|| PartitionConstants::open().unwrap());

pub(crate) static SYSTEM_TIME: Lazy<Instant> = Lazy::new(|| {
    TempFile::<Instant>::try_from(CONSTANTS.start_time_fd)
        .unwrap()
        .read()
        .unwrap()
});

pub(crate) static PARTITION_MODE: Lazy<TempFile<OperatingMode>> =
    Lazy::new(|| TempFile::<OperatingMode>::try_from(CONSTANTS.partition_mode_fd).unwrap());

pub(crate) static PERIODIC_PROCESS: OnceCell<Arc<Process>> = OnceCell::new();
pub(crate) static APERIODIC_PROCESS: OnceCell<Arc<Process>> = OnceCell::new();

pub(crate) type SamplingPortsType = (usize, Duration);
pub(crate) static SAMPLING_PORTS: Lazy<TempFile<ArrayVec<[SamplingPortsType; 32]>>> =
    Lazy::new(|| {
        if let Ok(fd) = get_memfd(SAMPLING_PORTS_FILE) {
            TempFile::try_from(fd).unwrap()
        } else {
            let file = TempFile::create(SAMPLING_PORTS_FILE).unwrap();
            file.write(&Default::default()).unwrap();
            file
        }
    });

pub(crate) type QueuingPortsType = usize;
pub(crate) static QUEUING_PORTS: Lazy<TempFile<ArrayVec<[QueuingPortsType; 32]>>> =
    Lazy::new(|| {
        if let Ok(fd) = get_memfd(QUEUING_PORTS_FILE) {
            TempFile::try_from(fd).unwrap()
        } else {
            let file = TempFile::create(QUEUING_PORTS_FILE).unwrap();
            file.write(&Default::default()).unwrap();
            file
        }
    });

pub(crate) static SENDER: Lazy<IpcSender<PartitionCall>> =
    Lazy::new(|| ipc::connect_sender(PartitionConstants::IPC_SENDER.as_ref()).unwrap());

#[cfg(feature = "socket")]
pub(crate) static UDP_IO_RX: Lazy<IoReceiver<UdpSocket>> =
    Lazy::new(|| unsafe { IoReceiver::<UdpSocket>::from_raw_fd(CONSTANTS.udp_io_fd) });

#[cfg(feature = "socket")]
pub(crate) static TCP_IO_RX: Lazy<IoReceiver<TcpStream>> =
    Lazy::new(|| unsafe { IoReceiver::<TcpStream>::from_raw_fd(CONSTANTS.tcp_io_fd) });

pub(crate) static SYSCALL: Lazy<OwnedFd> = Lazy::new(|| {
    let syscall_socket = socket::socket(
        AddressFamily::Unix,
        SockType::Datagram,
        SockFlag::empty(),
        None,
    )
    .unwrap();

    connect(
        syscall_socket.as_raw_fd(),
        &UnixAddr::new(SYSCALL_SOCKET_PATH).unwrap(),
    )
    .unwrap();

    syscall_socket
});

#[cfg(feature = "socket")]
pub(crate) static UDP_SOCKETS: Lazy<Vec<UdpSocket>> = Lazy::new(|| receive_sockets(&UDP_IO_RX));

#[cfg(feature = "socket")]
pub(crate) static TCP_SOCKETS: Lazy<Vec<TcpStream>> = Lazy::new(|| receive_sockets(&TCP_IO_RX));

/// Receives sockets from the hypervisor.
/// Will panic if an error occurs while receiving the file descriptors of the
/// sockets.
#[cfg(feature = "socket")]
fn receive_sockets<T: FromRawFd>(receiver: &IoReceiver<T>) -> Vec<T> {
    let mut sockets: Vec<T> = Vec::default();
    loop {
        match unsafe { receiver.try_receive() } {
            Ok(i) => {
                if let Some(i) = i {
                    sockets.push(i);
                } else {
                    return sockets;
                }
            }
            Err(e) => panic!("Could not receive sockets from hypervisor: {e:?}"),
        }
    }
}
