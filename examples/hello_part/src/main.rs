#![allow(mutable_transmutes)]

#[macro_use]
extern crate log;

use std::cell::UnsafeCell;
use std::fs::read_to_string;
use std::mem::transmute;
use std::ops::DerefMut;
use std::pin::Pin;
use std::str::FromStr;
use std::thread::sleep;
use std::time::Duration;

use apex_hal::prelude::*;
use linux_apex_core::cgroup::{CGroup, DomainCGroup};
use linux_apex_core::file::get_fd;
use linux_apex_core::shmem::MmapMutExt;
use linux_apex_partition::partition::Partition;
use linux_apex_partition::process::Process as LinuxProcess;
use linux_apex_partition::{ProcessesType, PROCESSES};
use log::LevelFilter;
use memmap2::{Mmap, MmapOptions};
use procfs::process::Process as ProcProcess;

fn main() {
    log_panics::init();

    pretty_env_logger::formatted_builder()
        .parse_filters(&std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()))
        .format(linux_apex_core::log_helper::format)
        .filter_module("polling", LevelFilter::Off)
        .format_timestamp_secs()
        .init();

    //println!("Uid Child: {}", nix::unistd::getuid());
    trace!(
        "Pid Child: {:?}",
        Process::<Partition>::get_self().unwrap().id()
    );

    let cg = DomainCGroup::new(CGroup::mount_point().unwrap(), "process1").unwrap();

    //debug!(
    //    "Partition CGroup Type: {}",
    //    read_to_string(cg.path().parent().unwrap().join("cgroup.type")).unwrap()
    //);
    //debug!(
    //    "Partition Child CGroup Type: {}",
    //    read_to_string(cg.path().join("cgroup.type")).unwrap()
    //);
    debug!(
        "Partition CGroup Controllers: {}",
        read_to_string(cg.path().parent().unwrap().join("cgroup.controllers")).unwrap()
    );

    let fds = ProcProcess::myself().unwrap().fd().unwrap();
    for f in fds.flatten() {
        trace!("Existing File Descriptor: {f:#?}")
    }

    let mut test_proc = LinuxProcess::new(ProcessAttribute {
        period: apex_hal::prelude::SystemTime::Infinite,
        time_capacity: apex_hal::prelude::SystemTime::Infinite,
        entry_point: test,
        stack_size: 1000000,
        base_priority: 1,
        deadline: apex_hal::prelude::Deadline::Soft,
        name: Name::from_str("test").unwrap(),
    })
    .unwrap();
    test_proc.init().unwrap();
    test_proc.unfreeze().unwrap();

    let fds = ProcProcess::myself().unwrap().fd().unwrap();
    for f in fds.flatten() {
        trace!("Existing File Descriptor: {f:#?}")
    }
    debug!("proc fd: {}", PROCESSES.get_fd());
    //let mut map = unsafe { MmapOptions::default().map_mut(PROCESSES.get_fd()).expect("Map failed") };
    //let procs = map.as_mut_type::<ProcessesType>().unwrap();
    //procs.push(1).unwrap();
    //println!("{}", procs.len());

    loop {
        info!("Ping: {:?}", Time::<Partition>::get_time());
        //PartitionContext::send(15);
        //println!("{:?}", PartitionContext::recv());
        sleep(Duration::from_millis(500));
    }
}

fn test() {
    //stdio_override::StdoutOverride::override_raw(1).unwrap();
    info!(
        "Hello from Process: {}",
        Process::<Partition>::get_self().unwrap().id()
    );

    let fds = ProcProcess::myself().unwrap().fd().unwrap();
    for f in fds.flatten() {
        trace!("Existing File Descriptor: {f:#?}")
    }

    //let mut map = unsafe { MmapOptions::default().map_mut(PROCESSES.get_fd()).unwrap() };
    //let procs = map.as_mut_type::<ProcessesType>().unwrap();
    //procs.push(1).unwrap();
    //println!("{}", procs.len());

    loop {
        sleep(Duration::from_secs(1))
    }
}
