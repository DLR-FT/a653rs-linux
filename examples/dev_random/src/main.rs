use a653rs::partition;
use a653rs::prelude::PartitionExt;
use a653rs_linux::partition::ApexLogger;
use log::LevelFilter;

fn main() {
    ApexLogger::install_panic_hook();
    ApexLogger::install_logger(LevelFilter::Trace).unwrap();

    dev_random::Partition.run()
}

#[partition(a653rs_linux::partition::ApexLinuxPartition)]
mod dev_random {
    use std::fs::*;
    use std::io::Read;

    use log::info;

    #[start(cold)]
    fn cold_start(mut ctx: start::Context) {
        // create and start an aperiodic process
        ctx.create_process_0().unwrap().start().unwrap();
    }

    // do the same as a cold_start
    #[start(warm)]
    fn warm_start(ctx: start::Context) {
        cold_start(ctx);
    }

    // this aperiodic process opens /dev/random and reads some random bytes from it
    #[aperiodic(
        time_capacity = "Infinite",
        stack_size = "8KB",
        base_priority = 1,
        deadline = "Soft"
    )]
    fn process_0(_: process_0::Context) {
        info!("started process_0");

        // open the device file and read its metadata
        let filename = "/dev/random";
        let mut f = File::open(filename).expect("no file found");
        let metadata = metadata(filename).expect("unable to read metadata");
        info!("metadata: {metadata:#?}");

        // read 16 bytes from the device
        let mut buffer = [0u8; 16];
        f.read_exact(&mut buffer).expect("buffer overflow");
        info!("got some randomness: {buffer:?}");

        info!("terminating this partitiong by setting the operating mode to idle");
        // TODO wait for https://github.com/DLR-FT/a653rs/issues/22 to be fixed
        // Hypervisor::set_partition_mode(OperatingMode::Idle);
    }
}
