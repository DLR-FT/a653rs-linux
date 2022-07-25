cargo build -p hello_part --release --target x86_64-unknown-linux-musl

cargo run -p linux-apex-hypervisor --release -- examples/hello_part/hypervisor_config.yaml