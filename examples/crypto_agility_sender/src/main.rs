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
    use log::{debug, info};

    #[sampling_out(name = "crypto_api_req_p1"_p1, msg_size = "16MB")]
    struct CryptoReq;

    #[sampling_in(name = "crypto_api_resp_p1", msg_size = "16MB", refresh_period = "1s")]
    struct CryptoResp;

    #[sampling_out(name = "comm_channel", msg_size = "2KB")]
    struct CommChannel;

    #[start(cold)]
    fn cold_start(mut ctx: start::Context) {
        info!("initalize");
        ctx.create_crypto_req().unwrap();
        ctx.create_crypto_resp().unwrap();
        ctx.create_comm_channel().unwrap();
        ctx.create_sender_process().unwrap().start().unwrap();
        info!("init done");
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
    fn sender_process(ctx: sender_process::Context) {
        info!("started process");
        let mut my_pk = vec![0u8; 0x1000000]; // 16 KiB
        let mut their_pk = vec![0u8; 0x1000000]; // 16 KiB
        let mut my_pk_initialized = false;
        let mut their_pk_initialized = false;
        let mut i = 0;
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
                let other_peer_idx: usize = 1; // the other peer goes by the id 1
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

            // send a message to other peer
            let very_secrete_msg = format!(
                "This message must not leak to the public! We sent {i} messages previously"
            );

            i += 1;

            // encrypt the message
            let mut tx_buf = vec![2];
            tx_buf.extend_from_slice(&their_pk);
            tx_buf.extend_from_slice(very_secrete_msg.as_bytes());
            info!("requested encryption of pt");
            ctx.crypto_req.unwrap().send(&tx_buf).unwrap();
            ctx.periodic_wait().unwrap();

            // send encrypted message to receiver partition
            tx_buf.clear();
            tx_buf.reserve(0x1000000);
            tx_buf.extend(core::iter::repeat(0).take(tx_buf.capacity() - tx_buf.len()));

            let (_, ct) = ctx.crypto_resp.unwrap().receive(&mut tx_buf).unwrap();
            info!("received ct, sending it to receiver partition");
            ctx.comm_channel.unwrap().send(&ct).unwrap();
        }
    }
}
