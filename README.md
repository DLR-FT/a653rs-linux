# An ARINC 653 emulator for Linux &emsp; [![Latest Version]][crates.io] [![Docs]][docs.rs] [![Docs (macros)]][docs.rs (macros)]

[crates.io]: https://crates.io/crates/a653rs
[docs.rs]: https://docs.rs/a653rs/latest/a653rs
[docs.rs (macros)]: https://docs.rs/a653rs/latest/a653rs_macros

This repository contains a hypervisor for the APEX API defined in the
ARINC 653 standard.
The user provides a partition scheme and a normal Linux binary for each
partition, which will then in turn be scheduled and managed by the
`a653rs-linux-hypervisor` binary.
Each partition is a regular Unix process running in its own *CGroup*
and *namespace*, in order to not interfere with the host operating
system.

Currently, this software requires a somewhat modern version of both
the Linux kernel and the Rust toolchain, as it makes heavy use of the
`cgroups(7)` and `namespaces(7)` API for its internal operations.

As of now (November 2022), the project is relatively new and untested,
meaning that certain things may be subject to later change.
