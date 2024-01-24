use a653rs::partition;
use a653rs::prelude::PartitionExt;
use a653rs_linux::partition::ApexLogger;
use log::LevelFilter;

fn main() {
    ApexLogger::install_panic_hook();
    ApexLogger::install_logger(LevelFilter::Trace).unwrap();

    ping_server::Partition.run()
}

#[partition(a653rs_linux::partition::ApexLinuxPartition)]
mod ping_server {
    use log::{info, warn};

    #[sampling_in(name = "ping_request", msg_size = "16B", refresh_period = "10s")]
    struct PingRequest;

    #[sampling_out(name = "ping_response", msg_size = "32B")]
    struct PingResponse;

    #[start(cold)]
    fn cold_start(mut ctx: start::Context) {
        // intialize the request destination port
        ctx.create_ping_request().unwrap();

        // intialize the response source port
        ctx.create_ping_response().unwrap();

        // launch the periodic process
        ctx.create_periodic_ping_server().unwrap().start().unwrap();
    }

    #[start(warm)]
    fn warm_start(ctx: start::Context) {
        cold_start(ctx);
    }

    // the server process is super simple; all it does is receive a request and
    // respond to it
    #[periodic(
        period = "0ms",
        time_capacity = "Infinite",
        stack_size = "8KB",
        base_priority = 1,
        deadline = "Soft"
    )]
    fn periodic_ping_server(ctx: periodic_ping_server::Context) {
        info!("started ping_server process");
        loop {
            info!("forwarding request as response ");

            // allocate a buffer to receive into
            let mut buf = [0u8; 32];

            // receive a request, storing it to `buf`
            if let Err(e) = ctx.ping_request.unwrap().receive(&mut buf) {
                warn!("Failed to receive ping request: {e:?}");
                continue;
            }

            // `ctx.get_time()` returns a [SystemTime], which might be `Infinite`, or just a
            // normal time. Thus we have to check that indeed a normal time was returned.
            let SystemTime::Normal(time) = ctx.get_time() else {
                panic!("could not read time");
            };

            // convert the current time to an u128 integer representing nanoseconds, and
            // serialize the integer to a byte array
            let time_in_nanoseconds = time.as_nanos();
            buf[16..32].copy_from_slice(&time_in_nanoseconds.to_le_bytes());

            // send the contents of `buf` back as response
            ctx.ping_response.unwrap().send(&buf).unwrap();

            // wait until the next partition window / MiF
            ctx.periodic_wait().unwrap();
        }
    }
}
