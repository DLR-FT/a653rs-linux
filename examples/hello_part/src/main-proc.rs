#[partition(linux_apex_partition::partition::ApexLinuxPartition)]
mod hello_foo {
    #[sampling_out(msg_size = "10KB")]
    struct ExampleChannel;

    #[start(pre)]
    fn pre_start() {
        ApexLogger::install_panic_hook();
        ApexLogger::install_logger(LevelFilter::Trace).unwrap();
    }

    #[start(cold)]
    fn cold_start(ctx: start::Context) {
        ctx.init_example_channel().unwrap();
        ctx.init_periodic_hello().unwrap();
        ctx.init_aperiodic_hello().unwrap();
    }

    #[start(warm)]
    fn warm_start(ctx: start::Context) {
        cold_start(ctx);
    }

    #[aperiodic(
        time_capacity = "Infinite",
        stack_size = "100KB",
        base_priority = 1,
        deadline = "Soft"
    )]
    fn aperiodic_hello(ctx: aperiodic_hello::Context) {
        let mut i = 0usize;
        loop {
            if let SystemTime::Normal(time) = ctx.get_time() {
                let round = Duration::from_millis(time.as_millis() as u64);
                info!(
                    "{:?}: Aperiodic: Hello {i}",
                    format_duration(round).to_string()
                );
            }
            sleep(Duration::from_millis(1))
        }
    }

    #[periodic(
        period = "0ms",
        time_capacity = "Infinite",
        stack_size = "100KB",
        base_priority = 1,
        deadline = "Soft"
    )]
    fn periodic_hello(ctx: periodic_hello::Context) {
        let mut i = 0usize;
        loop {
            let time = ctx.get_time().unwrap_duration();
            info!(
                "{:?}: Aperiodic: Hello {i}",
                format_duration(time).to_string()
            );
            sleep(Duration::from_millis(1));
            if i % 5 == 0 {
                ctx.example_channel
                    .unwrap()
                    .send(format!("Hello {}", i / 5).as_bytes())
                    .unwrap();

                ctx.periodic_wait().unwrap();
            }
        }
    }
}

#[partition(linux_apex_partition::partition::ApexLinuxPartition)]
mod hello_bar {
    #[sampling_in(refresh_period = "110ms")]
    #[sampling_in(msg_size = "10KB")]
    struct ExampleChannel;

    #[start(pre)]
    fn pre_start() {
        ApexLogger::install_panic_hook();
        ApexLogger::install_logger(LevelFilter::Trace).unwrap();
    }

    #[start(cold)]
    fn cold_start(ctx: start::Context) {
        ctx.init_example_channel().unwrap();
        ctx.init_periodic_hello().unwrap();
        ctx.init_aperiodic_hello().unwrap();
    }

    #[start(warm)]
    fn warm_start(ctx: start::Context) {
        cold_start(ctx);
    }

    #[aperiodic(
        time_capacity = "Infinite",
        stack_size = "100KB",
        base_priority = 1,
        deadline = "Soft"
    )]
    fn aperiodic_hello(ctx: aperiodic_hello::Context) {
        let mut i = 0usize;
        loop {
            if let SystemTime::Normal(time) = ctx.get_time() {
                let round = Duration::from_millis(time.as_millis() as u64);
                info!(
                    "{:?}: Aperiodic: Hello {i}",
                    format_duration(round).to_string()
                );
            }
            sleep(Duration::from_millis(1))
        }
    }

    #[periodic(
        period = "0ms",
        time_capacity = "Infinite",
        stack_size = "100KB",
        base_priority = 1,
        deadline = "Soft"
    )]
    fn periodic_hello(ctx: periodic_hello::Context) {
        let mut i = 0usize;
        loop {
            let time = ctx.get_time().unwrap_duration();
            info!(
                "{:?}: Aperiodic: Hello {i}",
                format_duration(time).to_string()
            );
            sleep(Duration::from_millis(1));
            if i % 5 == 0 {
                let mut buf = [0; ExampleChannel::MSG_SIZE as usize];
                let (valid, data) = ctx.example_channel.unwrap().receive(&mut buf).unwrap();

                info!(
                    "Received via Sampling Port: {:?}, len {}, valid: {valid:?}",
                    std::str::from_utf8(data),
                    data.len()
                );

                ctx.periodic_wait().unwrap();
            }
        }
    }
}
