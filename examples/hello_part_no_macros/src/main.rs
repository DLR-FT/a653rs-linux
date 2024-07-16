//! # Example `hello_part_no_macros`
//!
//! ## FAQ
//!
//! - What is [`StartContext<Hypervisor>`]?
//!   - Some parts of the ARINC 653 API are only available on a specific mode.
//!     In particular, the process and port creationg API can only be called
//!     during {`COLD`,`WARM`}`_START` mode (see [`OperatingMode`]).
//!     `StartContext` is a struct that exposes this API only during
//!     {`COLD`,`WARM`}`_START` mode, and can not be constructed by the user.
//!     This makes misuse of the API (as in attempting to create a port/process
//!     during `NORMAL`) impossible.

use core::str::FromStr;
use core::time::Duration;

use a653rs::bindings::ApexPartitionP4;
use a653rs::prelude::*;
use a653rs_linux::partition::ApexLogger;
use a653rs_postcard::sampling::{SamplingPortDestinationExt, SamplingPortSourceExt};
use humantime::format_duration;
use log::info;
use once_cell::sync::OnceCell;

fn main() {
    ApexLogger::install_panic_hook();
    ApexLogger::install_logger(log::LevelFilter::Trace).unwrap();

    HelloPartition.run()
}

/// Alias the hypervisor in use. This is the handle for all further ARINC 653
/// services/API calls.
type Hypervisor = a653rs_linux::partition::ApexLinuxPartition;

/// Union struct for our partition
pub struct HelloPartition;

// Ports are initialized in in the {cold,warm}_start functions, but are used in
// the processes. We us static variables to pass them over. However, `static
// mut` requires `unsafe`, hence we use a `OnceCell`.
static SOURCE_PORT: OnceCell<SamplingPortSource<Hypervisor>> = OnceCell::new();
static DESTINATION_PORT: OnceCell<SamplingPortDestination<Hypervisor>> = OnceCell::new();

// Implements the Partition trait for the given hypervisor, a653rs-linux in this
// case
impl a653rs::prelude::Partition<Hypervisor> for HelloPartition {
    // cold start function, as defined by ARINC 653
    fn cold_start(&self, ctx: &mut a653rs::prelude::StartContext<Hypervisor>) {
        // Get the partition ID, and based on that decide whether this becomes the
        // sender or the receiver partition
        let ident = Hypervisor::get_partition_status().identifier;
        if ident == 0 {
            // create port name (which can fail if the string is too long)
            let port_name = Name::from_str("Hello").unwrap();
            // create port (which can fail if the port is not configured on the hypervisor)
            let port = ctx.create_sampling_port_source(port_name, 10_000).unwrap();
            // store port in static var for later retrieval by the processes
            SOURCE_PORT.set(port).unwrap();
        } else if ident == 1 {
            let port_name = Name::from_str("Hello").unwrap();
            let port = ctx
                .create_sampling_port_destination(port_name, 10_000, Duration::from_secs(1_000))
                .unwrap();
            DESTINATION_PORT.set(port).unwrap();
        }

        // create aperiodic process
        let process_attributes = ProcessAttribute {
            period: a653rs::prelude::SystemTime::Infinite,
            time_capacity: SystemTime::Infinite,
            entry_point: aperiodic,
            stack_size: 100_000,
            base_priority: 1,
            deadline: Deadline::Soft,
            name: Name::from_str("aperiodic").unwrap(),
        };
        let process_handle = ctx.create_process(process_attributes).unwrap();
        process_handle.start().unwrap();

        // create periodic process
        let process_attributes = ProcessAttribute {
            period: 0.into(),
            time_capacity: SystemTime::Infinite,
            entry_point: periodic,
            stack_size: 100_000,
            base_priority: 1,
            deadline: Deadline::Soft,
            name: Name::from_str("periodic").unwrap(),
        };
        let process_handle = ctx.create_process(process_attributes).unwrap();
        process_handle.start().unwrap();
    }

    fn warm_start(&self, ctx: &mut a653rs::prelude::StartContext<Hypervisor>) {
        self.cold_start(ctx)
    }
}

/// Entry point for the aperiodic process
///
/// This process runs in background mode
extern "C" fn aperiodic() {
    info!("Start Aperiodic");
    for i in 0..i32::MAX {
        if let SystemTime::Normal(time) = Hypervisor::get_time() {
            // round the time to an integer value of milliseconds
            let round = Duration::from_millis(time.as_millis() as u64);
            info!("{:?}: AP MSG {i}", format_duration(round).to_string());
        }
        std::thread::sleep(Duration::from_millis(1))
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CustomMessage {
    msg: String,
    when: Duration,
}

extern "C" fn periodic() {
    let ident = Hypervisor::get_partition_status().identifier;
    for i in 1..i32::MAX {
        if let SystemTime::Normal(time) = Hypervisor::get_time() {
            let round = Duration::from_millis(time.as_millis() as u64);
            info!("{:?}: P MSG {i}", format_duration(round).to_string());
        }
        std::thread::sleep(Duration::from_millis(1));

        if i % 5 == 0 {
            if ident == 0 {
                SOURCE_PORT
                    .get()
                    .unwrap()
                    .send_type(CustomMessage {
                        msg: format!("Sampling MSG {}", i / 5),
                        when: Hypervisor::get_time().unwrap_duration(),
                    })
                    .ok()
                    .unwrap();
            } else if ident == 1 {
                let (valid, data) = DESTINATION_PORT
                    .get()
                    .unwrap()
                    .recv_type::<CustomMessage>()
                    .ok()
                    .unwrap();

                info!("Received via Sampling Port: {:?}, valid: {valid:?}", data)
            }

            Hypervisor::periodic_wait().unwrap();
        }
    }
}
