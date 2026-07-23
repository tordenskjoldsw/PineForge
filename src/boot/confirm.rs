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
