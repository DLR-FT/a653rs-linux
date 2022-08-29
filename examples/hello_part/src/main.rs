#![allow(unconditional_panic, unconditional_recursion, dead_code)]

#[macro_use]
extern crate log;

use std::str::FromStr;
use std::thread::sleep;
use std::time::Duration;

use apex_hal::prelude::*;
use humantime::format_duration;
use linux_apex_partition::partition::{ApexLinuxPartition, ApexLogger};
use log::LevelFilter;
use once_cell::sync::Lazy;

static FOO: Lazy<bool> = Lazy::new(|| Hello::get_partition_status().identifier == 0);
static BAR: Lazy<bool> = Lazy::new(|| Hello::get_partition_status().identifier == 1);
const HELLO_SAMPLING_PORT_SIZE: u32 = 10000;

fn main() {
    ApexLogger::install_panic_hook();
    ApexLogger::install_logger(LevelFilter::Trace).unwrap();

    Hello.run()
}

struct Hello;

impl Partition<ApexLinuxPartition> for Hello {
    fn cold_start(&self, ctx: &mut StartContext<ApexLinuxPartition>) {
        if *FOO {
            ctx.create_sampling_port_source(
                Name::from_str("Hello").unwrap(),
                HELLO_SAMPLING_PORT_SIZE,
            )
            .unwrap();
        } else if *BAR {
            ctx.create_sampling_port_destination(
                Name::from_str("Hello").unwrap(),
                HELLO_SAMPLING_PORT_SIZE,
                Duration::from_millis(110),
            )
            .unwrap();
        }

        ctx.create_process(ProcessAttribute {
            period: apex_hal::prelude::SystemTime::Infinite,
            time_capacity: apex_hal::prelude::SystemTime::Infinite,
            entry_point: aperiodic_hello,
            stack_size: 100000,
            base_priority: 1,
            deadline: apex_hal::prelude::Deadline::Soft,
            name: Name::from_str("aperiodic_hello").unwrap(),
        })
        .unwrap()
        .start()
        .unwrap();

        ctx.create_process(ProcessAttribute {
            period: apex_hal::prelude::SystemTime::Normal(Duration::ZERO),
            time_capacity: apex_hal::prelude::SystemTime::Infinite,
            entry_point: periodic_hello,
            stack_size: 100000,
            base_priority: 1,
            deadline: apex_hal::prelude::Deadline::Soft,
            name: Name::from_str("periodic_hello").unwrap(),
        })
        .unwrap()
        .start()
        .unwrap();
    }

    fn warm_start(&self, ctx: &mut StartContext<ApexLinuxPartition>) {
        self.cold_start(ctx)
    }
}

fn aperiodic_hello() {
    for i in 0..i32::MAX {
        if let SystemTime::Normal(time) = get_time::<ApexLinuxPartition>() {
            let round = Duration::from_millis(time.as_millis() as u64);
            info!(
                "{:?}: Aperiodic: Hello {i}",
                format_duration(round).to_string()
            );
        }
        sleep(Duration::from_millis(1))
    }
}

fn periodic_hello() {
    //sleep(Duration::from_millis(1));
    //ApexLinuxPartition::raise_system_error(SystemError::Segmentation);
    //rec(0);

    for i in 1..i32::MAX {
        if let SystemTime::Normal(time) = get_time::<ApexLinuxPartition>() {
            let round = Duration::from_millis(time.as_millis() as u64);
            info!(
                "{:?}: Periodic: Hello {i}",
                format_duration(round).to_string()
            );
        }
        sleep(Duration::from_millis(1));

        //if i % 4 == 0 {
        //    rec(0);
        //}

        if i % 5 == 0 {
            if *FOO {
                SamplingPortSource::<ApexLinuxPartition>::send_unchecked(
                    1,
                    format!("Hello {}", i / 5).as_bytes(),
                )
                .unwrap()
            } else if *BAR {
                let mut buf = [0; HELLO_SAMPLING_PORT_SIZE as usize];
                let (valid, data) = unsafe {
                    SamplingPortDestination::<ApexLinuxPartition>::receive_unchecked(1, &mut buf)
                        .unwrap()
                };

                info!(
                    "Received via Sampling Port: {:?}, len {}, valid: {valid:?}",
                    std::str::from_utf8(data),
                    data.len()
                )
            }

            periodic_wait::<ApexLinuxPartition>().unwrap();
        }
    }
}

extern "C" fn test() {
    println!("Hello");
}

fn rec(i: usize) {
    print!("\r{i}");
    rec(i + 1)
}
