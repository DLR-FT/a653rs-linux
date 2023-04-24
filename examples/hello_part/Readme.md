cargo build -p hello_part --release --target x86_64-unknown-linux-musl

cargo run -p a653rs-linux-hypervisor --release -- examples/hello_part/hypervisor_config.yaml