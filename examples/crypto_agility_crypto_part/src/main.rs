use a653rs::partition;
use a653rs::prelude::PartitionExt;
use a653rs_linux::partition::ApexLogger;
use log::LevelFilter;

fn main() {
    ApexLogger::install_panic_hook();
    ApexLogger::install_logger(LevelFilter::Trace).unwrap();

    ping_client::Partition.run()
}

#[partition(a653rs_linux::partition::ApexLinuxPartition)]
mod ping_client {
    use log::{info, warn};

    #[sampling_in(name = "crypto_api_req_p1", msg_size = "16MB", refresh_period = "1s")]
    struct P1Req;

    #[sampling_out(name = "crypto_api_resp_p1", msg_size = "16MB")]
    struct P1Resp;

    #[sampling_in(name = "crypto_api_req_p2", msg_size = "16MB", refresh_period = "1s")]
    struct P2Req;

    #[sampling_out(name = "crypto_api_resp_p2", msg_size = "16MB")]
    struct P2Resp;

    #[start(cold)]
    fn cold_start(mut ctx: start::Context) {
        // initialize all ports
        ctx.create_p_1_req().unwrap();
        ctx.create_p_1_resp().unwrap();
        ctx.create_p_2_req().unwrap();
        ctx.create_p_2_resp().unwrap();

        // create and start a periodic process for each server
        ctx.create_p1_server().unwrap().start().unwrap();
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
    fn p1_server(ctx: p1_server::Context) {
        info!("started {} process", ctx.proc_self.id());

        // TODO make this heapless
        let mut buf: Vec<u8> = vec![0u8; 0x1000000];
        let _sk: Vec<u8> = Vec::new();
        let pk: Vec<u8> = Vec::new();

        // a periodic process does not actually return at the end of a partition window,
        // it just pauses itself once it is done with the work from the current MiF
        // see below at the `ctx.periodic_wait().unwrap()` call.
        loop {
            // first, recv a request (if any)
            let (validity, received_msg) = ctx.p_1_req.unwrap().receive(&mut buf).unwrap();

            // TODO improve this check
            if validity != Validity::Valid || received_msg.len() == 0 {
                warn!("received invalid request, skipping it");
                ctx.periodic_wait().unwrap();
                continue;
            }

            // AEAD: AES-GCM
            //   - https://docs.rs/aes-gcm
            //
            // KDF: KMAC-SHA3
            //   - https://docs.rs/tiny-keccak/latest/tiny_keccak/struct.Kmac.html
            //
            // Pre-quantum KEM: HPKE ecdh KEM
            //   - ???
            //
            // Post-quantum KEM: Kyber
            //   - https://docs.rs/pqc_kyber
            //
            // Post-quantum Signatures: Krystals-Kyber
            //   - ???
            match received_msg[0] {
                // setup
                0 => {
                    // setup() -> pk
                    //   sk_kem, pk_kem <- KEM.keygen()
                    //   sk_sig, pk_sig <- Signature.keygen()
                    //   sk <- concat(sk_kem, sk_sig)
                    //   return concat(pk_kem, pk_sig)

                    if pk.is_empty() {
                        //initizlize it
                    } else {
                        ctx.p_1_resp.unwrap().send(&pk).unwrap();
                    }
                }
                // encrypt
                1 => {
                    // encrypt(pk_peer, pt) -> ct
                    //   concat(sk_kem, sk_sig) <- sk
                    //   concat(pk_peer_kem, pk_peer_sig) <- pk_peer
                    //   base_shk, shk_ct <- KEM.encaps(pk_kem)
                    //   key_for_encryption, commitment <- KDF(base_shk , "My
                    // product name is beautiful thing")
                    //   sig <- Signature.sign(sk_sig, commitment)
                    //   ct <- AEAD.enc(shk_for_encryption, 0, null, pt)
                    //   return concat(shk_ct, sig, ct)
                }
                // decrypt
                2 => {
                    // decrypt(pk_peer, ct') -> pt
                    //   concat(sk_kem, sk_sig) <- sk
                    //   concat(pk_peer_kem, pk_peer_sig) <- pk_peer
                    //   concat(shk_ct, sig, ct) <- ct'
                    //   base_shk <- KEM.decaps(sk, shk_ct)
                    //   shk_for_encryption, commitment <- KDF(base_shk, "My
                    //   product name is beautiful thing") sig
                    //   <- Signature.validate(pk_peer_sig, commitment) # This
                    //   aborts pt <- AEAD.
                    //   dec(shk_for_encryption, 0, null, ct) # This abort
                    //   return pt
                }
                opcode => {
                    warn!("unknown opcode {opcode:02x} received, ignoring it");
                }
            }

            // wait until the beginning of this partitions next MiF. In scheduling terms
            // this function would probably be called `yield()`.
            ctx.periodic_wait().unwrap();
        }
    }
}
