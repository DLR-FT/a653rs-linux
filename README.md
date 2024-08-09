# ARINC 653 Hypervisor for Linux

This repository contains two crates which both depend on the `a653rs-linux-core` crate:
- `a653rs-linux-hypervisor` is an [ARINC 653](https://aviation-ia.sae-itc.com/standards/arinc653p0-3-653p0-3-avionics-application-software-standard-interface-part-0-overview-arinc-653)-compliant type 2 hypervisor that supports paravirtualization on a process level. It is based on the Linux OS and provides an APEX-like API, as defined in the `a653rs-linux-core` crate, to the partitions.
- `a653rs-linux` is a [`a653rs`](https://github.com/DLR-FT/a653rs) shim library used in partition development. It uses the hypervisor's APEX-like API to provide an actual APEX API.

<p align="center"><picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://github.com/user-attachments/assets/3f1e2424-681b-4418-9e12-fc63f0a46230">
    <img width="75%" alt="A figure showing the three different crates in this project and the a653rs crate and their relations." src="https://github.com/user-attachments/assets/0e57736f-86fa-4e66-a5be-36c928bc5bb8">
</picture></p>

The goal of this project is to provide a familiar environment for the functional development of ARINC 653 partitions.

The user provides a partitioning scheme and a normal Linux binary for each partition, which will then in turn be scheduled and managed by the `a653rs-linux-hypervisor` binary.
Each partition is a regular Unix process running in its own *CGroup* and *namespace*, to not interfere with the host operating system.

## Example

In this example, the partitions with the binaries `fuel_tank_simulation` and `fuel_tank_controller` exchange data using two ARINC 653 sampling channels.
The location of the binaries is discovered using the `PATH` environment variable.

```yaml
# examples/fuel_tank.yaml
major_frame: 20ms
partitions:
  - id: 0
    name: fuel_tank_simulation
    duration: 10ms
    offset: 0ms
    period: 20ms
    image: fuel_tank_simulation
  - id: 1
    name: fuel_tank_controller
    offset: 10ms
    duration: 10ms
    image: fuel_tank_controller
    period: 20ms
channel:
  - !Sampling
    msg_size: 10KB
    source:
      partition: fuel_tank_simulation
      port: fuel_sensors
    destination:
      - partition: fuel_tank_controller
        port: fuel_sensors
  - !Sampling
    msg_size: 10KB
    source:
      partition: fuel_tank_controller
      port: fuel_actuators
    destination:
      - partition: fuel_tank_simulation
        port: fuel_actuators
```

```sh
cargo build --release --target x86_64-unknown-linux-musl -p fuel_tank_simulation -p fuel_tank_controller
PATH="target/x86_64-unknown-linux-musl/release:$PATH"
RUST_LOG=trace cargo run --package a653rs-linux-hypervisor --release -- examples/fuel_tank.yaml
```

## Compatibility

The hypervisor runs as a regular POSIX process requiring only user-level privileges on most modern Linux distributions.
For this, the hypervisor requires a somewhat modern version of both the Linux kernel and the Rust toolchain, as it makes heavy use of the `cgroups(7)` and `namespaces(7)` APIs for its internal operations.
Support for precise temporal isolation of partitions is currently not implemented and provided on a best-effort basis only.

Support of ARINC 653 is still incomplete and expanded continuously.
The following traits of [a653rs](https://github.com/DLR-FT/a653rs) are currently implemented:

- `ApexProcessP4`
- `ApexPartitionP4`
- `ApexSamplingPortP4`
- `ApexTimeP4`
- `ApexErrorP4`

## Stability

As of now (February 2024), the project is relatively new and untested, meaning that certain things may be subject to change.

## Related Work

There has been a small but steady stream of work towards ARINC 653 execution environments.
This is a (non-exhaustive!) list of projects with a similar, ARINC 653 related, scope:

- [Airbus a653lib](https://github.com/airbus/a653lib)
  - Runs on Linux
  - Based on POSIX process API
  - Licensed as [LGPL-2.1-or-later](https://spdx.org/licenses/LGPL-2.1-or-later.html)
- [pok](https://pok-kernel.github.io/)
  - [GitHub repo](https://github.com/pok-kernel/pok)
  - Runs bare-metal on x86, PowerPC, Leon
  - POSIX & ARINC 653 compatible
  - Licensed as [BSD-2-Clause](https://spdx.org/licenses/BSD-2-Clause.html)
- [JetOS](https://pok-kernel.github.io/)
  - Fork of pok
  - [GitHub repo #1](https://github.com/HESL-polymtl/CHPOK)
  - [GitHub repo #2](https://github.com/yoogx/forge.ispras.ru-git-chpok)
  - Runs bare-metal on x86, PowerPC, Leon
  - POSIX & ARINC 653 compatible
  - Licensed as mix of [BSD-2-Clause](https://spdx.org/licenses/BSD-2-Clause.html), [GPL-3.0-only](https://spdx.org/licenses/GPL-3.0-only.html)
- [ARISS](https://github.com/ARISSIM/ARISS)
  - Runs on Linux
- [arinc653emulator](https://github.com/adubey14/arinc653emulator)
  - Runs on Linux
- [ARINC653_ARMV7A_Z7000](https://github.com/lfarcaro/ARINC653_ARMV7A_Z7000)
  - Runs bare-metal on Xilinx Zynq 7000
  - Licensed as [BSD-2-Clause](https://spdx.org/licenses/BSD-2-Clause.html)
