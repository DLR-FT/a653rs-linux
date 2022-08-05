#[macro_use]
extern crate log;

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use clap::Parser;
use linux_apex_core::cgroup::CGroup;
use linux_apex_core::partition::NAME_ENV;
use linux_apex_hypervisor::hypervisor::config::Config;
use linux_apex_hypervisor::hypervisor::linux::Hypervisor;
use log::LevelFilter;
use nix::sys::signal::*;

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
    log_panics::init();

    let level = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into());
    std::env::set_var("RUST_LOG", level.clone());
    std::env::set_var(NAME_ENV, "Hypervisor");

    pretty_env_logger::formatted_builder()
        .parse_filters(&level)
        .format(linux_apex_core::log_helper::format)
        .filter_module("polling", LevelFilter::Off)
        .format_timestamp_secs()
        .init();

    // Register Handler for SIGINT
    let sig_action = SigAction::new(
        SigHandler::Handler(unwind),
        SaFlags::empty(),
        SigSet::empty(),
    );
    unsafe { sigaction(SIGINT, &sig_action) }.unwrap();

    trace!("parsing args");
    let mut args = Args::parse();

    let my_pid = procfs::process::Process::myself().unwrap();
    trace!("My pid is {}", my_pid.pid);

    // assumes cgroupv2
    let cgroups_mount_point = CGroup::mount_point().unwrap();

    let cgroup = args.cgroup.get_or_insert_with(|| {
        let cgroups = my_pid
            .cgroups()
            .expect("unable to retrieve my parent cgroup");
        let cgroup_path = cgroups[0].pathname.strip_prefix('/').unwrap(); // this can't fail, the cgroup reported will always start with a leading '/'
        cgroups_mount_point.join(cgroup_path)
    });
    // Add Additional cgroup layer
    let cgroup = cgroup.join("linux-hypervisor");

    info!("parsing config");
    let f = File::open(args.config_file).unwrap();
    let mut config: Config = serde_yaml::from_reader(&f).unwrap();
    config.cgroup = cgroup;

    //trace!("{config:#?}");

    info!("launching hypervisor");
    Hypervisor::new(config).unwrap().run();
}

extern "C" fn unwind(_: i32) {
    print!("\r");
    std::io::stdout().flush().unwrap();
    info!("Exiting");
    quit::with_code(0)
}
