//! Owns the external flash for display settings and the BLE bond.
//!
//! Boot decodes both settings slots and the bond record, publishing the
//! newest valid settings (or defaults) and the stored bond. UI-submitted
//! settings persist debounced across two slots; bond writes are one-shot.
//! All regions are documented in `docs/FLASH-MAP.md`.

use defmt::{info, warn};
use embassy_futures::select::{Either3, select3};
use embassy_time::{Duration, Instant, with_deadline};
use pineforge_state::{
    BOND_RECORD_LEN, DFU_SLOT_SIZE, DfuFailReason, DisplaySettings, SETTINGS_RECORD_LEN,
    SettingsSlot, frame_bond, parse_bond, select_slot,
};

use crate::{
    board::buses::FlashSpi,
    drivers::xt25f32::{Error as FlashError, Xt25f32, is_supported_jedec_id},
    services::events::{
        BOND_LOADED, BOND_STORE, DFU_FLASH_COMMANDS, DFU_FLASH_RESULT, DISPLAY_SETTINGS,
        DfuFlashCommand, SETTINGS_COMMANDS, StoredBond,
    },
};

/// Mirrors the reserved region in `docs/FLASH-MAP.md`.
const BOND_ADDRESS: u32 = 0x003F_D000;
const SETTINGS_SLOT_A_ADDRESS: u32 = 0x003F_E000;
const SETTINGS_SLOT_B_ADDRESS: u32 = 0x003F_F000;
/// `MCUBoot` secondary slot in external flash; DFU stages the image here.
const DFU_SLOT_BASE: u32 = 0x0004_0000;

/// Collapses bursts of preset cycling into a single flash write.
const PERSIST_DEBOUNCE: Duration = Duration::from_secs(2);

const fn slot_address(slot: SettingsSlot) -> u32 {
    match slot {
        SettingsSlot::A => SETTINGS_SLOT_A_ADDRESS,
        SettingsSlot::B => SETTINGS_SLOT_B_ADDRESS,
    }
}

fn read_slot(flash: &mut Xt25f32<FlashSpi>, slot: SettingsSlot) -> Option<(DisplaySettings, u32)> {
    let mut record = [0_u8; SETTINGS_RECORD_LEN];
    flash.read(slot_address(slot), &mut record).ok()?;
    DisplaySettings::decode(&record).ok()
}

fn read_bond(flash: &mut Xt25f32<FlashSpi>) -> Option<StoredBond> {
    let mut record = [0_u8; BOND_RECORD_LEN];
    flash.read(BOND_ADDRESS, &mut record).ok()?;
    parse_bond(&record)
}

/// Maps a low-level flash error to the on-screen DFU failure reason,
/// defaulting to the operation the caller was performing.
const fn fail_reason<E>(error: &FlashError<E>, default: DfuFailReason) -> DfuFailReason {
    match error {
        FlashError::VerifyFailed => DfuFailReason::VerifyFailed,
        FlashError::Spi(_) | FlashError::BusyTimeout => default,
    }
}

/// Executes one DFU flash op, rebasing and range-checking the image-relative
/// offset so a write can only ever land inside the secondary slot.
async fn run_dfu_command(
    flash: &mut Xt25f32<FlashSpi>,
    command: DfuFlashCommand,
) -> Result<(), DfuFailReason> {
    match command {
        DfuFlashCommand::Erase(offset) => {
            if offset >= DFU_SLOT_SIZE {
                return Err(DfuFailReason::EraseFailed);
            }
            flash
                .erase_sector(DFU_SLOT_BASE + offset)
                .await
                .map_err(|e| fail_reason(&e, DfuFailReason::EraseFailed))
        }
        DfuFlashCommand::Program { offset, data } => {
            let len = u32::try_from(data.len()).unwrap_or(u32::MAX);
            if offset.saturating_add(len) > DFU_SLOT_SIZE {
                return Err(DfuFailReason::ProgramFailed);
            }
            flash
                .program(DFU_SLOT_BASE + offset, &data)
                .await
                .map_err(|e| fail_reason(&e, DfuFailReason::ProgramFailed))
        }
    }
}

