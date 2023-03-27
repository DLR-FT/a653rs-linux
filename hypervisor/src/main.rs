#[macro_use]
extern crate log;

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use anyhow::anyhow;
use clap::Parser;
use linux_apex_core::cgroup;
use linux_apex_core::error::{ResultExt, SystemError, TypedResult};
use linux_apex_core::health::ModuleRecoveryAction;
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

    /// Only execute the hypervisor for this duration, then quit
    ///
    /// The condition is only checked in between major frames, e.g. a major
    /// frame is never interrupted.
    #[clap(short, long)]
    duration: Option<humantime::Duration>,
}

#[quit::main]
fn main() -> TypedResult<()> {
    let level = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into());
    std::env::set_var("RUST_LOG", level.clone());

    pretty_env_logger::formatted_builder()
        .parse_filters(&level)
        //.format(linux_apex_core::log_helper::format)
        .filter_module("polling", LevelFilter::Off)
        .format_timestamp_secs()
        .init();

    // Register Handler for SIGINT
    // Maybe use https://crates.io/crates/signal-hook instead
    let sig = SigAction::new(
        SigHandler::Handler(sighdlr),
        SaFlags::empty(),
        SigSet::empty(),
    );
    unsafe { sigaction(SIGINT, &sig) }.typ(SystemError::Panic)?;
    unsafe { sigaction(SIGTERM, &sig) }.typ(SystemError::Panic)?;

    trace!("parsing args");
    let mut args = Args::parse();

    let my_pid = procfs::process::Process::myself().typ(SystemError::Panic)?;
    trace!("My pid is {}", my_pid.pid);

    // assumes cgroupv2
    let cgroups_mount_point = cgroup::mount_point().typ(SystemError::CGroup)?;

    let cgroup = args.cgroup.get_or_insert_with(|| {
        let cgroups = my_pid
            .cgroups()
            .expect("unable to retrieve my parent cgroup");
        let cgroup_path = cgroups
            .iter()
            .filter(|c| c.hierarchy == 0)
            .next()
            .unwrap()
            .pathname
            .strip_prefix('/')
            .unwrap(); // this can't fail, the cgroup reported will always start with a leading '/'
        cgroups_mount_point.join(cgroup_path)
    });
    // Add Additional cgroup layer
    let cgroup = cgroup.join("linux-hypervisor");

    info!("parsing config");
    let f = File::open(args.config_file).typ(SystemError::Config)?;
    let mut config: Config = serde_yaml::from_reader(&f).typ(SystemError::Config)?;
    config.cgroup = cgroup;

    let terminate_after = args.duration.map(|d| d.into());

    loop {
        info!("Start Hypervisor");
        match Hypervisor::new(config.clone(), terminate_after)?.run() {
            Ok(_) => {
                return Err(anyhow!(
                    "Hypervisor Run is not supposed to exit with an OK variant"
                ))
                .typ(SystemError::Panic)
            }
            Err(e) => {
                let action = config
                    .hm_init_table
                    .try_action(e.err())
                    .unwrap_or(config.hm_run_table.panic);
                match action {
                    ModuleRecoveryAction::Ignore => {}
                    ModuleRecoveryAction::Shutdown => return Ok(()),
                    ModuleRecoveryAction::Reset => {}
                }
            }
        }
    }
}

pub extern "C" fn sighdlr(_: i32) {
    print!("\r");
    std::io::stdout().flush().unwrap();
    info!("Exiting");
    quit::with_code(0)
}
