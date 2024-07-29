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

    ApexLogger::install_panic_hook();
    ApexLogger::install_logger(LevelFilter::Trace).unwrap();

    guess_game::Partition.run()
}

#[partition(a653rs_linux::partition::ApexLinuxPartition)]
mod guess_game {
    use std::cmp::Ordering;

    impl<'a> process_0::Context<'a> {
        fn read_user_input(&self) -> i32 {
            let stdin = std::io::stdin();
            let input = &mut String::new();

            input.clear();
            while stdin.read_line(input).unwrap() == 0 {}
            input.trim_end().parse().unwrap()
        }

        fn send_request(&self, num: i32) {
            let bytes = num.to_ne_bytes();
            self.request_port
                .unwrap()
                .send(&bytes, SystemTime::Infinite)
                .unwrap()
        }

        fn get_response(&self) -> Ordering {
            let mut buf = [0; 1];
            while self
                .response_port
                .unwrap()
                .receive(&mut buf, SystemTime::Infinite)
                .is_err()
            {}
            unsafe { std::mem::transmute(buf[0] as i8) }
        }
    }

    #[queuing_out(
        name = "req_src",
        msg_size = "4B",
        msg_count = "1",
        discipline = "FIFO"
    )]
    struct RequestPort;

    #[queuing_in(
        name = "resp_dest",
        msg_size = "1B",
        msg_count = "1",
        discipline = "FIFO"
    )]
    struct ResponsePort;

    #[start(cold)]
    fn cold_start(mut ctx: start::Context) {
        // initialize ports
        ctx.create_request_port().unwrap();
        ctx.create_response_port().unwrap();
        // create and start an aperiodic process
        ctx.create_process_0().unwrap().start().unwrap();
    }

    // do the same as a cold_start
    #[start(warm)]
    fn warm_start(ctx: start::Context) {
        cold_start(ctx);
    }

    #[aperiodic(
        time_capacity = "Infinite",
        stack_size = "8KB",
        base_priority = 1,
        deadline = "Soft"
    )]
    fn process_0(ctx: process_0::Context) {
        loop {
            let user_input = ctx.read_user_input();
            ctx.send_request(user_input);
        }
        let resp = ctx.get_response();
        println!("Got {resp:?} as ordering");
    }
}
