use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use linux_apex_core::syscall::ptrace::PartitionTrace;
use linux_apex_core::syscall::ApexSyscall;
use nix::sys::ptrace::{self, getregs, syscall};
use nix::sys::signal::{raise, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag};
use nix::unistd::{fork, ForkResult};
use tokio::runtime::Runtime;

pub fn wait_benchmark(c: &mut Criterion) {
    let child = match unsafe { fork() } {
        Ok(ForkResult::Parent { child, .. }) => child,
        Ok(ForkResult::Child) => {
            ApexSyscall::install().unwrap();
            unsafe {
                loop {
                    nix::libc::syscall(6529);
                }
            }
        }
        Err(_) => {
            panic!("Fork failed")
        }
    };

    c.bench_function("wait_and_continue", move |b| {
        b.iter(|| {
            waitpid(child, None).unwrap();
            syscall(child, None).unwrap();
        })
    });
}

pub fn waitpid_benchmark(c: &mut Criterion) {
    let child = match unsafe { fork() } {
        Ok(ForkResult::Parent { child, .. }) => child,
        Ok(ForkResult::Child) => {
            ApexSyscall::install().unwrap();
            loop {
                sleep(Duration::from_secs(1))
            }
        }
        Err(_) => {
            panic!("Fork failed")
        }
    };

    let rt = Runtime::new().unwrap();
    let _partition =
        rt.block_on(async { Arc::new(Mutex::new(PartitionTrace::new(child).await.unwrap())) });
    panic!("{:?}", waitpid(child, Some(WaitPidFlag::WNOHANG)));
    c.bench_function("unresponded_waitpid", |b| {
        b.iter(|| waitpid(child, Some(WaitPidFlag::WNOHANG)))
    });
}

criterion_group!(ptrace, wait_benchmark, waitpid_benchmark);
criterion_main!(ptrace);
