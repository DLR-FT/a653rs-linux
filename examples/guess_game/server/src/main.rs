use a653rs::partition;
use a653rs::prelude::PartitionExt;
use a653rs_linux::partition::ApexLogger;
use log::LevelFilter;

fn main() {
    ApexLogger::install_panic_hook();
    ApexLogger::install_logger(LevelFilter::Trace).unwrap();

    redirect_stdio::Partition.run()
}

#[partition(a653rs_linux::partition::ApexLinuxPartition)]
mod redirect_stdio {
    use std::cmp::Ordering;

    use rand::Rng;

    impl<'a> process_0::Context<'a> {
        fn get_next_number(&self) -> i32 {
            let mut buf = [0; 4];
            while self
                .request_port
                .unwrap()
                .receive(&mut buf, SystemTime::Infinite)
                .is_err()
            {}
            i32::from_ne_bytes(buf)
        }

        fn send_response(&self, ord: Ordering) {
            self.response_port
                .unwrap()
                .send(&[ord as i8 as u8], SystemTime::Infinite)
                .unwrap();
        }
    }

    #[queuing_in(
        name = "req_dest",
        msg_size = "4B",
        msg_count = "1",
        discipline = "FIFO"
    )]
    struct RequestPort;

    #[queuing_out(
        name = "resp_src",
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

    // this aperiodic process opens /dev/random and reads some random bytes from it
    #[aperiodic(
        time_capacity = "Infinite",
        stack_size = "8KB",
        base_priority = 1,
        deadline = "Soft"
    )]
    fn process_0(ctx: process_0::Context) {
        let random_number: i32 = rand::thread_rng().gen_range(0..100);

        loop {
            let req = ctx.get_next_number();
            log::warn!("Got {req} as number");
        }
        ctx.send_response(Ordering::Greater);
    }
}
