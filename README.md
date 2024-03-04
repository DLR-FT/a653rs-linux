# An ARINC 653 emulator for Linux

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

# Related Works

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
