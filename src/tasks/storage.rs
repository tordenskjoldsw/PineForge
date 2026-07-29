//! Owns the external flash: display settings, the BLE bond, and the DFU slot.
//!
//! Boot decodes both settings slots and the bond record, publishing the
//! newest valid settings (or defaults) and the stored bond. UI-submitted
//! settings persist debounced across two slots; bond writes are one-shot.
//! All regions are documented in `docs/FLASH-MAP.md`.
//!
//! Named for the peripheral it owns rather than the first thing stored on it.
//! As `services::settings` it was the one hardware-owning task outside
//! `tasks`, and the name accounted for one of the three regions it writes.

use defmt::{info, warn};
use embassy_futures::select::{Either3, select3};
use embassy_time::{Duration, Instant, with_deadline};
use pineforge_state::{
    AppEvent, BOND_RECORD_LEN, DFU_SLOT_SIZE, DfuFailReason, DisplaySettings, SETTINGS_RECORD_LEN,
    STORAGE_BASE, STORAGE_DATA_SECTOR_COUNT, STORAGE_FORMAT_VERSION, STORAGE_HEADER_LEN,
    STORAGE_PROGRESS_OFFSET, STORAGE_READY_HEADER_OFFSET, STORAGE_SECTOR_SIZE, SettingsSlot,
    StorageHeader, StorageState, bond_schema_tag, decode_storage_header, encode_storage_header,
    frame_bond, parse_bond, select_slot, storage_header_version,
};

use crate::{
    board::buses::FlashSpi,
    boot::watchdog::BootloaderWatchdog,
    drivers::xt25f32::{Error as FlashError, Xt25f32, is_supported_jedec_id},
    ipc::{
        BOND_LOADED, BOND_STORE, DFU_FLASH_COMMANDS, DFU_FLASH_RESULT, DISPLAY_SETTINGS,
        DfuFlashCommand, SETTINGS_COMMANDS, StoredBond, UI_EVENTS,
    },
};

async fn initialize_storage(
    flash: &mut Xt25f32<FlashSpi>,
    watchdog: BootloaderWatchdog,
) -> Result<(), FlashError<<FlashSpi as embedded_hal::spi::ErrorType>::Error>> {
    let mut formatting = [0_u8; STORAGE_HEADER_LEN];
    let mut ready = [0_u8; STORAGE_HEADER_LEN];
    flash.read(STORAGE_BASE, &mut formatting)?;
    flash.read(STORAGE_BASE + STORAGE_READY_HEADER_OFFSET, &mut ready)?;

    if decode_storage_header(&ready) == Some(StorageHeader::Ready) {
        info!("PineForge storage format is ready");
        return Ok(());
    }
    if [formatting, ready].iter().any(|header| {
        storage_header_version(header).is_some_and(|version| version != STORAGE_FORMAT_VERSION)
    }) {
        warn!("Unsupported PineForge storage version; refusing destructive downgrade");
        return Err(FlashError::VerifyFailed);
    }

    if decode_storage_header(&formatting) != Some(StorageHeader::Formatting) {
        flash.erase_sector(STORAGE_BASE).await?;
        watchdog.pet();
        formatting = encode_storage_header(StorageHeader::Formatting);
        flash.program_verified(STORAGE_BASE, &formatting).await?;
    }

    let mut last_percent = u8::MAX;
    for index in 0..STORAGE_DATA_SECTOR_COUNT {
        let marker_address =
            STORAGE_BASE + STORAGE_PROGRESS_OFFSET + u32::try_from(index).unwrap_or(u32::MAX);
        let mut marker = [0xff_u8; 1];
        flash.read(marker_address, &mut marker)?;
        if marker[0] != 0 {
            let sector_address =
                STORAGE_BASE + STORAGE_SECTOR_SIZE * (u32::try_from(index).unwrap_or(u32::MAX) + 1);
            flash.erase_sector(sector_address).await?;
            watchdog.pet();
            flash.program_verified(marker_address, &[0]).await?;
        }

        let percent = u8::try_from(((index + 1) * 100) / STORAGE_DATA_SECTOR_COUNT).unwrap_or(100);
        if percent != last_percent {
            let _ = UI_EVENTS.try_send(AppEvent::StorageUpdated(StorageState::Formatting(percent)));
            last_percent = percent;
        }
    }

    ready = encode_storage_header(StorageHeader::Ready);
    flash
        .program_verified(STORAGE_BASE + STORAGE_READY_HEADER_OFFSET, &ready)
        .await?;
    watchdog.pet();
    UI_EVENTS
        .send(AppEvent::StorageUpdated(StorageState::Ready))
        .await;
    info!("PineForge storage format complete");
    Ok(())
}

/// Mirrors the reserved region in `docs/FLASH-MAP.md`.
const BOND_ADDRESS: u32 = 0x003F_D000;
const SETTINGS_SLOT_A_ADDRESS: u32 = 0x003F_E000;
const SETTINGS_SLOT_B_ADDRESS: u32 = 0x003F_F000;
/// `MCUBoot` secondary slot in external flash; DFU stages the image here.
const DFU_SLOT_BASE: u32 = 0x0004_0000;

/// Collapses bursts of preset cycling into a single flash write.
const PERSIST_DEBOUNCE: Duration = Duration::from_secs(2);

/// Names the serialized bond layout this build reads and writes.
///
/// `build.rs` derives it from the locked BLE stack version, so an update that
/// changes the layout changes the tag without anyone remembering to.
const BOND_SCHEMA: &str = env!("PINEFORGE_BOND_SCHEMA");
/// The layout that records written before tagging carry implicitly.
///
/// Such a record is only trustworthy while this build still reads that same
/// layout - which stops being true by itself as soon as `BOND_SCHEMA` moves,
/// and then the record is discarded in favour of re-pairing.
const UNTAGGED_BOND_SCHEMA: &str = "0.7.0";

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
    let stored = parse_bond(&record)?;

    // The record is intact; the question is whether its bytes still mean what
    // this build would read them to mean. Installing keys decoded under the
    // wrong layout would leave the phone believing in a bond the watch cannot
    // honour, which is worse than pairing again.
    match stored.schema {
        Some(schema) if bond_schema_tag(BOND_SCHEMA) == Some(schema) => Some(stored.payload),
        None if BOND_SCHEMA == UNTAGGED_BOND_SCHEMA => Some(stored.payload),
        _ => {
            warn!("Stored bond was written for another BLE layout; re-pairing");
            None
        }
    }
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
pub async fn run(spi: FlashSpi, watchdog: BootloaderWatchdog) {
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

    // Publish the stored bond before the potentially long one-time storage
    // format below. The BLE task waits only briefly for it in restore_bond, so
    // signalling here (the bond record lives outside the formatted range) keeps
    // bond restore from racing the format and timing out on the first confirmed
    // boot. Always signal, even when absent, so the BLE task never blocks.
    let bond = if writable {
        read_bond(&mut flash)
    } else {
        None
    };
    info!("BLE bond loaded: stored={}", bond.is_some());
    BOND_LOADED.signal(bond);

    if writable && crate::boot::confirm::is_validated() {
        if initialize_storage(&mut flash, watchdog).await.is_err() {
            warn!("PineForge storage initialization failed");
            UI_EVENTS
                .send(AppEvent::StorageUpdated(StorageState::Failed))
                .await;
        }
    } else if writable {
        info!("Storage remains reserved while firmware rollback is possible");
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
                let Some(record) =
                    bond_schema_tag(BOND_SCHEMA).and_then(|schema| frame_bond(schema, &payload))
                else {
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
