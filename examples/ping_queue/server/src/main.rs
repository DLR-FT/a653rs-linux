use a653rs::partition;
use a653rs::prelude::PartitionExt;
use a653rs_linux::partition::ApexLogger;
use log::LevelFilter;

fn main() {
    ApexLogger::install_panic_hook();
    ApexLogger::install_logger(LevelFilter::Trace).unwrap();

    ping_queue_server::Partition.run()
}

#[partition(a653rs_linux::partition::ApexLinuxPartition)]
mod ping_queue_server {
    use log::{info, warn};

    #[queuing_in(
        name = "req_dest",
        msg_size = "16B",
        msg_count = "10",
        discipline = "Fifo"
    )]
    struct PingRequest;

    #[queuing_out(
        name = "res_source",
        msg_size = "32B",
        msg_count = "10",
        discipline = "Fifo"
    )]
    struct PingResponse;

    #[start(cold)]
    fn cold_start(mut ctx: start::Context) {
        // initialize both queuing ports
        ctx.create_ping_request().unwrap();
        ctx.create_ping_response().unwrap();

        // create and start a periodic process
        ctx.create_periodic_ping_queue_server()
            .unwrap()
            .start()
            .unwrap();
    }

    // do the same as a cold_start
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
    fn periodic_ping_queue_server(ctx: periodic_ping_queue_server::Context) {
        info!("started ping_queue_server process");
        loop {
            // allocate a buffer to receive into
            let mut buf = [0u8; 32];

            // receive a request into `&mut buf`, and save the slice of actual received data
            // as `bytes`
            match ctx
                .ping_request
                .unwrap()
                .receive(&mut buf, SystemTime::Infinite)
            {
                Ok((_bytes, false)) => {
                    // `ctx.get_time()` returns a [SystemTime], which might be `Infinite`, or just a
                    // normal time. Thus we have to check that indeed a normal time was returned.
                    let SystemTime::Normal(time) = ctx.get_time() else {
                        panic!("could not read time");
                    };

                    // convert current time to bytes and store in upper 16 bytes of request
                    let time_in_nanoseconds = time.as_nanos();
                    buf[16..32].copy_from_slice(&time_in_nanoseconds.to_le_bytes());

                    info!("Forwarding request with timestamp as response");

                    // send the contents of `buf` back as response
                    match ctx.ping_response.unwrap().send(&buf, SystemTime::Infinite) {
                        Ok(_) => {}
                        Err(Error::NotAvailable) => warn!("Failed to send ping response"),
                        Err(other) => panic!("Failed to send ping response: {:?}", other),
                    }
                }
                Err(Error::NotAvailable) => warn!("Failed to receive ping request"),
                other => panic!("Failed to receive ping request: {:?}", other),
            }

            // wait until the next partition window / MiF
            ctx.periodic_wait().unwrap();
        }
    }
}
