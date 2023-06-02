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
    use core::time::Duration;
    use std::thread::sleep;

    use a653rs::prelude::SystemTime;
    use a653rs_postcard::prelude::*;
    use humantime::format_duration;
    use log::*;
    use serde::{Deserialize, Serialize};

    #[sampling_in(name = "fuel_actuators", msg_size = "10KB", refresh_period = "20ms")]
    struct FuelActuators;

    #[sampling_out(name = "fuel_sensors", msg_size = "10KB")]
    struct FuelSensors;

    #[start(cold)]
    fn cold_start(mut ctx: start::Context) {
        ctx.create_fuel_actuators().unwrap();
        ctx.create_fuel_sensors().unwrap();
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
        info!("Start Aperiodic");

        // TODO implement cascading flow, filling one f32 takes from a slice of f32, starting from the right most element in the slice. After consumption move all fuel as far as possible to the left.

        loop {
            // Step 1: get control commands

            // Step 2: apply control commands

            // Step 3: advance the simulation by one tick

            // Step 4: check for errors

            // Step 5: take sensor measurements

            // wait until next slot
            ctx.periodic_wait().unwrap();
        }
    }
}
