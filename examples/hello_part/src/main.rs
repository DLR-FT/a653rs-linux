#![allow(mutable_transmutes)]

#[macro_use]
extern crate log;

use std::fs::read_to_string;
use std::str::FromStr;
use std::thread::sleep;
use std::time::Duration;

use apex_hal::prelude::*;
use linux_apex_core::cgroup::CGroup;
use linux_apex_partition::partition::ApexLinuxPartition;
use linux_apex_partition::process::Process as LinuxProcess;
use linux_apex_partition::APERIODIC_PROCESS;
use log::LevelFilter;
use nix::unistd::getpid;

fn main() {
    log_panics::init();

    pretty_env_logger::formatted_builder()
        .parse_filters(&std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()))
        .format(linux_apex_core::log_helper::format)
        .filter_module("polling", LevelFilter::Off)
        .format_timestamp_secs()
        .init();

    //println!("Uid Child: {}", nix::unistd::getuid());
    //trace!(
    //    "Pid Child: {:?}",
    //    Process::<ApexLinuxPartition>::get_self().unwrap().id()
    //);

    let cg = CGroup::new(CGroup::mount_point().unwrap(), "process1").unwrap();

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

    let test_proc_name = Name::from_str("test").unwrap();
    let test_proc = LinuxProcess::create(ProcessAttribute {
        period: apex_hal::prelude::SystemTime::Infinite,
        time_capacity: apex_hal::prelude::SystemTime::Infinite,
        entry_point: test,
        stack_size: 1000000,
        base_priority: 1,
        deadline: apex_hal::prelude::Deadline::Soft,
        name: test_proc_name,
    })
    .unwrap();
    let mut test_proc = APERIODIC_PROCESS.read().unwrap().unwrap();
    test_proc.start().unwrap();
    test_proc.unfreeze().unwrap();

    loop {
        //info!("Ping: {:?}", Time::<ApexLinuxPartition>::get_time());
        //PartitionContext::send(15);
        //println!("{:?}", PartitionContext::recv());
        //info!("Ping Main: {:?}", Time::<ApexLinuxPartition>::get_time());
        sleep(Duration::from_millis(500));
        test_proc.start().unwrap();
        test_proc.unfreeze().unwrap();
    }
}

fn test() {
    //stdio_override::StdoutOverride::override_raw(1).unwrap();
    info!(
        "Hello from Process: {}",
        //Process::<ApexLinuxPartition>::get_self().unwrap().id(),
        getpid()
    );

    //let mut map = unsafe { MmapOptions::default().map_mut(PROCESSES.get_fd()).unwrap() };
    //let procs = map.as_mut_type::<ProcessesType>().unwrap();
    //procs.push(1).unwrap();
    //println!("{}", procs.len());

    loop {
        info!("Ping Proc: {:?}", Time::<ApexLinuxPartition>::get_time());
        sleep(Duration::from_secs(1))
    }
}
