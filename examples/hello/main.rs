use std::{collections::HashSet, path::PathBuf, thread::sleep, time::Duration, fs::File};

use linux_apex::hypervisor::{
    config::{Config, Partition},
    linux::Hypervisor,
};

fn hello() {
    //println!("Uid Child: {}", nix::unistd::getuid());
    //println!("Pid Child: {}", nix::unistd::getpid());

    loop {
        println!("Hello");
        sleep(Duration::from_millis(500))
    }
}

fn world() {
    //println!("Uid Child: {}", nix::unistd::getuid());
    //println!("Pid Child: {}", nix::unistd::getpid());

    loop {
        println!("World");
        sleep(Duration::from_millis(500))
    }
}

fn main() {
    let root = "/sys/fs/cgroup/user.slice/user-125030.slice/user@125030.service/app.slice";
    let name = "linux-apex-root";

    let config = Config {
        major_frame: Duration::from_secs(1),
        cgroup_root: PathBuf::from(root),
        cgroup_name: name.to_string(),
        partitions: HashSet::from([
            Partition {
                name: "Foo".to_string(),
                duration: Duration::from_millis(500),
                offset: Duration::from_millis(0),
                entry: hello,
            },
            Partition{
                name: "Bar".to_string(),
                duration: Duration::from_millis(500),
                offset: Duration::from_millis(500),
                entry: world,
            }
        ]),
        channel: Default::default(),
        //channel: HashSet::from([Channel::Sampling(SamplingChannel{
        //    name: "Sampling1".to_string(),
        //    source: "Foo".to_string(),
        //    destination: HashSet::from(["Bar".to_string()]),
        //})]),
    };

    Hypervisor::new(config).unwrap().run();
}

