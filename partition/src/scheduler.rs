use std::{ptr::null_mut, thread::sleep};

use memmap2::MmapOptions;
use nix::{
    libc::{exit, stack_t},
    sys::{
        signal::{
            sigaction, SaFlags, SigAction, SigHandler,
            Signal::{self, SIGCHLD},
        },
        signalfd::SigSet,
        wait::wait,
    },
};

use crate::*;

extern "C" fn stop(_: i32) {
    unsafe { exit(1) };
}

pub fn scheduler() -> ! {
    debug!("Started Scheduler");

    // TODO stop scheduling if we are back to cold/warm start

    //Register alternate stack
    let mut alt_stack = MmapOptions::new()
        .stack()
        .len(nix::libc::SIGSTKSZ)
        .map_anon()
        .unwrap();
    unsafe {
        let stack = stack_t {
            ss_sp: alt_stack.as_mut_ptr() as *mut nix::libc::c_void,
            ss_flags: 0,
            ss_size: nix::libc::SIGSTKSZ,
        };
        nix::libc::sigaltstack(&stack, null_mut());

        let stop_action = SigAction::new(
            SigHandler::Handler(stop),
            SaFlags::SA_ONSTACK,
            SigSet::empty(),
        );
        sigaction(Signal::SIGSEGV, &stop_action).unwrap();

        let child_info_action = SigAction::new(
            SigHandler::Handler(handle_sigchld),
            SaFlags::SA_ONSTACK,
            SigSet::empty(),
        );
        sigaction(SIGCHLD, &child_info_action).unwrap();
    }

    //
    //let _wait = std::thread::spawn(|| {
    //    loop{
    //        trace!("{:?}", waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WSTOPPED)))
    //    }
    //});

    let mut periodic = PERIODIC_PROCESS.read().unwrap().and_then(|p| {
        if p.activated().unwrap() {
            Some(p)
        } else {
            None
        }
    });
    let mut aperiodic = APERIODIC_PROCESS.read().unwrap().and_then(|ap| {
        if ap.activated().unwrap() {
            ap.start().unwrap();
            Some(ap)
        } else {
            None
        }
    });

    let period = *PART_PERIOD;
    let _duration = *PART_DURATION;

    if period.is_zero() {
        error!("Period may not be Zero");
    }

    let mut start = *SYSTEM_TIME;
    while start.elapsed() > period {
        start += period;
    }

    // Run aperiodic process until we reach the next period
    if let Some(ap) = aperiodic.as_mut() {
        ap.unfreeze().unwrap();
    }
    //sleep till next period frame
    sleep(period.saturating_sub(start.elapsed()));
    start += period;

    if let Some(p) = periodic.as_mut() {
        loop {
            if let Some(ap) = aperiodic.as_mut() {
                ap.freeze().unwrap();
            }
            let pidfd = p.start().unwrap();
            p.unfreeze().unwrap();

            pidfd
                .wait_exited_timeout(period.saturating_sub(start.elapsed()))
                .unwrap();

            p.freeze().unwrap();
            p.kill().unwrap();

            //let res = waitpid(Pid::from_raw(-1), None);
            //trace!("{res:?}");
            //let res = waitpid(Pid::from_raw(-1), None);
            //trace!("{res:?}");

            if let Some(ap) = aperiodic.as_mut() {
                ap.unfreeze().unwrap();
            }

            sleep(period.saturating_sub(start.elapsed()));
            start += period;
        }
    } else {
        loop {
            sleep(Duration::from_secs(10000))
        }
    }
}

extern "C" fn handle_sigchld(_: nix::libc::c_int) {
    match wait() {
        Ok(w) => trace!("Successfully waited on process: {w:?}"),
        Err(e) => error!("Error waiting on process. {e}"),
    }
}
