//! `MCUBoot` image confirmation.
//!
//! The bootloader records whether the running image has been confirmed in the
//! primary-slot trailer. `InfiniTime`'s `FirmwareValidator` uses the same word:
//! `image_ok` at `0x0007_BFE8`, set to 1 once the image is confirmed.

/// `image_ok` word in the internal-flash primary-slot trailer.
const IMAGE_OK_ADDRESS: usize = 0x0007_BFE8;
const IMAGE_OK_VALUE: u32 = 1;

/// Returns whether the running image has been confirmed. An unconfirmed image
/// must refuse DFU so it never overwrites the rollback image.
#[must_use]
#[allow(unsafe_code)]
pub fn is_validated() -> bool {
    // SAFETY: reads one fixed word of always-mapped internal flash.
    let value = unsafe { core::ptr::read_volatile(IMAGE_OK_ADDRESS as *const u32) };
    value == IMAGE_OK_VALUE
}

/// Confirms the running image by setting `image_ok`, so the bootloader keeps
/// it instead of rolling back. Idempotent, and returns whether the image is
/// confirmed afterwards.
///
/// After this, a side-button reset no longer returns to `InfiniTime`.
#[allow(unsafe_code)]
pub fn confirm() -> bool {
    if is_validated() {
        return true;
    }

    let nvmc = nrf_pac::NVMC;
    nvmc.config()
        .write(|config| config.set_wen(nrf_pac::nvmc::vals::Wen::Wen));
    while !nvmc.ready().read().ready() {}
    // The trailer word is erased (all ones), so writing 1 only clears bits and
    // needs no page erase. SAFETY: a single word write to internal flash with
    // the controller in write mode.
    unsafe {
        core::ptr::write_volatile(IMAGE_OK_ADDRESS as *mut u32, IMAGE_OK_VALUE);
    }
    while !nvmc.ready().read().ready() {}
    nvmc.config()
        .write(|config| config.set_wen(nrf_pac::nvmc::vals::Wen::Ren));
    while !nvmc.ready().read().ready() {}

    is_validated()
}
