use std::{path::PathBuf, time::Duration};

use linux_apex_hypervisor::hypervisor::{
    config::{Config, Partition},
    linux::Hypervisor,
};

fn main() {
    let cgroup =
        "/sys/fs/cgroup/user.slice/user-125030.slice/user@125030.service/app.slice/linux-apex-root";

    let bin_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target/x86_64-unknown-linux-musl/release/hello_part");
    if !bin_path.exists() {
        panic!("\nNo partition binary!\nMake sure to run \"cargo build -p hello_part --release --target x86_64-unknown-linux-musl\" first!\n");
    }
    //println!("{}", env!("CARGO_BIN_FILE_HELLO_PART"));
    let config = Config {
        major_frame: Duration::from_secs(1),
        cgroup: PathBuf::from(cgroup),
        partitions: vec![
            Partition {
                name: "Foo".to_string(),
                duration: Duration::from_millis(500),
                offset: Duration::from_millis(0),
                bin: bin_path.clone(),
            },
            Partition {
                name: "Bar".to_string(),
                duration: Duration::from_millis(500),
                offset: Duration::from_millis(500),
                bin: bin_path,
            },
        ],
        channel: Default::default(),
        //channel: HashSet::from([Channel::Sampling(SamplingChannel{
        //    name: "Sampling1".to_string(),
        //    source: "Foo".to_string(),
        //    destination: HashSet::from(["Bar".to_string()]),
        //})]),
    };

    Hypervisor::new(config).unwrap().run();
}
