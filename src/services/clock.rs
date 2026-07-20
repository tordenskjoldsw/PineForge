#![allow(dead_code)]

use embassy_time::Instant;

#[derive(Clone, Copy, Debug)]
pub struct ClockSnapshot {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub total_seconds: u64,
}

pub struct SoftwareClock {
    start_seconds: u64,
    started_at: Instant,
}

impl SoftwareClock {
    pub const fn new(hour: u8, minute: u8, second: u8, started_at: Instant) -> Self {
        Self {
            start_seconds: hour as u64 * 3600 + minute as u64 * 60 + second as u64,
            started_at,
        }
    }

    pub fn now(&self, instant: Instant) -> ClockSnapshot {
        let elapsed = instant.duration_since(self.started_at).as_secs();
        let total = self.start_seconds + elapsed;
        let day = total % 86_400;
        ClockSnapshot {
            hour: (day / 3600) as u8,
            minute: ((day / 60) % 60) as u8,
            second: (day % 60) as u8,
            total_seconds: total,
        }
    }
}
