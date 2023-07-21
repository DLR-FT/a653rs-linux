use a653rs::partition;
use a653rs::prelude::PartitionExt;
use a653rs_linux::partition::ApexLogger;
use log::LevelFilter;

fn main() {
    ApexLogger::install_panic_hook();
    ApexLogger::install_logger(LevelFilter::Trace).unwrap();

    sender::Partition.run()
}

#[partition(a653rs_linux::partition::ApexLinuxPartition)]
mod sender {
    use log::info;

    #[sampling_out(name = "crypto_api_req_p1"_p1, msg_size = "16MB")]
    struct CryptoReq;

    #[sampling_in(name = "crypto_api_resp_p1", msg_size = "16MB", refresh_period = "1s")]
    struct CryptoResp;

    #[start(cold)]
    fn cold_start(mut ctx: start::Context) {
        info!("initalize");
        // ctx.create_crypto_req().unwrap();
        // ctx.create_crypto_resp().unwrap();
        // ctx.create_sender_process().unwrap();
        ctx.create_dummy_process().unwrap();
        info!("init done");
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
    fn dummy_process(ctx: dummy_process::Context) {
        info!("does it work, ehh?!");
    }

    #[periodic(
        period = "0s",
        time_capacity = "Infinite",
        stack_size = "8KB",
        base_priority = 1,
        deadline = "Soft"
    )]
    fn sender_process(ctx: sender_process::Context) {
        info!("started process");
        let mut my_pk = vec![0u8; 0x1000000]; // 16 KiB
        let mut pk_initialized = false;
        loop {
            info!("enter loop");
            // request own pk if it is missing
            if !pk_initialized {
                info!("my_pk empty, requesting a new one");
                ctx.crypto_req.unwrap().send(&[1]).unwrap();
                ctx.periodic_wait().unwrap();
                let (_, received_msg) = ctx.crypto_resp.unwrap().receive(&mut my_pk).unwrap();

                // truncate my_pk to a suitable size
                let len = received_msg.len();
                my_pk.truncate(len);
                pk_initialized = true;
                info!("received public key, storing it in my_pk")
            }

            // send a message to other peer

            ctx.periodic_wait().unwrap();
        }
    }
}
