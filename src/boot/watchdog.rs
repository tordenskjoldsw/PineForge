/// Handle for the watchdog started by the `InfiniTime` `MCUBoot` bootloader.
///
/// The bootloader enables reload register 0 and starts the WDT with a timeout
/// of roughly seven seconds. Once started, the peripheral cannot be stopped or
/// reconfigured. This implementation mirrors `hal_watchdog_tickle()` directly
/// instead of depending on ownership reconstructed by a HAL.
#[derive(Clone, Copy)]
pub struct BootloaderWatchdog {
    private: (),
}

impl BootloaderWatchdog {
    const RELOAD_REGISTER_0: *mut u32 = 0x4001_0600 as *mut u32;
    const RELOAD_VALUE: u32 = 0x6E52_4635;

    #[must_use]
    pub const fn take_over() -> Self {
        Self { private: () }
    }

    /// Reloads register 0 of the already-running watchdog.
    #[allow(unsafe_code)]
    pub fn pet(&self) {
        let () = self.private;
        // SAFETY: This is the nRF52832 WDT RR[0] register. The InfiniTime
        // bootloader enables RR[0] immediately before starting the image, and
        // the nRF52 product specification defines this exact reload value.
        unsafe { core::ptr::write_volatile(Self::RELOAD_REGISTER_0, Self::RELOAD_VALUE) };
    }
}
