use std::fs::{File, OpenOptions};
use std::path::Path;

use a653rs::partition;
use a653rs::prelude::PartitionExt;
use a653rs_linux::partition::ApexLogger;
use anyhow::Result;
use log::LevelFilter;

fn stdio_fd<U: AsRef<Path>>(new: U, write: bool) -> Result<File> {
    Ok(OpenOptions::new()
        .write(write)
        .read(!write)
        .truncate(write)
        .open(new)?)
}

fn main() {
    let in_fd = stdio_fd("/stdin", false).unwrap();
    nix::unistd::dup2_stdin(in_fd).unwrap();
    let out_fd = stdio_fd("/stdout", true).unwrap();
    nix::unistd::dup2_stdout(out_fd).unwrap();
    let err_fd = stdio_fd("/stderr", true).unwrap();
    nix::unistd::dup2_stderr(err_fd).unwrap();

    ApexLogger::install_panic_hook();
    ApexLogger::install_logger(LevelFilter::Trace).unwrap();

    redirect_stdio::Partition.run()
}

#[partition(a653rs_linux::partition::ApexLinuxPartition)]
mod redirect_stdio {
    use std::io::BufRead;

    use log::info;

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
