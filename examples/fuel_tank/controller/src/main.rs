// TODO Do something

use a653rs::partition;
use a653rs::prelude::PartitionExt;
use a653rs_linux::partition::ApexLogger;
use log::LevelFilter;

fn main() {
    ApexLogger::install_panic_hook();
    ApexLogger::install_logger(LevelFilter::Trace).unwrap();

    hello::Partition.run()
}

#[partition(a653rs_linux::partition::ApexLinuxPartition)]
mod hello {
    use a653rs::prelude::SystemTime;

    #[sampling_in(name = "fuel_sensors", msg_size = "10KB", refresh_period = "10s")]
    struct FuelSensors;

    #[sampling_out(name = "fuel_actuators", msg_size = "10KB")]
    struct FuelActuators;
    #[start(cold)]
    fn cold_start(mut ctx: start::Context) {
        ctx.create_periodic().unwrap().start().unwrap();
    }

    #[start(warm)]
    fn warm_start(ctx: start::Context) {
        cold_start(ctx)
    }

    #[periodic(
        period = "0ms",
        time_capacity = "Infinite",
        stack_size = "100KB",
        base_priority = 1,
        deadline = "Soft"
    )]
    fn periodic(ctx: periodic::Context) {
        loop {
            // Empty for now
            ctx.periodic_wait().unwrap();
        }
    }
}
