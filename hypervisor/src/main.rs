#[macro_use]
extern crate log;

use std::{fs::File, path::PathBuf};

use linux_apex_hypervisor::hypervisor::{config::Config, linux::Hypervisor};

use clap::Parser;

/// Hypervisor based on cgroups in Linux
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Args {
    /// Configuration file for the hypervisor
    #[clap()]
    config_file: PathBuf,

    /// Target cgroup to use
    #[clap(short = 'g', long)]
    cgroup: Option<PathBuf>,
}

fn main() {
    std::env::set_var(
        "RUST_LOG",
        std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
    );
    pretty_env_logger::init();

    trace!("parsing args");
    let mut args = Args::parse();

    let my_pid = procfs::process::Process::myself().unwrap();
    trace!("My pid is {}", my_pid.pid);

    // assumes cgroupv2
    let cgroups_mount_point = my_pid
        .mountinfo()
        .expect("unable to acquire mountinfo")
        .iter()
        .find(|m| m.mount_source == Some("cgroup2".into()))
        .expect("no cgroup2 mount found")
        .mount_point
        .clone();

    trace!("cgroups mount point is {cgroups_mount_point:?}");

    let cgroup = args.cgroup.get_or_insert_with(|| {
        let cgroups = my_pid
            .cgroups()
            .expect("unable to retrieve my parent cgroup");
        let cgroup_path = cgroups[0].pathname.strip_prefix('/').unwrap(); // this can't fail, the cgroup reported will always start with a leading '/'
        cgroups_mount_point.join(cgroup_path)
    });

    info!("parsing config");
    let f = File::open(args.config_file).unwrap();
    let mut config: Config = serde_yaml::from_reader(&f).unwrap();
    config.cgroup = cgroup.clone();

    trace!("{config:#?}");

    info!("launching hypervisor");
    Hypervisor::new(config).unwrap().run();
}
