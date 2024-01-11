//! Hypervisor side of the Linux based ARINC 653 hypervisor

#[macro_use]
extern crate log;

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use a653rs_linux_core::cgroup;
use a653rs_linux_core::error::{ErrorLevel, LeveledResult, ResultExt, SystemError, TypedResultExt};
use a653rs_linux_core::health::ModuleRecoveryAction;
use anyhow::anyhow;
use clap::Parser;
use hypervisor::config::Config;
use hypervisor::linux::Hypervisor;
use nix::sys::signal::*;

pub mod hypervisor;

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

/// Hypervisor entrypoint
pub fn run_hypervisor() -> LeveledResult<()> {
    // Register Handler for SIGINT
    // Maybe use https://crates.io/crates/signal-hook instead
    let sig = SigAction::new(
        SigHandler::Handler(sighdlr),
        SaFlags::empty(),
        SigSet::empty(),
    );
    unsafe { sigaction(SIGINT, &sig) }.lev_typ(SystemError::Panic, ErrorLevel::ModuleInit)?;
    unsafe { sigaction(SIGTERM, &sig) }.lev_typ(SystemError::Panic, ErrorLevel::ModuleInit)?;

    trace!("parsing args");
    let mut args = Args::parse();

    let my_pid =
        procfs::process::Process::myself().lev_typ(SystemError::Panic, ErrorLevel::ModuleInit)?;
    trace!("My pid is {}", my_pid.pid);

    // assumes cgroupv2
    let cgroups_mount_point = cgroup::mount_point()
        .typ(SystemError::CGroup)
        .lev(ErrorLevel::ModuleInit)?;

    let cgroup = args.cgroup.get_or_insert_with(|| {
        let cgroups = my_pid
            .cgroups()
            .expect("unable to retrieve my parent cgroup");
        let cgroup_path = cgroups
            .iter()
            .find(|c| c.hierarchy == 0)
            .unwrap()
            .pathname
            .strip_prefix('/')
            .unwrap(); // this can't fail, the cgroup reported will always start with a leading '/'
        cgroups_mount_point.join(cgroup_path)
    });
    // Add Additional cgroup layer
    let cgroup = cgroup.join("linux-hypervisor");

    info!("parsing config");
    let f = File::open(args.config_file).lev_typ(SystemError::Config, ErrorLevel::ModuleInit)?;
    let mut config: Config =
        serde_yaml::from_reader(&f).lev_typ(SystemError::Config, ErrorLevel::ModuleInit)?;
    config.cgroup = cgroup;

    let terminate_after = args.duration.map(|d| d.into());

    loop {
        info!("Start Hypervisor");
        match Hypervisor::new(config.clone(), terminate_after)?.run() {
            Ok(_) => {
                return Err(anyhow!(
                    "Hypervisor Run is not supposed to exit with an OK variant"
                ))
                .lev_typ(SystemError::Panic, ErrorLevel::ModuleRun)
            }
            Err(e) => {
                let action = match e.level() {
                    // Partition Level is not expected here
                    ErrorLevel::Partition => return Err(e),
                    ErrorLevel::ModuleInit => config
                        .hm_init_table
                        .try_action(e.err())
                        .unwrap_or(config.hm_init_table.panic),
                    ErrorLevel::ModuleRun => config
                        .hm_run_table
                        .try_action(e.err())
                        .unwrap_or(config.hm_run_table.panic),
                };
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

/// Shorthand macro to return a new
/// [`TypedError`](a653rs_linux_core::error::TypedError)
///
/// Allows expressing
///
/// ```no_run
/// # use anyhow::anyhow;
/// # use a653rs_linux_core::error::{TypedError, TypedResult, SystemError};
/// # fn main() -> TypedResult<()>{
/// let extra_info = "problem";
/// let problem = anyhow!("a {extra_info} description");
/// return Err(TypedError::new(SystemError::Panic, problem));
/// # }
/// ```
///
/// as a more compact
///
/// ```no_run
/// # use a653rs_linux_core::error::TypedResult;
/// # use a653rs_linux_hypervisor::problem;
/// # fn main() -> TypedResult<()>{
/// # let extra_info = "problem";
/// problem!(Panic, "a {extra_info} description");
/// # }
/// ```
#[macro_export]
macro_rules! problem {
    ($typed_err: expr, $($tail:tt)*) => {{
        #[allow(unused_imports)]
        use ::a653rs_linux_core::error::SystemError::*;
        let problem = ::anyhow::anyhow!($($tail)*);
        return ::a653rs_linux_core::error::TypedResult::Err(
            ::a653rs_linux_core::error::TypedError::new($typed_err, problem)
        );
    }};
}

#[cfg(test)]
mod test {
    use a653rs_linux_core::error::{SystemError, TypedError, TypedResult};
    use anyhow::anyhow;

    fn problem_manual() -> TypedResult<()> {
        let extra_info = "problem";
        let problem = anyhow!("a {extra_info} description");
        Err(TypedError::new(SystemError::Panic, problem))
    }

    fn problem_macro() -> TypedResult<()> {
        let extra_info = "problem";
        problem!(Panic, "a {extra_info} description");
    }

    #[test]
    fn problem() {
        assert_eq!(
            problem_manual().unwrap_err().to_string(),
            problem_macro().unwrap_err().to_string()
        );
    }
}
