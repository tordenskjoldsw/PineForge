use embassy_time::Timer;

use crate::boot::watchdog::BootloaderWatchdog;

/// Keeps the bootloader-owned watchdog alive while the executor is healthy.
#[embassy_executor::task]
pub async fn run(watchdog: BootloaderWatchdog) {
    loop {
        watchdog.pet();
        Timer::after_secs(1).await;
    }
}
