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
    use a653rs::prelude::Validity;
    use crypto_agility_crypto_api::client::{request::RequestBuilder, response::Response, Key};
    use log::{error, info};

    #[sampling_out(name = "crypto_api_req_p1"_p1, msg_size = "16MB")]
    struct CryptoReq;

    #[sampling_in(name = "crypto_api_resp_p1", msg_size = "16MB", refresh_period = "1s")]
    struct CryptoResp;

    #[sampling_out(name = "comm_channel", msg_size = "1MB")]
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
        stack_size = "32MB",
        base_priority = 1,
        deadline = "Soft"
    )]
    fn sender_process(ctx: sender_process::Context) {
        info!("started process");
        let mut builder_buffer = [0u8; 0x1000000];
        let mut request_builder = RequestBuilder::new(&mut builder_buffer);
        let other_peer_idx: u32 = 1; // the other peer goes by the id 1
        let their_pk = receive_their_public_key(&ctx, other_peer_idx, &mut request_builder);
        info!("received public key, storing it in their_pk");
        let mut i = 0;
        loop {
            info!("enter loop");
            // send a message to other peer
            let very_secrete_msg = format!(
                "This message must not leak to the public! We sent {i} messages previously"
            );

            i += 1;

            // encrypt the message
            let encrypted_message =
                encrypt_message(&ctx, &their_pk, &very_secrete_msg, &mut request_builder);
            info!("received ct, sending it to receiver partition");
            ctx.comm_channel.unwrap().send(encrypted_message).unwrap();
        }
    }

    fn receive_their_public_key(
        ctx: &sender_process::Context,
        their_id: u32,
        builder: &mut RequestBuilder,
    ) -> Key<0x10000> {
        let request = builder.build_peer_public_key_request(their_id).unwrap();
        ctx.crypto_req.unwrap().send(request).unwrap();
        info!("requested encryption of pt");
        loop {
            ctx.periodic_wait().unwrap();
            let (validity, resp) = ctx.crypto_resp.unwrap().receive(builder).unwrap();
            if validity == Validity::Invalid {
                continue;
            }
            let resp = Response::try_from(resp).unwrap();
            if let Response::PeerPublicKey(key) = resp {
                return key.into_key().unwrap();
            } else {
                panic!("{resp:?}")
            }
        }
    }

    fn encrypt_message<'a>(
        ctx: &sender_process::Context,
        their_key: &Key<0x10000>,
        msg: &str,
        builder: &'a mut RequestBuilder,
    ) -> &'a [u8] {
        let additional_data = b"This data is identical between sender and receiver";
        let request = builder
            .build_encryption_request(their_key, msg.as_bytes(), additional_data)
            .unwrap();
        ctx.crypto_req.unwrap().send(request).unwrap();
        loop {
            ctx.periodic_wait().unwrap();
            let (validity, resp) = ctx.crypto_resp.unwrap().receive(builder).unwrap();
            if validity == Validity::Invalid {
                continue;
            }
            let len = resp.len();
            let resp = Response::try_from(&builder[..len]).unwrap();
            if let Response::EncryptedMessage(msg) = resp {
                return msg;
            } else {
                panic!("{resp:?}")
            }
        }
    }
}
