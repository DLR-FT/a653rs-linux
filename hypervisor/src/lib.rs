#[macro_use]
extern crate log;

pub mod hypervisor;

// TODO Wanja:
// - Make the hypervisor a binary
//   - Make example/hello_hyp work again
//       Maybe add sh script which compiles example/hello_part
//       and then runs the hypervisor with a specified config
// - Add CLI
//   - specify config
//   - Maybe outsource target cgroup to cli?
// - Parse Config (Yaml?)
