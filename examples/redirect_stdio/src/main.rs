use std::fs::OpenOptions;
use std::os::fd::AsRawFd;
use std::path::Path;

use a653rs::partition;
use a653rs::prelude::PartitionExt;
use a653rs_linux::partition::ApexLogger;
use anyhow::Result;
use log::LevelFilter;

fn replace_stdio<T: AsRawFd, U: AsRef<Path>>(stdio: T, new: U, write: bool) -> Result<()> {
    let new = OpenOptions::new()
        .write(write)
        .read(!write)
        .truncate(write)
        .open(new)?;
    nix::unistd::dup2(new.as_raw_fd(), stdio.as_raw_fd())?;
    Ok(())
}

fn main() {
    replace_stdio(std::io::stdin(), "/stdin", false).unwrap();
    replace_stdio(std::io::stdout(), "/stdout", true).unwrap();
    replace_stdio(std::io::stderr(), "/stderr", true).unwrap();

    ApexLogger::install_panic_hook();
    ApexLogger::install_logger(LevelFilter::Trace).unwrap();

    redirect_stdio::Partition.run()
}

#[partition(a653rs_linux::partition::ApexLinuxPartition)]
mod redirect_stdio {
    use log::info;
    use std::io::BufRead;

    #[start(cold)]
    fn cold_start(mut ctx: start::Context) {
        // create and start an aperiodic process
        ctx.create_process_0().unwrap().start().unwrap();
    }

    // do the same as a cold_start
    #[start(warm)]
    fn warm_start(ctx: start::Context) {
        cold_start(ctx);
    }

    // this aperiodic process opens /dev/random and reads some random bytes from it
    #[aperiodic(
        time_capacity = "Infinite",
        stack_size = "8KB",
        base_priority = 1,
        deadline = "Soft"
    )]
    fn process_0(ctx: process_0::Context) {
        info!("started process with redirected stdio/stdout/stderr");

        info!("Reading stdin to stdout");
        println!("Start reading stdin to stdout");
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            println!("{}", line.unwrap())
        }
        println!("Finished reading stdin to stdout");

        info!("Writing messages to stderr");
        eprintln!("Error was encountered: None");
        eprintln!("But it was printed to stderr");

        info!("Terminating partition");
        ctx.set_partition_mode(OperatingMode::Idle).unwrap();
    }
}
