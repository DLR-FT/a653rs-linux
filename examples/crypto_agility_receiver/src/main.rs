use a653rs::partition;
use a653rs::prelude::PartitionExt;
use a653rs_linux::partition::ApexLogger;
use log::LevelFilter;

fn main() {
    ApexLogger::install_panic_hook();
    ApexLogger::install_logger(LevelFilter::Trace).unwrap();

    receiver::Partition.run()
}

#[partition(a653rs_linux::partition::ApexLinuxPartition)]
mod receiver {
    use log::info;

    #[sampling_out(name = "crypto_api_req_p2", msg_size = "16MB")]
    struct P2Req;

    #[sampling_in(name = "crypto_api_resp_p2", msg_size = "16MB", refresh_period = "1s")]
    struct P2Resp;

    #[start(cold)]
    fn cold_start(mut ctx: start::Context) {
        info!("initialize");
        ctx.create_p_2_req().unwrap();
        ctx.create_p_2_resp().unwrap();
        info!("init done")
    }

    // do the same as a cold_start
    #[start(warm)]
    fn warm_start(ctx: start::Context) {
        cold_start(ctx);
    }
}
