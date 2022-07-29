use std::{fs::read_to_string, thread::sleep, time::Duration};

use linux_apex_core::cgroup::ThreadedCGroup;
use procfs::process::Process;

fn main() {
    let id = std::env::args().collect::<Vec<_>>()[1].clone();

    //println!("Uid Child: {}", nix::unistd::getuid());
    //println!("Pid Child: {}", nix::unistd::getpid());

    let cg = ThreadedCGroup::new("/sys/fs/cgroup", "process1").unwrap();
    println!("{}", read_to_string(cg.path().join("cgroup.type")).unwrap());
    println!(
        "{}",
        read_to_string(cg.path().parent().unwrap().join("cgroup.type")).unwrap()
    );

    let fds = Process::myself().unwrap().fd().unwrap();
    for f in fds.flatten() {
        println!("{f:#?}")
    }

    loop {
        println!("Ping: {id}");
        //PartitionContext::send(15);
        //println!("{:?}", PartitionContext::recv());
        sleep(Duration::from_millis(500))
    }
}
