use std::time::{Duration, Instant};

/// A simple object for keeping track of a timeout that starts at some instant
/// and has a fixed duration. This object also exposes some basic functionality
/// like querying the remaining time.
#[derive(Copy, Clone)]
pub(crate) struct Timeout {
    start: Instant,
    stop: Duration,
}

impl Timeout {
    pub fn new(start: Instant, stop: Duration) -> Self {
        Self { start, stop }
    }

    pub fn remaining_time(&self) -> Duration {
        self.stop.saturating_sub(self.start.elapsed())
    }

    pub fn has_time_left(&self) -> bool {
        self.remaining_time() > Duration::ZERO
    }

    pub fn total_duration(&self) -> Duration {
        self.stop
    }
}
