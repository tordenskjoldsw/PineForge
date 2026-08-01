use defmt::info;
use embassy_time::Timer;
use pineforge_state::{AppEvent, StackUsage};

use crate::{
    boot::{stack, watchdog::BootloaderWatchdog},
    ipc::UI_EVENTS,
};

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
    let mut published = 0;
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
        // A full UI queue at boot is ordinary: every peripheral is publishing
        // its probe result. Retry until this high-water mark has made it into
        // the retained status model rather than losing the only update.
        if deepest > published
            && UI_EVENTS
                .try_send(AppEvent::StackUpdated(StackUsage {
                    used: u16::try_from(deepest).unwrap_or(u16::MAX),
                    capacity: u16::try_from(stack::capacity()).unwrap_or(u16::MAX),
                }))
                .is_ok()
        {
            published = deepest;
        }

        Timer::after_secs(1).await;
    }
}
