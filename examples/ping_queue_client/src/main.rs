use a653rs::partition;
use a653rs::prelude::PartitionExt;
use log::LevelFilter;

use a653rs_linux::partition::ApexLogger;

fn main() {
    ApexLogger::install_panic_hook();
    ApexLogger::install_logger(LevelFilter::Trace).unwrap();

    ping_client::Partition.run()
}

#[partition(a653rs_linux::partition::ApexLinuxPartition)]
mod ping_client {
    use core::time::Duration;
    use log::{info, warn};

    #[queuing_out(
        name = "req_source",
        msg_size = "16B",
        msg_count = "10",
        discipline = "Fifo"
    )]
    struct PingRequest;

    #[queuing_in(
        name = "res_dest",
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
        ctx.create_periodic_ping_client().unwrap().start().unwrap();
    }

    // do the same as a cold_start
    #[start(warm)]
    fn warm_start(ctx: start::Context) {
        cold_start(ctx);
    }

    // this process requests a ping from the server at the beginning of each
    // partition window / MiF
    #[periodic(
        period = "0ms",
        time_capacity = "Infinite",
        stack_size = "8KB",
        base_priority = 1,
        deadline = "Soft"
    )]
    fn periodic_ping_client(ctx: periodic_ping_client::Context) {
        info!("started periodic_ping_client process");

        // a periodic process does not actually return at the end of a partition window,
        // it just pauses itself once it is done with the work from the current MiF
        // see below at the `ctx.periodic_wait().unwrap()` call.
        loop {
            // first, send a request:

            // `ctx.get_time()` returns a [SystemTime], which might be `Infinite`, or just a
            // normal time. Thus we have to check that indeed a normal time was returned.
            let SystemTime::Normal(time) = ctx.get_time() else {
                panic!("could not read time");
            };

            // convert the current time to an u128 integer representing nanoseconds, and
            // serialize the integer to a byte array

            let time_in_nanoseconds = time.as_nanos();
            let buf = time_in_nanoseconds.to_le_bytes();

            // finally send the byte array to the ping_request port
            ctx.ping_request
                .unwrap()
                .send(&buf, SystemTime::Normal(Duration::from_secs(10)))
                .unwrap();

            // then receive a response, if any:

            // allocate a buffer on the stack for receival of the response
            let mut buf = [0u8; 32];

            // sample the ping_response sampling port into `buf`
            // - validity indicates whether data received was sitting in the samplin port
            //   for no more than the refresh_period
            // - `bytes` is a subslice of `buf`, containing only the bytes actually read
            //   from the sampling port
            let (bytes, _) = ctx
                .ping_response
                .unwrap()
                .receive(&mut buf, SystemTime::Normal(Duration::from_secs(10)))
                .unwrap();

            if bytes.len() == 0 {
                continue;
            }

            // deserialize the bytes into an u128
            let request_timestamp = u128::from_le_bytes(bytes[0..16].try_into().unwrap());
            let response_timestamp = u128::from_le_bytes(bytes[16..32].try_into().unwrap());
            // the difference is the time passed since sending the request for this response
            let round_trip = time_in_nanoseconds - request_timestamp;
            let req_to_server = response_timestamp - request_timestamp;
            let resp_to_client = time_in_nanoseconds - response_timestamp;

            // convert the integers of nanoseconds back to a [Duration]s for nicer logging
            let req_sent_to_resp_recv = Duration::from_nanos(round_trip as u64);
            let req_sent_to_resp_sent = Duration::from_nanos(req_to_server as u64);
            let resp_sent_to_resp_recv = Duration::from_nanos(resp_to_client as u64);

            // and log the results!
            info!("received valid response:\n\t round-trip {req_sent_to_resp_recv:?}\n\treq-to-server {req_sent_to_resp_sent:?}\n\tresp-to-client{resp_sent_to_resp_recv:?}");

            // wait until the beginning of this partitions next MiF. In scheduling terms
            // this function would probably be called `yield()`.
            ctx.periodic_wait().unwrap();
        }
    }
}
