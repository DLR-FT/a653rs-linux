use crate::*;

pub fn scheduler() -> ! {
    // TODO stop scheduling if we are back to cold/warm start
    let periodic = PERIODIC_PROCESS.read().unwrap();
    let aperiodic = APERIODIC_PROCESS.read().unwrap();

    let period = *PART_PERIOD;
    let duration = *PART_DURATION;

    loop {}
}
