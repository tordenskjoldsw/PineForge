//! `MCUBoot` image confirmation.
//!
//! The bootloader records whether the running image has been confirmed in the
//! primary-slot trailer. `InfiniTime`'s `FirmwareValidator` uses the same word:
//! `image_ok` at `0x0007_BFE8`, set to 1 once the image is confirmed.

// `PRIMARY_SLOT_END`: where the primary image slot ends, derived from
// `memory.x` by `build.rs`. The one number here that is not this module's to
// choose - it is the far end of the region the linker script hands the image,
// which is the same edge the bootloader measures its trailer back from.
include!(concat!(env!("OUT_DIR"), "/flash_map.rs"));

/// How far back from the end of the slot `MCUBoot` keeps `image_ok`: past the
/// 16-byte magic and the write-aligned `copy_done` word behind it.
const IMAGE_OK_OFFSET_FROM_END: usize = 0x18;

/// `image_ok` word in the internal-flash primary-slot trailer.
const IMAGE_OK_ADDRESS: usize = PRIMARY_SLOT_END - IMAGE_OK_OFFSET_FROM_END;
const IMAGE_OK_VALUE: u32 = 1;

/// The address `InfiniTime`'s `FirmwareValidator` writes, and the row in
/// `docs/FLASH-MAP.md`.
///
/// Deriving the address from the linker script is what keeps it correct when
/// the layout moves; this is what refuses to let it move silently. `confirm()`
/// is the firmware's only write to internal flash, on a watch with no exposed
/// SWD pads - so a slot edge that shifts must stop the build and be re-checked
/// against the bootloader, not be trusted because the arithmetic still ran.
const _: () = assert!(
    IMAGE_OK_ADDRESS == 0x0007_BFE8,
    "the primary slot moved: re-check the MCUBoot trailer offset and docs/FLASH-MAP.md \
     before letting this write to a new address"
);

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
