# apex-linux

This repository contains a hypervisor for the APEX API defined in the
ARINC 653 standard.
The user provides a parition scheme and a normal Linux binary for each
partition, which will then in turn be scheduled and managed by the
`apex-linux` binary.
Each parition is a regular Unix process running in its own *cgroup*
and *namespace*, in order to not interfere with the host operating
system.

Currently, this software requires a somewhat modern version of both,
the Linux kernel and the Rust toolchain, as it makes heavy use of the
`cgroups(7)` and `namespaces(7)` API for it's internal operations.

As of now (November 2022), the project is relatively new and untested,
meaning that certain things may be subject to later change.
