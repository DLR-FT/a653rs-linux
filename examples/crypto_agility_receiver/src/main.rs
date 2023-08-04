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
    use crypto_agility_crypto_api::client::{request::RequestBuilder, response::Response, Key};
    use log::info;

    #[sampling_out(name = "crypto_api_req_p2", msg_size = "16MB")]
    struct CryptoReq;

    #[sampling_in(name = "crypto_api_resp_p2", msg_size = "16MB", refresh_period = "1s")]
    struct CryptoResp;

    #[sampling_in(name = "comm_channel", msg_size = "1MB", refresh_period = "1s")]
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
        stack_size = "64MB",
        base_priority = 1,
        deadline = "Soft"
    )]
    fn receiver_process(ctx: receiver_process::Context) {
        info!("started process");
        let mut builder_buffer = [0u8; 0x1000000];
        let mut request_builder = RequestBuilder::new(&mut builder_buffer);
        let other_peer_idx: u32 = 0; // the other peer goes by the id 1
        let their_pk = receive_their_public_key(&ctx, other_peer_idx, &mut request_builder);
        info!("received public key, storing it in their_pk");
        let mut receive_buffer = [0u8; 0x100000];

        loop {
            info!("enter loop");

            // receive encrypted message from sender partition
            let encrypted_message = receive_message(&ctx, &mut receive_buffer);

            // decrypt the message
            let msg = decrypt_message(&ctx, &their_pk, encrypted_message, &mut request_builder);

            info!("received a message:\n{:?}", std::str::from_utf8(msg));

            ctx.periodic_wait().unwrap();
        }
    }

    fn receive_their_public_key(
        ctx: &receiver_process::Context,
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

    fn receive_message<'a>(ctx: &receiver_process::Context, buffer: &'a mut [u8]) -> &'a [u8] {
        loop {
            let (validity, msg) = ctx.comm_channel.unwrap().receive(buffer).unwrap();
            if validity == Validity::Invalid {
                ctx.periodic_wait().unwrap();
                continue;
            }
            let len = msg.len();
            return &buffer[..len];
        }
    }

    fn decrypt_message<'a>(
        ctx: &receiver_process::Context,
        their_key: &Key<0x10000>,
        encrypted_msg: &[u8],
        builder: &'a mut RequestBuilder,
    ) -> &'a [u8] {
        let additional_data = b"This data is identical between sender and receiver";
        let request = builder
            .build_decrypt_request(their_key, encrypted_msg, additional_data)
            .unwrap();
        ctx.crypto_req.unwrap().send(request).unwrap();
        loop {
            let (validity, resp) = ctx.crypto_resp.unwrap().receive(builder).unwrap();
            if validity == Validity::Invalid {
                ctx.periodic_wait().unwrap();
                continue;
            }
            let len = resp.len();
            let resp = Response::try_from(&builder[..len]).unwrap();
            if let Response::DecryptedMessage(message) = resp {
                return message;
            } else {
                panic!("{resp:?}")
            }
        }
    }
}
