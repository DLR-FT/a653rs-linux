use a653rs::partition;
use a653rs::prelude::PartitionExt;
use a653rs_linux::partition::ApexLogger;
use log::LevelFilter;

fn main() {
    ApexLogger::install_panic_hook();
    ApexLogger::install_logger(LevelFilter::Trace).unwrap();

    crypto_partition::Partition.run()
}

#[partition(a653rs_linux::partition::ApexLinuxPartition)]
mod crypto_partition {
    use a653rs::prelude::*;
    use core::str::FromStr;
    use core::time::Duration;
    use crypto_agility_crypto_api::server::{example::ExampleEndpoint, CipherServer};
    use log::{debug, info, warn};

    /// Number of API stubs to generate
    const SLOTS: usize = 2;

    /// Maximum message size for an API stub, in this case 16 KiB
    const PORT_SIZE: u32 = 0x1000000;

    static mut SAMPLING_DESTINATIONS: heapless::Vec<
        SamplingPortDestination<PORT_SIZE, Hypervisor>,
        SLOTS,
    > = heapless::Vec::new();
    static mut SAMPLING_SOURCES: heapless::Vec<SamplingPortSource<PORT_SIZE, Hypervisor>, SLOTS> =
        heapless::Vec::new();
    #[start(cold)]
    fn cold_start(mut ctx: start::Context) {
        // initialize all endpoints
        for i in 1..=SLOTS {
            // initialize the request port
            let samp_req_name = Name::from_str(&format!("crypto_api_req_p{i}")).unwrap();
            let port = ctx
                .ctx
                .create_sampling_port_destination::<PORT_SIZE>(
                    samp_req_name,
                    Duration::from_millis(1000), // equal to partition period
                )
                .unwrap();
            unsafe {
                SAMPLING_DESTINATIONS.push(port).unwrap();
            }

            // initialize the response port
            let samp_resp_name = Name::from_str(&format!("crypto_api_resp_p{i}")).unwrap();
            let port = ctx
                .ctx
                .create_sampling_port_source::<PORT_SIZE>(samp_resp_name)
                .unwrap();
            unsafe {
                SAMPLING_SOURCES.push(port).unwrap();
            }
        }

        // create and start a periodic server process
        ctx.create_server().unwrap().start().unwrap();
    }

    // do the same as a cold_start
    #[start(warm)]
    fn warm_start(ctx: start::Context) {
        cold_start(ctx);
    }

    // this process requests a ping from the server at the beginning of each
    // partition window / MiF
    #[periodic(
        period = "500ms",
        time_capacity = "100ms",
        // crypto ops needs a lot of stack memory
        stack_size = "16KB",
        base_priority = 1,
        deadline = "Soft"
    )]
    fn server(ctx: server::Context) {
        info!("started server process with id {}", ctx.proc_self.id());

        let mut rx_buf: Vec<u8> = vec![0u8; PORT_SIZE as usize];
        // let salt = b"ARINC 653 crypto partition example";
        let salt = &[0, 1, 2, 3];
        let mut cipher_server = CipherServer::new();
        for i in 0..2 {
            cipher_server.insert_endpoint(i, ExampleEndpoint::new(salt))
        }

        // a periodic process does not actually return at the end of a it just pauses
        // itself once it is done see below at the `ctx.periodic_wait().unwrap()` call.
        loop {
            for i in 0..SLOTS {
                debug!("processing slot {}", i + 1);
                let req_port = unsafe { &SAMPLING_DESTINATIONS[i] };
                let resp_port = unsafe { &SAMPLING_SOURCES[i] };

                // first, recv a request (if any)
                let (validity, received_msg) = req_port.receive(&mut rx_buf).unwrap();

                // second, check that the message is valid i.e. has to be processed
                if validity != Validity::Valid {
                    debug!("message validity flag indicates it is outdated, skipping it");
                    continue;
                }

                if received_msg.is_empty() {
                    debug!("received empty request, skipping it");
                    continue;
                }

                match cipher_server.process_endpoint_request(i, received_msg) {
                    Ok(msg) => resp_port.send(&msg).unwrap(),
                    Err(err) => warn!("{err:?}"),
                };
            }

            // wait until the beginning of this partitions next MiF. In scheduling terms
            // this function would probably be called `yield()`.
            ctx.periodic_wait().unwrap();
        }
    }
}
