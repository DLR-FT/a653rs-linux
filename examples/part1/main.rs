use std::{time::Duration, thread::sleep};

fn main() {
  let id = std::env::args().collect::<Vec<_>>()[1].clone();

  println!("Uid Child: {}", nix::unistd::getuid());
  println!("Pid Child: {}", nix::unistd::getpid());

  loop {
      println!("Ping: {id}");
      //PartitionContext::send(15);
      //println!("{:?}", PartitionContext::recv());
      sleep(Duration::from_millis(500))
  }
}