#[embassy_executor::task]
#[allow(clippy::too_many_lines)]
pub async fn run(spi: FlashSpi) {
    let mut flash = Xt25f32::new(spi);
    let sender = DISPLAY_SETTINGS.sender();

    // A missing or foreign chip degrades to RAM-only settings; the firmware
    // must stay fully usable without persistence. When it is not writable we
    // remember why, so a later DFU attempt can surface the cause on-screen
    // rather than the sealed watch's only symptom being lost settings.
    let mut writable = true;
    let mut flash_fault = DfuFailReason::FlashInitFailed;
    match flash.init().await {
        Ok(id) if is_supported_jedec_id(id) => {}
        Ok(id) => {
            warn!(
                "Unexpected flash JEDEC id {=[u8]:#04x}; settings stay in RAM",
                id
            );
            writable = false;
            flash_fault = DfuFailReason::FlashUnrecognized(id);
        }
        Err(_) => {
            warn!("Flash init failed; settings stay in RAM");
            writable = false;
            flash_fault = DfuFailReason::FlashInitFailed;
        }
    }

    let decision = if writable {
        select_slot(
            read_slot(&mut flash, SettingsSlot::A),
            read_slot(&mut flash, SettingsSlot::B),
        )
    } else {
        select_slot(None, None)
    };
    let mut current = decision
        .current
        .map_or(DisplaySettings::DEFAULT, |(settings, _)| settings);
    let mut write_slot = decision.write_slot;
    let mut next_sequence = decision.next_sequence;
    info!(
        "Settings loaded: stored={} sequence={}",
        decision.current.is_some(),
        next_sequence
    );
    sender.send(current);

    // Publish the stored bond so the BLE task can install it before it starts
    // advertising; always signal, even absent, so it never blocks waiting.
    let bond = if writable {
        read_bond(&mut flash)
    } else {
        None
    };
    info!("BLE bond loaded: stored={}", bond.is_some());
    BOND_LOADED.signal(bond);

    loop {
        match select3(
            SETTINGS_COMMANDS.receive(),
            BOND_STORE.receive(),
            DFU_FLASH_COMMANDS.receive(),
        )
        .await
        {
            Either3::First(first) => {
                let mut pending = first;
                sender.send(pending);
                while let Ok(next) = with_deadline(
                    Instant::now() + PERSIST_DEBOUNCE,
                    SETTINGS_COMMANDS.receive(),
                )
                .await
                {
                    pending = next;
                    sender.send(pending);
                }

                if pending == current {
                    continue;
                }
                if writable {
                    let address = slot_address(write_slot);
                    let record = pending.encode(next_sequence);
                    let written = match flash.erase_sector(address).await {
                        Ok(()) => flash.program_verified(address, &record).await,
                        Err(error) => Err(error),
                    };
                    match written {
                        Ok(()) => {
                            info!("Settings persisted with sequence {}", next_sequence);
                            write_slot = write_slot.other();
                            next_sequence = next_sequence.wrapping_add(1);
                        }
                        Err(error) => {
                            // Keep the snapshot live but stop wearing a failing
                            // chip; remember why for a later DFU diagnosis.
                            warn!("Settings write failed; continuing without persistence");
                            writable = false;
                            flash_fault = fail_reason(&error, DfuFailReason::ProgramFailed);
                        }
                    }
                }
                current = pending;
            }
            Either3::Second(payload) => {
                if !writable {
                    continue;
                }
                let Some(record) = frame_bond(&payload) else {
                    warn!("Bond payload too large to persist");
                    continue;
                };
                let written = match flash.erase_sector(BOND_ADDRESS).await {
                    Ok(()) => flash.program_verified(BOND_ADDRESS, &record).await,
                    Err(error) => Err(error),
                };
                if written.is_ok() {
                    info!("BLE bond persisted");
                } else {
                    warn!("Bond write failed; pairing will not survive reboot");
                }
            }
            Either3::Third(command) => {
                let result = if writable {
                    run_dfu_command(&mut flash, command).await
                } else {
                    // Never writable, so no op ran; report why it can't.
                    Err(flash_fault)
                };
                DFU_FLASH_RESULT.send(result).await;
            }
        }
    }
}
