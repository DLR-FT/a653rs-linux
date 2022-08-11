use std::thread::sleep;

use crate::*;

pub fn scheduler() -> ! {
    debug!("Started Scheduler");

    // TODO stop scheduling if we are back to cold/warm start

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
