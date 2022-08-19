#![allow(unconditional_panic)]
use std::{ptr::null_mut, thread::sleep};

use linux_apex_core::health_event::SystemError;
use memmap2::MmapOptions;
use nix::{
    libc::{exit, stack_t},
    sys::{
        signal::{
            raise, sigaction, SaFlags, SigAction, SigHandler,
            Signal::{self, SIGCHLD, SIGFPE},
        },
        signalfd::SigSet,
        wait::wait,
    },
};

use crate::{partition::ApexLinuxPartition, *};

pub fn scheduler() -> ! {
    loop {}
    //    debug!("Started Scheduler");
    //
    //    // TODO stop scheduling if we are back to cold/warm start
    //
    //    //Register alternate stack
    //    let mut alt_stack = MmapOptions::new()
    //        .stack()
    //        .len(nix::libc::SIGSTKSZ)
    //        .map_anon()
    //        .unwrap();
    //    unsafe {
    //        let stack = stack_t {
    //            ss_sp: alt_stack.as_mut_ptr() as *mut nix::libc::c_void,
    //            ss_flags: 0,
    //            ss_size: nix::libc::SIGSTKSZ,
    //        };
    //        nix::libc::sigaltstack(&stack, null_mut());
    //
    //        let report_sigsegv_action = SigAction::new(
    //            SigHandler::Handler(handle_sigsegv),
    //            SaFlags::SA_ONSTACK,
    //            SigSet::empty(),
    //        );
    //        sigaction(Signal::SIGSEGV, &report_sigsegv_action).unwrap();
    //
    //        let reaper_action = SigAction::new(
    //            SigHandler::Handler(handle_sigchld),
    //            SaFlags::SA_ONSTACK,
    //            SigSet::empty(),
    //        );
    //        sigaction(SIGCHLD, &reaper_action).unwrap();
    //
    //        let report_sigfpe_action = SigAction::new(
    //            SigHandler::Handler(handle_sigfpe),
    //            SaFlags::SA_ONSTACK,
    //            SigSet::empty(),
    //        );
    //        sigaction(SIGFPE, &report_sigfpe_action).unwrap();
    //    }
    //
    //    let mut periodic = PERIODIC_PROCESS.read().unwrap().and_then(|p| {
    //        if p.state().unwrap() {
    //            let pidfd = p.start().unwrap();
    //            Some((p, pidfd))
    //        } else {
    //            None
    //        }
    //    });
    //    let mut aperiodic = APERIODIC_PROCESS.read().unwrap().and_then(|ap| {
    //        if ap.state().unwrap() {
    //            ap.start().unwrap();
    //            Some(ap)
    //        } else {
    //            None
    //        }
    //    });
    //
    //    let period = *PART_PERIOD;
    //    let _duration = *PART_DURATION;
    //
    //    if period.is_zero() {
    //        error!("Period may not be Zero");
    //    }
    //
    //    let mut start = *SYSTEM_TIME;
    //    while start.elapsed() > period {
    //        start += period;
    //    }
    //
    //    // Run aperiodic process until we reach the next period
    //    if let Some(ap) = aperiodic.as_mut() {
    //        ap.unfreeze().unwrap();
    //    }
    //    //sleep till next period frame
    //    sleep(period.saturating_sub(start.elapsed()));
    //    start += period;
    //
    //    if let Some((p, pidfd)) = periodic.as_mut() {
    //        loop {
    //            if let Some(ap) = aperiodic.as_mut() {
    //                ap.freeze().unwrap();
    //            }
    //            //let pidfd = p.start().unwrap();
    //            p.clear_stack();
    //            p.unfreeze().unwrap();
    //
    //            let res = pidfd.wait_exited_timeout(period.saturating_sub(start.elapsed()));
    //
    //
    //            match res {
    //                linux_apex_core::fd::PidWaitResult::Exited => {
    //
    //                },
    //                linux_apex_core::fd::PidWaitResult::Timeout => {
    //                    ApexLinuxPartition::raise_system_error(SystemError::TimeDurationExceeded);
    //                },
    //                linux_apex_core::fd::PidWaitResult::Err(e) => {
    //                    error!("{e:#?}");
    //                    ApexLinuxPartition::raise_system_error(SystemError::PartitionMainPanic);
    //                },
    //            }
    //
    //            p.freeze().unwrap();
    //            //p.kill().unwrap();
    //
    //            if period.saturating_sub(start.elapsed()) > Duration::ZERO{
    //                if let Some(ap) = aperiodic.as_mut() {
    //                    ap.unfreeze().unwrap();
    //                }
    //
    //                sleep(period.saturating_sub(start.elapsed()));
    //            }
    //
    //
    //            start += period;
    //        }
    //    } else {
    //        loop {
    //            sleep(Duration::from_secs(10000))
    //        }
    //    }
}

extern "C" fn handle_sigchld(_: i32) {
    match wait() {
        Ok(w) => trace!("Successfully waited on process: {w:?}"),
        Err(e) => error!("Error waiting on process. {e}"),
    }
}

extern "C" fn handle_sigfpe(_: i32) {
    ApexLinuxPartition::raise_system_error(SystemError::FloatingPoint);
}

extern "C" fn handle_sigsegv(_: i32) {
    ApexLinuxPartition::raise_system_error(SystemError::Segmentation);
}
