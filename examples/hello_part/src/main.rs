#[macro_use]
extern crate log;

use std::{
    fs::read_to_string,
    thread::sleep,
    time::{Duration, Instant},
};

use linux_apex_core::{
    cgroup::ThreadedCGroup,
    file::TempFile,
    partition::{get_fd, SYSTEM_TIME},
};
use procfs::process::Process;

fn main() {
    std::env::set_var(
        "RUST_LOG",
        std::env::var("RUST_LOG").unwrap_or_else(|_| "trace".into()),
    );
    pretty_env_logger::init();

    let id = std::env::args().collect::<Vec<_>>()[1].clone();

    //println!("Uid Child: {}", nix::unistd::getuid());
    //println!("Pid Child: {}", nix::unistd::getpid());

    let cg = ThreadedCGroup::new("/sys/fs/cgroup", "process1").unwrap();
    let sys_time: Instant = TempFile::from_fd(get_fd(SYSTEM_TIME).unwrap())
        .read()
        .unwrap();
    debug!(
        target: &format!("Partition: {id}"),
        "Partition CGroup Type: {}",
        read_to_string(cg.path().parent().unwrap().join("cgroup.type")).unwrap()
    );
    debug!(
        target: &format!("Partition: {id}"),
        "Partition Child CGroup Type: {}",
        read_to_string(cg.path().join("cgroup.type")).unwrap()
    );

    let fds = Process::myself().unwrap().fd().unwrap();
    for f in fds.flatten() {
        trace!(
            target: &format!("Partition: {id}"),
            "Open File Descriptor: {f:#?}"
        )
    }

    loop {
        info!(
            target: &format!("Partition: {id}"),
            "Ping: {:?}",
            sys_time.elapsed()
        );
        //PartitionContext::send(15);
        //println!("{:?}", PartitionContext::recv());
        sleep(Duration::from_millis(500))
    }
}
