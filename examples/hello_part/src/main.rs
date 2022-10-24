#![allow(unconditional_panic, unconditional_recursion, dead_code)]

#[macro_use]
extern crate log;

use std::str::FromStr;
use std::thread::sleep;
use std::time::Duration;

use apex_rs::prelude::*;
use apex_rs_postcard::prelude::*;
use humantime::format_duration;
use linux_apex_partition::partition::ApexLogger;
use log::LevelFilter;
use once_cell::sync::{Lazy, OnceCell};
use serde::{Deserialize, Serialize};

pub type Hypervisor = linux_apex_partition::partition::ApexLinuxPartition;

static FOO: Lazy<bool> = Lazy::new(|| Hello::get_partition_status().identifier == 0);
static BAR: Lazy<bool> = Lazy::new(|| Hello::get_partition_status().identifier == 1);
static SOURCE_HELLO: OnceCell<SamplingPortSource<HELLO_SAMPLING_PORT_SIZE, Hypervisor>> =
    OnceCell::new();
static DESTINATION_HELLO: OnceCell<SamplingPortDestination<HELLO_SAMPLING_PORT_SIZE, Hypervisor>> =
    OnceCell::new();
const HELLO_SAMPLING_PORT_SIZE: u32 = 10000;

fn main() {
    ApexLogger::install_panic_hook();
    ApexLogger::install_logger(LevelFilter::Trace).unwrap();

    Hello.run()
}

struct Hello;

impl Partition<Hypervisor> for Hello {
    fn cold_start(&self, ctx: &mut StartContext<Hypervisor>) {
        if *FOO {
            let source = ctx
                .create_sampling_port_source(Name::from_str("Hello").unwrap())
                .unwrap();
            SOURCE_HELLO.set(source).unwrap();
        } else if *BAR {
            let destination = ctx
                .create_sampling_port_destination(
                    Name::from_str("Hello").unwrap(),
                    Duration::from_millis(110),
                )
                .unwrap();
            DESTINATION_HELLO.set(destination).unwrap();
        }

        ctx.create_process(ProcessAttribute {
            period: apex_rs::prelude::SystemTime::Infinite,
            time_capacity: apex_rs::prelude::SystemTime::Infinite,
            entry_point: aperiodic_hello,
            stack_size: 100000,
            base_priority: 1,
            deadline: apex_rs::prelude::Deadline::Soft,
            name: Name::from_str("aperiodic_hello").unwrap(),
        })
        .unwrap()
        .start()
        .unwrap();

        ctx.create_process(ProcessAttribute {
            period: apex_rs::prelude::SystemTime::Normal(Duration::ZERO),
            time_capacity: apex_rs::prelude::SystemTime::Infinite,
            entry_point: periodic_hello,
            stack_size: 100000,
            base_priority: 1,
            deadline: apex_rs::prelude::Deadline::Soft,
            name: Name::from_str("periodic_hello").unwrap(),
        })
        .unwrap()
        .start()
        .unwrap();
    }

    fn warm_start(&self, ctx: &mut StartContext<Hypervisor>) {
        self.cold_start(ctx)
    }
}

extern "C" fn aperiodic_hello() {
    for i in 0..i32::MAX {
        if let SystemTime::Normal(time) = Hypervisor::get_time() {
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

extern "C" fn periodic_hello() {
    //sleep(Duration::from_millis(1));
    //rec(0);

    for i in 1..i32::MAX {
        if let SystemTime::Normal(time) = Hypervisor::get_time() {
            let round = Duration::from_millis(time.as_millis() as u64);
            info!("{:?}: P MSG {i}", format_duration(round).to_string());
        }
        sleep(Duration::from_millis(1));

        //if i % 4 == 0 {
        //    rec(0);
        //}

        if i % 5 == 0 {
            if *FOO {
                SOURCE_HELLO
                    .get()
                    .unwrap()
                    // .send(format!("Hello {}", i / 5).as_bytes())
                    .send_type(CustomMessage {
                        msg: format!("Sampling MSG {}", i / 5),
                        when: Hypervisor::get_time().unwrap_duration(),
                    })
                    .ok()
                    .unwrap();
            } else if *BAR {
                // let mut buf = [0; HELLO_SAMPLING_PORT_SIZE as usize];
                let (valid, data) = DESTINATION_HELLO
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

extern "C" fn test() {
    println!("Hello");
}

fn rec(i: usize) {
    print!("\r{i}");
    rec(i + 1)
}
