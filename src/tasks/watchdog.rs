use defmt::info;
use embassy_time::Timer;

use crate::boot::{stack, watchdog::BootloaderWatchdog};

/// Keeps the bootloader-owned watchdog alive while the executor is healthy.
///
/// It also reports the stack's high-water mark, which is a second kind of
/// health. This task already wakes on a timer and holds no state of its own, so
/// the reading costs a comparison per second rather than a task of its own.
#[embassy_executor::task]
pub async fn run(watchdog: BootloaderWatchdog) {
    // Reported only when it grows. A line every second would bury the one
    // moment that matters - the first time a path goes deeper than any before
    // it, which is what names the path worth looking at.
    let mut deepest = 0;
    loop {
        watchdog.pet();

        let used = stack::used();
        if used > deepest {
            deepest = used;
            let capacity = stack::capacity();
            info!(
                "Stack high-water mark: {} of {} bytes, {} still unused",
                used,
                capacity,
                capacity - used
            );
        }

        Timer::after_secs(1).await;
    }
}
