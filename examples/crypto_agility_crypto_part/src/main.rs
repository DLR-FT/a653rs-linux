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
    use hpke::{Deserializable, Kem as KemTrait, OpModeR, OpModeS, Serializable};
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

    type Kem = hpke::kem::X25519HkdfSha256;
    type Aead = hpke::aead::ChaCha20Poly1305;
    type Kdf = hpke::kdf::HkdfSha384;

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
                    Duration::from_millis(500), // equal to partition period
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
        period = "0ms",
        time_capacity = "Infinite",
        stack_size = "8KB",
        base_priority = 1,
        deadline = "Soft"
    )]
    fn server(ctx: server::Context) {
        info!("started server process with id {}", ctx.proc_self.id());

        // TODO make this heapless
        let mut rx_buf: Vec<u8> = vec![0u8; 0x1000000];
        let mut sk_store = heapless::FnvIndexMap::<_, _, SLOTS>::new();
        let mut pk_store = heapless::FnvIndexMap::<_, _, SLOTS>::new();
        let mut rng = rand::thread_rng();
        let pk_size = <Kem as hpke::Kem>::PublicKey::size();
        let sk_size = <Kem as hpke::Kem>::PrivateKey::size();

        // a periodic process does not actually return at the end of a partition window,
        // it just pauses itself once it is done with the work from the current MiF
        // see below at the `ctx.periodic_wait().unwrap()` call.
        loop {
            for i in 0..SLOTS {
                debug!("processing slot {}", i + 1);
                let req_port = unsafe { &SAMPLING_DESTINATIONS[i] };
                let resp_port = unsafe { &SAMPLING_SOURCES[i] };

                // first, recv a request (if any)
                let (validity, received_msg) = req_port.receive(&mut rx_buf).unwrap();

                // second, check that the message is valid i.e. has to be processed
                if validity != Validity::Valid {
                    warn!("message validity flag indicates it is outdated, skipping it");
                    ctx.periodic_wait().unwrap();
                    continue;
                }

                if received_msg.is_empty() {
                    warn!("received empty request, skipping it");
                    ctx.periodic_wait().unwrap();
                    continue;
                }

                let info_str = b"ARINC 653 crypto partition example";
                match received_msg[0] {
                    // setup
                    0 => {
                        // setup() -> pk
                        //   sk_kem, pk_kem <- KEM.keygen()
                        //   sk_sig, pk_sig <- Signature.keygen()
                        //   sk <- concat(sk_kem, sk_sig)
                        //   return concat(pk_kem, pk_sig)

                        if pk_store.get(&i).is_none() {
                            info!("initializing key-slot {i}");

                            let (sk, pk) = Kem::gen_keypair(&mut rng);

                            pk_store.insert(i, pk).unwrap();
                            sk_store.insert(i, sk).unwrap();
                        }

                        let pk = pk_store.get(&i).unwrap();

                        resp_port.send(&pk.to_bytes()).unwrap();
                    }
                    // encrypt
                    // IN:
                    //   first byte msg type
                    //   n bytes receipient pk
                    //   rest of bytes is pt
                    // OUT:
                    //   ct
                    1 => {
                        // encrypt(pk_peer, pt) -> ct
                        //   concat(sk_kem, sk_sig) <- sk
                        //   concat(pk_peer_kem, pk_peer_sig) <- pk_peer
                        //   base_shk, shk_ct <- KEM.encaps(pk_kem)
                        //   key_for_encryption, commitment <- KDF(base_shk ,
                        // "My product name is beautiful
                        // thing")   sig <- Signature.
                        // sign(sk_sig, commitment)   ct
                        // <- AEAD.enc(shk_for_encryption, 0, null, pt)
                        //   return concat(shk_ct, sig, ct)

                        let Ok(pk_recip) = <Kem as hpke::Kem>::PublicKey::from_bytes(
                            &received_msg[1..1 + pk_size],
                        ) else {
                            warn!("could not extract public key");
                            continue;
                        };

                        let pt = &received_msg[1 + pk_size..];

                        let (shk_ct, tx_buf) = hpke::single_shot_seal::<Aead, Kdf, Kem, _>(
                            &OpModeS::Base,
                            &pk_recip,
                            info_str,
                            pt,
                            &[],
                            &mut rng,
                        )
                        .unwrap();

                        resp_port.send(&tx_buf).unwrap();
                    }
                    // decrypt
                    // IN:
                    //   first byte msg type
                    //   n bytes sender pk
                    //   rest of bytes ct
                    // OUT:
                    //   pt (if valid)
                    //   empty array (if invalid)
                    2 => {
                        // decrypt(pk_peer, ct') -> pt
                        //   concat(sk_kem, sk_sig) <- sk
                        //   concat(pk_peer_kem, pk_peer_sig) <- pk_peer
                        //   concat(shk_ct, sig, ct) <- ct'
                        //   base_shk <- KEM.decaps(sk, shk_ct)
                        //   shk_for_encryption, commitment <- KDF(base_shk, "My
                        //   product name is beautiful thing") sig
                        //   <- Signature.validate(pk_peer_sig, commitment) #
                        // This   aborts pt <- AEAD.
                        //   dec(shk_for_encryption, 0, null, ct) # This abort
                        //   return pt

                        let Ok(sk_recip) = <Kem as hpke::Kem>::PrivateKey::from_bytes(
                            &received_msg[1..1 + pk_size],
                        ) else {
                            warn!("could not extract secret key");
                            continue;
                        };

                        let ct = &received_msg[1..1 + pk_size];
                        let encapped_key = todo!();

                        let pt = hpke::single_shot_open::<Aead, Kdf, Kem>(
                            &OpModeR::Base,
                            &sk_recip,
                            encapped_key,
                            info_str,
                            ct,
                            &[],
                        );
                    }
                    opcode => {
                        warn!("unknown opcode {opcode:02x} received, ignoring it");
                    }
                }
            }

            // wait until the beginning of this partitions next MiF. In scheduling terms
            // this function would probably be called `yield()`.
            ctx.periodic_wait().unwrap();
        }
    }
}
