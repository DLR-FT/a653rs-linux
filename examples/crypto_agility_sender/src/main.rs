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
    struct P1Req;

    #[sampling_in(name = "crypto_api_resp_p1", msg_size = "16MB", refresh_period = "1s")]
    struct P1Resp;

    #[start(cold)]
    fn cold_start(mut ctx: start::Context) {
        info!("initalize");
        ctx.create_p_1_req().unwrap();
        ctx.create_p_1_resp().unwrap();
        ctx.create_sender_process().unwrap();
        info!("init done");
    }

    // do the same as a cold_start
    #[start(warm)]
    fn warm_start(ctx: start::Context) {
        cold_start(ctx);
    }

    #[periodic(
        period = "0ms",
        time_capacity = "Infinite",
        stack_size = "8KB",
        base_priority = 1,
        deadline = "Soft"
    )]
    fn sender_process(ctx: sender_process::Context) {
        loop {
            ctx.periodic_wait().unwrap();
        }
    }
}
