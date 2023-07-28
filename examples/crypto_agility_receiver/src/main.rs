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
    struct CryptoReq;

    #[sampling_in(name = "crypto_api_resp_p2", msg_size = "16MB", refresh_period = "1s")]
    struct CryptoResp;

    #[sampling_in(name = "comm_channel"_p1, msg_size = "2KB", refresh_period = "1s")]
    struct CommChannel;

    #[start(cold)]
    fn cold_start(mut ctx: start::Context) {
        info!("initialize");
        ctx.create_crypto_req().unwrap();
        ctx.create_crypto_resp().unwrap();
        ctx.create_comm_channel().unwrap();
        ctx.create_receiver_process().unwrap().start().unwrap();
        info!("init done")
    }

    // do the same as a cold_start
    #[start(warm)]
    fn warm_start(ctx: start::Context) {
        cold_start(ctx);
    }

    #[periodic(
        period = "0s",
        time_capacity = "Infinite",
        stack_size = "8KB",
        base_priority = 1,
        deadline = "Soft"
    )]
    fn receiver_process(ctx: receiver_process::Context) {
        info!("started process");
        let mut my_pk = vec![0u8; 0x1000000]; // 16 KiB
        let mut their_pk = vec![0u8; 0x1000000]; // 16 KiB
        let mut my_pk_initialized = false;
        let mut their_pk_initialized = false;
        let mut rx_buf = Vec::new();

        loop {
            info!("enter loop");
            // request own pk if it is missing
            if !my_pk_initialized {
                info!("my_pk empty, requesting a new one");
                ctx.crypto_req.unwrap().send(&[1]).unwrap();
                ctx.periodic_wait().unwrap();
                let (_, received_msg) = ctx.crypto_resp.unwrap().receive(&mut my_pk).unwrap();

                // truncate my_pk to a suitable size
                let len = received_msg.len();
                my_pk.truncate(len);
                my_pk_initialized = true;
                info!("received public key, storing it in my_pk")
            }

            // request other pk if it is missing
            if !their_pk_initialized {
                let mut req = vec![4];
                let other_peer_idx: usize = 0; // the other peer goes by the id 0
                req.extend_from_slice(&other_peer_idx.to_le_bytes());
                info!("their_pk empty, requesting a new one");
                ctx.crypto_req.unwrap().send(&req).unwrap();
                ctx.periodic_wait().unwrap();
                let (_, received_msg) = ctx.crypto_resp.unwrap().receive(&mut their_pk).unwrap();

                // truncate their_pk to a suitable size
                let len = received_msg.len();
                their_pk.truncate(len);
                their_pk_initialized = true;
                info!("received public key, storing it in their_pk")
            }

            // receive encrypted message from sender partition
            rx_buf.clear();
            rx_buf.reserve(0x1000000);
            rx_buf.push(3); // operation decrypt
            rx_buf.extend(core::iter::repeat(0).take(rx_buf.capacity() - rx_buf.len()));
            let (_, received_msg) = ctx.comm_channel.unwrap().receive(&mut rx_buf[1..]).unwrap();
            let received_msg_len = received_msg.len();

            // decrypt the message
            ctx.crypto_req
                .unwrap()
                .send(&rx_buf[..1 + received_msg_len])
                .unwrap();
            ctx.periodic_wait().unwrap();
            let (_, ct) = ctx.crypto_resp.unwrap().receive(&mut rx_buf).unwrap();

            info!("received a message:\n{:?}", std::str::from_utf8(ct));

            ctx.periodic_wait().unwrap();
        }
    }
}
