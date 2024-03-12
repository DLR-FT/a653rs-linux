use a653rs::partition;
use a653rs::prelude::PartitionExt;
use a653rs_linux::partition::ApexLogger;

fn main() {
    ApexLogger::install_panic_hook();
    ApexLogger::install_logger(log::LevelFilter::Trace).unwrap();

    hello::Partition.run()
}

#[partition(a653rs_linux::partition::ApexLinuxPartition)]
mod hello {
    use core::time::Duration;
    use std::thread::sleep;

    use a653rs_postcard::prelude::*;
    use humantime::format_duration;
    use log::*;
    use serde::{Deserialize, Serialize};

    #[sampling_out(name = "Hello", msg_size = "10KB")]
    struct HelloSource;

    #[sampling_in(name = "Hello", msg_size = "10KB", refresh_period = "100ms")]
    struct HelloDestination;

    #[start(cold)]
    fn cold_start(mut ctx: start::Context) {
        // Get the partition ID, and based on that decide whether this becomes the
        // sender or the receiver partition
        let ident = ctx.get_partition_status().identifier;
        if ident == 0 {
            ctx.create_hello_source().unwrap();
        } else if ident == 1 {
            ctx.create_hello_destination().unwrap();
        }

        // create aperiodic process
        ctx.create_aperiodic().unwrap().start().unwrap();

        // create periodic process
        ctx.create_periodic().unwrap().start().unwrap();
    }

    #[start(warm)]
    fn warm_start(ctx: start::Context) {
        cold_start(ctx)
    }

    #[aperiodic(
        time_capacity = "Infinite",
        stack_size = "100KB",
        base_priority = 1,
        deadline = "Soft"
    )]
    fn aperiodic(ctx: aperiodic::Context) {
        info!("Start Aperiodic");
        for i in 0..i32::MAX {
            if let SystemTime::Normal(time) = ctx.get_time() {
                // round the time to an integer value of milliseconds
                let round = Duration::from_millis(time.as_millis() as u64);
                info!("{:?}: AP MSG {i}", format_duration(round).to_string());
            }
            sleep(Duration::from_millis(1))
        }
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct CustomMessage {
        msg: String,
        when: Duration,
    }

    #[periodic(
        period = "0ms",
        time_capacity = "Infinite",
        stack_size = "100KB",
        base_priority = 1,
        deadline = "Soft"
    )]
    fn periodic(ctx: periodic::Context) {
        let ident = ctx.get_partition_status().identifier;
        for i in 1..i32::MAX {
            if let SystemTime::Normal(time) = ctx.get_time() {
                let round = Duration::from_millis(time.as_millis() as u64);
                info!("{:?}: P MSG {i}", format_duration(round).to_string());
            }
            sleep(Duration::from_millis(1));

            if i % 5 == 0 {
                if ident == 0 {
                    ctx.hello_source
                        .unwrap()
                        .send_type(CustomMessage {
                            msg: format!("Sampling MSG {}", i / 5),
                            when: ctx.get_time().unwrap_duration(),
                        })
                        .ok()
                        .unwrap();
                } else if ident == 1 {
                    let (valid, data) = ctx
                        .hello_destination
                        .unwrap()
                        .recv_type::<CustomMessage>()
                        .ok()
                        .unwrap();

                    info!("Received via Sampling Port: {:?}, valid: {valid:?}", data)
                }

                ctx.periodic_wait().unwrap();
            }
        }
    }
}
